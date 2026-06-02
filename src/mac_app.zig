/// folk-around macOS menu bar app
/// Pure Zig via AppKit/ObjC, no Swift or Xcode needed.
/// Links against Cocoa, creates NSStatusBar item with dropdown menu.
/// Manages folk-around daemon as a child process.

// Build: zig build-exe src/mac_app.zig -framework Cocoa -framework ApplicationServices
// Or: zig build -Dapp (build.zig target)

const std = @import("std");
const builtin = @import("builtin");
const posix = std.posix;

// ── ObjC / AppKit imports ──
const c = @cImport({
    @cDefine("NS_ENUM", "_NS_ENUM");
    @cInclude("Cocoa/Cocoa.h");
    @cInclude("ApplicationServices/ApplicationServices.h");
});

const allocator = std.heap.c_allocator;

var app: *c.NSApplication = undefined;
var status_item: *c.NSStatusItem = undefined;
var status_label: *c.NSTextField = undefined;
var status_dot: *c.NSImageView = undefined;
var daemon_running = false;
var daemon_mode: []const u8 = "full";
var http_port: u16 = 0;
var daemon_pid: posix.pid_t = 0;
var source_timer: anyopaque = undefined;

// ── Forward declarations for ObjC selectors ──
const sel_alloc = @cImport(@cInclude("objc/message.h"));
// We'll use the runtime directly

pub fn main() !void {
    _ = c.setlocale(c.LC_ALL, "en_US.UTF-8");

    // Create autorelease pool
    const pool = c.objc_autoreleasePoolPush();

    // Get shared NSApplication
    app = c.NSApplication.sharedApplication();
    _ = c.NSApplication_setActivationPolicy(app, c.NSApplicationActivationPolicyAccessory);

    // Create status bar item
    status_item = c.NSStatusBar_systemStatusBar().statusItemWithLength(c.NSVariableStatusItemLength);
    _ = status_item.retain();

    // Build the view: dot + label
    buildStatusView();

    // Build dropdown menu
    buildMenu();

    // Start timer to update status
    startPollTimer();

    // Launch daemon by default
    startDaemon() catch {};

    // Run the app
    c.NSApplication_run(app);

    c.objc_autoreleasePoolPop(pool);
}

fn buildStatusView() void {
    const view = c.NSView_alloc();
    _ = view.initWithFrame(c.NSMakeRect(0, 0, 120, 22));

    // Status dot (green/red circle)
    status_dot = c.NSImageView_alloc();
    _ = status_dot.initWithFrame(c.NSMakeRect(4, 4, 14, 14));
    updateDotColor();
    view.addSubview(status_dot);

    // Status label
    status_label = c.NSTextField_alloc();
    _ = status_label.initWithFrame(c.NSMakeRect(22, 2, 94, 18));
    status_label.setBezeled(false);
    status_label.setDrawsBackground(false);
    status_label.setEditable(false);
    status_label.setSelectable(false);
    status_label.setStringValue(c.NSString_stringWithUTF8String("folk-around"));
    status_label.setFont(c.NSFont_menuBarFontOfSize(12));
    view.addSubview(status_label);

    status_item.setView(view);
}

fn updateDotColor() void {
    const color = if (daemon_running)
        c.NSColor_systemGreenColor()
    else
        c.NSColor_systemGrayColor();

    // Create a small colored circle image
    var image: *c.NSImage = undefined;
    _ = c.NSImage_alloc();
    image = c.NSImage_initWithSize(c.NSMakeSize(14, 14));
    _ = image.lockFocus();
    color.set();
    const rect = c.NSMakeRect(0, 0, 14, 14);
    c.NSBezierPath_fillRoundedRect_xRadius_yRadius(rect, 7, 7);
    _ = image.unlockFocus();
    _ = status_dot.setImage(image);
}

