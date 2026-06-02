const std = @import("std");
const builtin = @import("builtin");

const Allocator = std.mem.Allocator;

pub const Result = struct {
    stdout: []u8,
    stderr: []u8,
    exit_code: i32,
};

pub fn exec(allocator: Allocator, command: []const u8, cwd: ?[]const u8) !Result {
    var child = std.process.Child.init(&[_][]const u8{ "/bin/sh", "-c", command }, allocator);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    if (cwd) |dir| child.cwd = dir;

    try child.spawn();

    // Read BEFORE waiting to avoid pipe deadlock issues
    const stdout = child.stdout orelse return Result{ .stdout = "", .stderr = "", .exit_code = -1 };
    const stderr = child.stderr orelse return Result{ .stdout = "", .stderr = "", .exit_code = -1 };
    const out = try stdout.reader().readAllAlloc(allocator, 1024 * 1024);
    const err = try stderr.reader().readAllAlloc(allocator, 1024 * 1024);

    const term = try child.wait();

    const code = switch (term) {
        .Exited => |c| @as(i32, @intCast(c)),
        .Signal => |s| -@as(i32, @intCast(s)),
        .Stopped => |s| -@as(i32, @intCast(s)),
        .Unknown => |c| @as(i32, @intCast(c)),
    };

    return Result{ .stdout = out, .stderr = err, .exit_code = code };
}

pub fn spawn(allocator: Allocator, command: []const u8) !std.process.Child {
    var child = std.process.Child.init(&[_][]const u8{ "/bin/sh", "-c", command }, allocator);
    child.stdin_behavior = .Close;
    try child.spawn();
    return child;
}