/// folk-around macOS menu bar app
/// Pure Zig via AppKit/ObjC, no Swift or Xcode needed.
/// Links against Cocoa, creates NSStatusBar item with dropdown menu.
/// Manages folk-around daemon as a child process.

// Build: zig build-exe src/mac_app.zig -framework Cocoa -framework ApplicationServices
// Or: zig build -Dapp (build.zig target)

const std = @import("std");
const startup = @import("startup.zig");

const c = @cImport({
    @cInclude("locale.h");
    @cInclude("objc/message.h");
    @cInclude("objc/runtime.h");
});

const Id = c.id;
const Sel = c.SEL;
const allocator = std.heap.smp_allocator;

var daemon_child: ?std.process.Child = null;
var status_menu_item: Id = null;
var daemon_menu_item: Id = null;
var app_login_menu_item: Id = null;
var daemon_login_menu_item: Id = null;
var app_executable_path: ?[]const u8 = null;
var objc: Objc = undefined;
var strings: Strings = undefined;

extern "c" fn objc_msgSend(self: Id, op: Sel, ...) callconv(.c) Id;

const Objc = struct {
    ns_application: Id,
    ns_status_bar: Id,
    ns_menu: Id,
    ns_menu_item: Id,
    ns_object: c.Class,
    ns_string: Id,
    alloc: Sel,
    init_sel: Sel,
    shared_application: Sel,
    set_activation_policy: Sel,
    system_status_bar: Sel,
    status_item_with_length: Sel,
    button: Sel,
    set_title: Sel,
    add_item_with_title_action_key_equivalent: Sel,
    set_enabled: Sel,
    separator_item: Sel,
    add_item: Sel,
    set_target: Sel,
    set_menu: Sel,
    run: Sel,
    terminate: Sel,
    set_state: Sel,
    string_with_utf8_string: Sel,
    toggle_daemon: Sel,
    toggle_app_login: Sel,
    toggle_daemon_login: Sel,

    fn init() Objc {
        return .{
            .ns_application = cls("NSApplication"),
            .ns_status_bar = cls("NSStatusBar"),
            .ns_menu = cls("NSMenu"),
            .ns_menu_item = cls("NSMenuItem"),
            .ns_object = @ptrCast(cls("NSObject")),
            .ns_string = cls("NSString"),
            .alloc = sel("alloc"),
            .init_sel = sel("init"),
            .shared_application = sel("sharedApplication"),
            .set_activation_policy = sel("setActivationPolicy:"),
            .system_status_bar = sel("systemStatusBar"),
            .status_item_with_length = sel("statusItemWithLength:"),
            .button = sel("button"),
            .set_title = sel("setTitle:"),
            .add_item_with_title_action_key_equivalent = sel("addItemWithTitle:action:keyEquivalent:"),
            .set_enabled = sel("setEnabled:"),
            .separator_item = sel("separatorItem"),
            .add_item = sel("addItem:"),
            .set_target = sel("setTarget:"),
            .set_menu = sel("setMenu:"),
            .run = sel("run"),
            .terminate = sel("terminate:"),
            .set_state = sel("setState:"),
            .string_with_utf8_string = sel("stringWithUTF8String:"),
            .toggle_daemon = sel("toggleDaemon:"),
            .toggle_app_login = sel("toggleAppLogin:"),
            .toggle_daemon_login = sel("toggleDaemonLogin:"),
        };
    }
};

const Strings = struct {
    app_name: Id,
    empty: Id,
    status_initial: Id,
    status_stopped: Id,
    status_running: Id,
    start_daemon: Id,
    stop_daemon: Id,
    run_app_at_login: Id,
    run_daemon_at_login: Id,
    quit: Id,
    quit_key: Id,

    fn init() Strings {
        return .{
            .app_name = nsstr("folk-around"),
            .empty = nsstr(""),
            .status_initial = nsstr("folk-around running"),
            .status_stopped = nsstr("folk-around stopped"),
            .status_running = nsstr("folk-around daemon running"),
            .start_daemon = nsstr("Start Daemon"),
            .stop_daemon = nsstr("Stop Daemon"),
            .run_app_at_login = nsstr("Run App at Login"),
            .run_daemon_at_login = nsstr("Run Daemon at Login"),
            .quit = nsstr("Quit"),
            .quit_key = nsstr("q"),
        };
    }
};

fn cls(name: [*:0]const u8) Id {
    return @ptrCast(@alignCast(c.objc_getClass(name)));
}

fn sel(name: [*:0]const u8) Sel {
    return c.sel_registerName(name);
}

fn msg(self: Id, selector: Sel) Id {
    return objc_msgSend(self, selector);
}

fn msg1(self: Id, selector: Sel, arg: anytype) Id {
    return objc_msgSend(self, selector, arg);
}

fn nsstr(text: [*:0]const u8) Id {
    return msg1(objc.ns_string, objc.string_with_utf8_string, text);
}

