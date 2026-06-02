/// folk-around macOS menu bar app
/// Pure Zig via AppKit/ObjC, no Swift or Xcode needed.
/// Links against Cocoa, creates NSStatusBar item with dropdown menu.
/// Manages folk-around daemon as a child process.

// Build: zig build-exe src/mac_app.zig -framework Cocoa -framework ApplicationServices
// Or: zig build -Dapp (build.zig target)

const std = @import("std");

const c = @cImport({
    @cInclude("locale.h");
    @cInclude("objc/message.h");
    @cInclude("objc/runtime.h");
});

const Id = c.id;
const Sel = c.SEL;

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

pub fn main() !void {
    _ = c.setlocale(c.LC_ALL, "en_US.UTF-8");

    const app = msg(cls("NSApplication"), "sharedApplication");
    _ = msg1(app, "setActivationPolicy:", @as(isize, 1));

    const status_bar = msg(cls("NSStatusBar"), "systemStatusBar");
    const status_item = msg1(status_bar, "statusItemWithLength:", @as(f64, -1.0));
    const button = msg(status_item, "button");
    _ = msg1(button, "setTitle:", nsstr("folk-around"));

    const menu = msg(msg(cls("NSMenu"), "alloc"), "init");
    const status_title = nsstr("folk-around running");
    const empty = nsstr("");
    const status_menu_item = objc_msgSend(menu, sel("addItemWithTitle:action:keyEquivalent:"), status_title, @as(Sel, null), empty);
    _ = msg1(status_menu_item, "setEnabled:", @as(bool, false));
    _ = msg1(menu, "addItem:", msg(cls("NSMenuItem"), "separatorItem"));

    const quit_item = objc_msgSend(menu, sel("addItemWithTitle:action:keyEquivalent:"), nsstr("Quit"), sel("terminate:"), nsstr("q"));
    _ = msg1(quit_item, "setTarget:", app);
    _ = msg1(status_item, "setMenu:", menu);

    _ = msg(app, "run");
}