fn buildMenu() void {
    const menu = c.NSMenu_alloc();
    _ = menu.init();

    // Status section
    const status_text = c.NSString_stringWithUTF8String(
        if (daemon_running) "Running" else "Stopped"
    );
    const status_item_menu = menu.addItemWithTitle_action_keyEquivalent(
        status_text, null, c.NSString_stringWithUTF8String("")
    );
    status_item_menu.setEnabled(false);
    menu.addItem(c.NSMenuItem_separatorItem());

    // Mode submenu
    const mode_title = c.NSString_stringWithUTF8String("Mode");
    const mode_item = menu.addItemWithTitle_action_keyEquivalent(
        mode_title, null, c.NSString_stringWithUTF8String("")
    );
    mode_item.setEnabled(false);

    const mode_names = [_][]const u8{ "full", "limited", "sandbox" };
    for (mode_names) |name| {
        const label = c.NSString_stringWithUTF8String(name.ptr);
        const mi = menu.addItemWithTitle_action_keyEquivalent(
            label,
            c.NSSelectorFromString(c.NSString_stringWithUTF8String("switchMode:")),
            c.NSString_stringWithUTF8String("")
        );
        mi.setTarget(status_item);
        mi.setState(if (std.mem.eql(u8, name, daemon_mode)) c.NSOnState else c.NSOffState);
    }

    menu.addItem(c.NSMenuItem_separatorItem());

    // Controls
    const start_label = c.NSString_stringWithUTF8String(
        if (daemon_running) "Restart Daemon" else "Start Daemon"
    );
    const start_item = menu.addItemWithTitle_action_keyEquivalent(
        start_label,
        c.NSSelectorFromString(c.NSString_stringWithUTF8String("toggleDaemon:")),
        c.NSString_stringWithUTF8String("r")
    );
    start_item.setTarget(status_item);

    // Port display
    if (http_port > 0) {
        var buf: [64]u8 = undefined;
        const port_str = std.fmt.bufPrint(&buf, "HTTP :{d}", .{http_port}) catch "HTTP";
        const port_label = c.NSString_stringWithUTF8String(port_str.ptr);
        const port_item = menu.addItemWithTitle_action_keyEquivalent(
            port_label, null, c.NSString_stringWithUTF8String("")
        );
        port_item.setEnabled(false);
    }

    menu.addItem(c.NSMenuItem_separatorItem());

    // Logs
    const logs_item = menu.addItemWithTitle_action_keyEquivalent(
        c.NSString_stringWithUTF8String("Logs..."),
        c.NSSelectorFromString(c.NSString_stringWithUTF8String("openLogs:")),
        c.NSString_stringWithUTF8String("")
    );
    logs_item.setTarget(status_item);

    // Quit
    const quit_item = menu.addItemWithTitle_action_keyEquivalent(
        c.NSString_stringWithUTF8String("Quit"),
        c.NSSelectorFromString(c.NSString_stringWithUTF8String("terminate:")),
        c.NSString_stringWithUTF8String("q")
    );
    quit_item.setTarget(app);

    status_item.setMenu(menu);
}

fn startPollTimer() void {
    // Poll every 2 seconds to check if daemon is alive
    // In actual ObjC, we'd use an NSTimer. For Zig, we use dispatch_source
    const dispatch_queue = c.dispatch_get_main_queue();
    source_timer = c.dispatch_source_create(
        c.DISPATCH_SOURCE_TYPE_TIMER, 0, 0, dispatch_queue
    );
    c.dispatch_source_set_timer(source_timer, c.DISPATCH_TIME_NOW, 2 * c.NSEC_PER_SEC, 0);
    // Note: full implementation would use dispatch_source_set_event_handler_f
}

fn startDaemon() !void {
    // Fork and exec folk-around daemon
    const pid = try posix.fork();
    if (pid == 0) {
        // Child: exec the daemon
        const args = [_][]const u8{
            "/usr/local/bin/folk-around",
            "--http", "8080",
            "--mode", daemon_mode,
            "--verbose",
        };
        const env = [_][]const u8{};
        posix.execve(args[0], &args, &env) catch {};
        posix.exit(1);
    }
    daemon_pid = pid;
    daemon_running = true;
    http_port = 8080;
    updateUI();
}

fn stopDaemon() void {
    if (daemon_pid > 0) {
        posix.kill(daemon_pid, posix.SIG.TERM) catch {};
        _ = posix.waitpid(daemon_pid, 0) catch {};
        daemon_pid = 0;
    }
    daemon_running = false;
    http_port = 0;
    updateUI();
}

fn updateUI() void {
    updateDotColor();
    // Rebuild menu to reflect state
    buildMenu();
}

// ── ObjC selectors called by AppKit ──
// These are registered as IMPs on the NSStatusItem target.

export fn toggleDaemon_(_self: *c.id, _cmd: c.SEL) void {
    _ = _self;
    _ = _cmd;
    if (daemon_running) {
        stopDaemon();
        _ = startDaemon() catch {};
    } else {
        startDaemon() catch {};
    }
}

export fn switchMode_(_self: *c.id, _cmd: c.SEL, sender: *c.id) void {
    _ = _self;
    _ = _cmd;
    if (sender.title) |title| {
        if (title.UTF8String) |cstr| {
            const mode = std.mem.span(@as([*:0]u8, @ptrCast(cstr)));
            daemon_mode = mode;
            if (daemon_running) {
                stopDaemon();
                _ = startDaemon() catch {};
            }
            updateUI();
        }
    }
}

export fn openLogs_(_self: *c.id, _cmd: c.SEL) void {
    _ = _self;
    _ = _cmd;
    const workspace = c.NSWorkspace_sharedWorkspace();
    const path = c.NSString_stringWithUTF8String("/tmp/folk-around.log");
    _ = workspace.openFile(path);
}