pub fn main(init: std.process.Init.Minimal) !void {
    _ = c.setlocale(c.LC_ALL, "en_US.UTF-8");
    objc = Objc.init();
    strings = Strings.init();
    app_executable_path = try resolveExecutablePath(std.mem.span(init.args.vector[0]));

    const target = registerMenuTarget();
    const app = msg(objc.ns_application, objc.shared_application);
    _ = msg1(app, objc.set_activation_policy, @as(isize, 1));

    const status_bar = msg(objc.ns_status_bar, objc.system_status_bar);
    const status_item = msg1(status_bar, objc.status_item_with_length, @as(f64, -1.0));
    const button = msg(status_item, objc.button);
    _ = msg1(button, objc.set_title, strings.app_name);

    const menu = msg(msg(objc.ns_menu, objc.alloc), objc.init_sel);
    status_menu_item = objc_msgSend(menu, objc.add_item_with_title_action_key_equivalent, strings.status_initial, @as(Sel, null), strings.empty);
    _ = msg1(status_menu_item, objc.set_enabled, @as(bool, false));
    _ = msg1(menu, objc.add_item, msg(objc.ns_menu_item, objc.separator_item));

    daemon_menu_item = objc_msgSend(menu, objc.add_item_with_title_action_key_equivalent, strings.start_daemon, objc.toggle_daemon, strings.empty);
    _ = msg1(daemon_menu_item, objc.set_target, target);

    app_login_menu_item = objc_msgSend(menu, objc.add_item_with_title_action_key_equivalent, strings.run_app_at_login, objc.toggle_app_login, strings.empty);
    _ = msg1(app_login_menu_item, objc.set_target, target);

    daemon_login_menu_item = objc_msgSend(menu, objc.add_item_with_title_action_key_equivalent, strings.run_daemon_at_login, objc.toggle_daemon_login, strings.empty);
    _ = msg1(daemon_login_menu_item, objc.set_target, target);

    _ = msg1(menu, objc.add_item, msg(objc.ns_menu_item, objc.separator_item));

    const quit_item = objc_msgSend(menu, objc.add_item_with_title_action_key_equivalent, strings.quit, objc.terminate, strings.quit_key);
    _ = msg1(quit_item, objc.set_target, app);
    _ = msg1(status_item, objc.set_menu, menu);
    updateMenu();

    _ = msg(app, objc.run);
}

fn registerMenuTarget() Id {
    const target_class = c.objc_allocateClassPair(objc.ns_object, "FolkAroundMenuTarget", 0) orelse @as(c.Class, @ptrCast(c.objc_getClass("FolkAroundMenuTarget")));
    _ = c.class_addMethod(target_class, objc.toggle_daemon, @ptrCast(&toggleDaemon), "v@:@");
    _ = c.class_addMethod(target_class, objc.toggle_app_login, @ptrCast(&toggleAppLogin), "v@:@");
    _ = c.class_addMethod(target_class, objc.toggle_daemon_login, @ptrCast(&toggleDaemonLogin), "v@:@");
    c.objc_registerClassPair(target_class);
    return msg(msg(@as(Id, @ptrCast(@alignCast(target_class))), objc.alloc), objc.init_sel);
}

fn toggleDaemon(_: Id, _: Sel, _: Id) callconv(.c) void {
    if (daemon_child == null) {
        startDaemon() catch {};
    } else {
        stopDaemon();
    }
    updateMenu();
}

fn toggleAppLogin(_: Id, _: Sel, _: Id) callconv(.c) void {
    if (startup.isEnabled(allocator, .app)) {
        startup.disable(allocator, .app) catch {};
    } else {
        const path = app_executable_path orelse return;
        startup.enable(allocator, .app, path) catch {};
    }
    updateMenu();
}

fn toggleDaemonLogin(_: Id, _: Sel, _: Id) callconv(.c) void {
    if (startup.isEnabled(allocator, .daemon)) {
        startup.disable(allocator, .daemon) catch {};
    } else {
        const path = startup.daemonExecutable(allocator) catch return;
        defer allocator.free(path);
        startup.enable(allocator, .daemon, path) catch {};
    }
    updateMenu();
}

fn startDaemon() !void {
    const io = std.Io.Threaded.global_single_threaded.io();
    const path = try startup.daemonExecutable(allocator);
    defer allocator.free(path);
    const argv = [_][]const u8{ path, "--p2p" };
    daemon_child = try std.process.spawn(io, .{
        .argv = &argv,
        .stdin = .ignore,
        .stdout = .ignore,
        .stderr = .ignore,
    });
}

fn stopDaemon() void {
    if (daemon_child) |*child| {
        child.kill(std.Io.Threaded.global_single_threaded.io());
        daemon_child = null;
    }
}

fn updateMenu() void {
    reapExitedDaemon();
    if (daemon_child == null) {
        _ = msg1(status_menu_item, objc.set_title, strings.status_stopped);
        _ = msg1(daemon_menu_item, objc.set_title, strings.start_daemon);
    } else {
        _ = msg1(status_menu_item, objc.set_title, strings.status_running);
        _ = msg1(daemon_menu_item, objc.set_title, strings.stop_daemon);
    }
    _ = msg1(app_login_menu_item, objc.set_state, @as(isize, if (startup.isEnabled(allocator, .app)) 1 else 0));
    _ = msg1(daemon_login_menu_item, objc.set_state, @as(isize, if (startup.isEnabled(allocator, .daemon)) 1 else 0));
}

fn reapExitedDaemon() void {
    if (daemon_child) |*child| {
        const pid = child.id orelse {
            daemon_child = null;
            return;
        };
        var status: c_int = 0;
        const result = std.c.waitpid(pid, &status, std.c.W.NOHANG);
        if (result == pid) {
            child.id = null;
            daemon_child = null;
        }
    }
}

fn resolveExecutablePath(arg0: []const u8) ![]u8 {
    const io = std.Io.Threaded.global_single_threaded.io();
    if (std.fs.path.isAbsolute(arg0)) {
        const real = std.Io.Dir.realPathFileAbsoluteAlloc(io, arg0, allocator) catch return allocator.dupe(u8, arg0);
        return real;
    }
    const real = std.Io.Dir.cwd().realPathFileAlloc(io, arg0, allocator) catch return allocator.dupe(u8, arg0);
    return real;
}
