const std = @import("std");

pub const StartupKind = enum {
    app,
    daemon,
};

pub fn isEnabled(allocator: std.mem.Allocator, kind: StartupKind) bool {
    const path = plistPath(allocator, kind) catch return false;
    defer allocator.free(path);
    std.Io.Dir.cwd().access(std.Io.Threaded.global_single_threaded.io(), path, .{}) catch return false;
    return true;
}

pub fn enable(allocator: std.mem.Allocator, kind: StartupKind, executable_path: []const u8) !void {
    const dir = try launchAgentsDir(allocator);
    defer allocator.free(dir);
    const io = std.Io.Threaded.global_single_threaded.io();
    try std.Io.Dir.cwd().createDirPath(io, dir);

    const path = try plistPath(allocator, kind);
    defer allocator.free(path);

    const contents = switch (kind) {
        .app => try plist(allocator, label(kind), &.{executable_path}),
        .daemon => if (std.mem.eql(u8, executable_path, "folk-around"))
            try plist(allocator, label(kind), &.{ "/usr/bin/env", "folk-around", "--p2p" })
        else
            try plist(allocator, label(kind), &.{ executable_path, "--p2p" }),
    };
    defer allocator.free(contents);

    var file = try std.Io.Dir.cwd().createFile(io, path, .{ .truncate = true });
    defer file.close(io);
    try file.writeStreamingAll(io, contents);
}

pub fn disable(allocator: std.mem.Allocator, kind: StartupKind) !void {
    const path = try plistPath(allocator, kind);
    defer allocator.free(path);
    std.Io.Dir.cwd().deleteFile(std.Io.Threaded.global_single_threaded.io(), path) catch |err| switch (err) {
        error.FileNotFound => {},
        else => return err,
    };
}

pub fn daemonExecutable(allocator: std.mem.Allocator) ![]u8 {
    if (accessOk("/usr/local/bin/folk-around")) return allocator.dupe(u8, "/usr/local/bin/folk-around");
    if (accessOk("/opt/homebrew/bin/folk-around")) return allocator.dupe(u8, "/opt/homebrew/bin/folk-around");
    return allocator.dupe(u8, "folk-around");
}

fn label(kind: StartupKind) []const u8 {
    return switch (kind) {
        .app => "dev.undivisible.folk-around.app",
        .daemon => "dev.undivisible.folk-around.daemon",
    };
}

fn plistPath(allocator: std.mem.Allocator, kind: StartupKind) ![]u8 {
    const dir = try launchAgentsDir(allocator);
    defer allocator.free(dir);
    const file_name = try std.fmt.allocPrint(allocator, "{s}.plist", .{label(kind)});
    defer allocator.free(file_name);
    return std.fs.path.join(allocator, &.{ dir, file_name });
}

fn launchAgentsDir(allocator: std.mem.Allocator) ![]u8 {
    const home = std.mem.span(std.c.getenv("HOME") orelse return error.HomeMissing);
    return std.fs.path.join(allocator, &.{ home, "Library", "LaunchAgents" });
}

fn plist(allocator: std.mem.Allocator, plist_label: []const u8, args: []const []const u8) ![]u8 {
    var list: std.ArrayList(u8) = .empty;
    errdefer list.deinit(allocator);

    try list.appendSlice(allocator,
        \\<?xml version="1.0" encoding="UTF-8"?>
        \\<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
        \\<plist version="1.0">
        \\<dict>
        \\  <key>Label</key>
        \\
    );
    try appendString(&list, allocator, plist_label);
    try list.appendSlice(allocator,
        \\  <key>ProgramArguments</key>
        \\  <array>
        \\
    );
    for (args) |arg| try appendArrayString(&list, allocator, arg);
    try list.appendSlice(allocator,
        \\  </array>
        \\  <key>RunAtLoad</key>
        \\  <true/>
        \\  <key>KeepAlive</key>
        \\  <false/>
        \\</dict>
        \\</plist>
        \\
    );

    return list.toOwnedSlice(allocator);
}

fn appendString(list: *std.ArrayList(u8), allocator: std.mem.Allocator, value: []const u8) !void {
    try list.appendSlice(allocator, "  <string>");
    try appendEscaped(list, allocator, value);
    try list.appendSlice(allocator, "</string>\n");
}

fn appendArrayString(list: *std.ArrayList(u8), allocator: std.mem.Allocator, value: []const u8) !void {
    try list.appendSlice(allocator, "    <string>");
    try appendEscaped(list, allocator, value);
    try list.appendSlice(allocator, "</string>\n");
}

fn appendEscaped(list: *std.ArrayList(u8), allocator: std.mem.Allocator, value: []const u8) !void {
    for (value) |byte| {
        switch (byte) {
            '&' => try list.appendSlice(allocator, "&amp;"),
            '<' => try list.appendSlice(allocator, "&lt;"),
            '>' => try list.appendSlice(allocator, "&gt;"),
            '"' => try list.appendSlice(allocator, "&quot;"),
            '\'' => try list.appendSlice(allocator, "&apos;"),
            else => try list.append(allocator, byte),
        }
    }
}

fn accessOk(path: []const u8) bool {
    std.Io.Dir.cwd().access(std.Io.Threaded.global_single_threaded.io(), path, .{}) catch return false;
    return true;
}
