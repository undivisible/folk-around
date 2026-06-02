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

extern "c" fn objc_msgSend(self: Id, op: Sel, ...) callconv(.c) Id;

fn cls(name: [*:0]const u8) Id {
    return @ptrCast(@alignCast(c.objc_getClass(name)));
}

fn sel(name: [*:0]const u8) Sel {
    return c.sel_registerName(name);
}

fn msg(self: Id, name: [*:0]const u8) Id {
    return objc_msgSend(self, sel(name));
}

fn msg1(self: Id, name: [*:0]const u8, arg: anytype) Id {
    return objc_msgSend(self, sel(name), arg);
}

fn nsstr(text: [*:0]const u8) Id {
    return msg1(cls("NSString"), "stringWithUTF8String:", text);
}

pub fn main(init: std.process.Init.Minimal) !void {
    _ = c.setlocale(c.LC_ALL, "en_US.UTF-8");
    app_executable_path = try resolveExecutablePath(std.mem.span(init.args.vector[0]));

    const target = registerMenuTarget();
    const app = msg(cls("NSApplication"), "sharedApplication");
    _ = msg1(app, "setActivationPolicy:", @as(isize, 1));

    const status_bar = msg(cls("NSStatusBar"), "systemStatusBar");
    const status_item = msg1(status_bar, "statusItemWithLength:", @as(f64, -1.0));
    const button = msg(status_item, "button");
    _ = msg1(button, "setTitle:", nsstr("folk-around"));

    const menu = msg(msg(cls("NSMenu"), "alloc"), "init");
    const status_title = nsstr("folk-around running");
    const empty = nsstr("");
    status_menu_item = objc_msgSend(menu, sel("addItemWithTitle:action:keyEquivalent:"), status_title, @as(Sel, null), empty);
    _ = msg1(status_menu_item, "setEnabled:", @as(bool, false));
    _ = msg1(menu, "addItem:", msg(cls("NSMenuItem"), "separatorItem"));

    daemon_menu_item = objc_msgSend(menu, sel("addItemWithTitle:action:keyEquivalent:"), nsstr("Start Daemon"), sel("toggleDaemon:"), empty);
    _ = msg1(daemon_menu_item, "setTarget:", target);

    app_login_menu_item = objc_msgSend(menu, sel("addItemWithTitle:action:keyEquivalent:"), nsstr("Run App at Login"), sel("toggleAppLogin:"), empty);
    _ = msg1(app_login_menu_item, "setTarget:", target);

    daemon_login_menu_item = objc_msgSend(menu, sel("addItemWithTitle:action:keyEquivalent:"), nsstr("Run Daemon at Login"), sel("toggleDaemonLogin:"), empty);
    _ = msg1(daemon_login_menu_item, "setTarget:", target);

    _ = msg1(menu, "addItem:", msg(cls("NSMenuItem"), "separatorItem"));

    const quit_item = objc_msgSend(menu, sel("addItemWithTitle:action:keyEquivalent:"), nsstr("Quit"), sel("terminate:"), nsstr("q"));
    _ = msg1(quit_item, "setTarget:", app);
    _ = msg1(status_item, "setMenu:", menu);
    updateMenu();

    _ = msg(app, "run");
}

fn registerMenuTarget() Id {
    const superclass: c.Class = @ptrCast(cls("NSObject"));
    const target_class = c.objc_allocateClassPair(superclass, "FolkAroundMenuTarget", 0) orelse @as(c.Class, @ptrCast(c.objc_getClass("FolkAroundMenuTarget")));
    _ = c.class_addMethod(target_class, sel("toggleDaemon:"), @ptrCast(&toggleDaemon), "v@:@");
    _ = c.class_addMethod(target_class, sel("toggleAppLogin:"), @ptrCast(&toggleAppLogin), "v@:@");
    _ = c.class_addMethod(target_class, sel("toggleDaemonLogin:"), @ptrCast(&toggleDaemonLogin), "v@:@");
    c.objc_registerClassPair(target_class);
    return msg(msg(@as(Id, @ptrCast(@alignCast(target_class))), "alloc"), "init");
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
    if (daemon_child == null) {
        _ = msg1(status_menu_item, "setTitle:", nsstr("folk-around stopped"));
        _ = msg1(daemon_menu_item, "setTitle:", nsstr("Start Daemon"));
    } else {
        _ = msg1(status_menu_item, "setTitle:", nsstr("folk-around daemon running"));
        _ = msg1(daemon_menu_item, "setTitle:", nsstr("Stop Daemon"));
    }
    _ = msg1(app_login_menu_item, "setState:", @as(isize, if (startup.isEnabled(allocator, .app)) 1 else 0));
    _ = msg1(daemon_login_menu_item, "setState:", @as(isize, if (startup.isEnabled(allocator, .daemon)) 1 else 0));
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
