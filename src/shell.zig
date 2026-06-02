const std = @import("std");
const builtin = @import("builtin");

const Allocator = std.mem.Allocator;

pub const Result = struct {
    stdout: []u8,
    stderr: []u8,
    exit_code: i32,
};

fn exitCode(term: std.process.Child.Term) i32 {
    return switch (term) {
        .exited => |c| @as(i32, @intCast(c)),
        .signal => -1,
        .stopped => -1,
        .unknown => |c| @as(i32, @intCast(c)),
    };
}

pub fn exec(allocator: Allocator, command: []const u8, cwd: ?[]const u8) !Result {
    return execArgv(allocator, &[_][]const u8{ "/bin/sh", "-c", command }, cwd);
}

pub fn execArgv(allocator: Allocator, argv: []const []const u8, cwd: ?[]const u8) !Result {
    var threaded = std.Io.Threaded.init(allocator, .{});
    defer threaded.deinit();
    const io = threaded.io();

    const result = try std.process.run(allocator, io, .{
        .argv = argv,
        .cwd = if (cwd) |dir| .{ .path = dir } else .inherit,
        .stdout_limit = .limited(1024 * 1024),
        .stderr_limit = .limited(1024 * 1024),
    });
    return Result{ .stdout = result.stdout, .stderr = result.stderr, .exit_code = exitCode(result.term) };
}

pub fn execArgvInput(allocator: Allocator, argv: []const []const u8, input: []const u8, cwd: ?[]const u8) !Result {
    var threaded = std.Io.Threaded.init(allocator, .{});
    defer threaded.deinit();
    const io = threaded.io();
    var child = try std.process.spawn(io, .{
        .argv = argv,
        .cwd = if (cwd) |dir| .{ .path = dir } else .inherit,
        .stdin = .pipe,
        .stdout = .pipe,
        .stderr = .pipe,
    });
    defer child.kill(io);

    try child.stdin.?.writeStreamingAll(io, input);
    child.stdin.?.close(io);
    child.stdin = null;

    var stdout_buffer: [4096]u8 = undefined;
    var stderr_buffer: [4096]u8 = undefined;
    var stdout_reader = child.stdout.?.readerStreaming(io, &stdout_buffer);
    var stderr_reader = child.stderr.?.readerStreaming(io, &stderr_buffer);

    const out = try stdout_reader.interface.allocRemaining(allocator, .limited(1024 * 1024));
    errdefer allocator.free(out);
    const err = try stderr_reader.interface.allocRemaining(allocator, .limited(1024 * 1024));
    errdefer allocator.free(err);

    const term = try child.wait(io);
    return Result{ .stdout = out, .stderr = err, .exit_code = exitCode(term) };
}

pub fn spawn(allocator: Allocator, command: []const u8) !void {
    var threaded = std.Io.Threaded.init(allocator, .{});
    defer threaded.deinit();
    const io = threaded.io();
    _ = try std.process.spawn(io, .{
        .argv = &[_][]const u8{ "/bin/sh", "-c", command },
        .stdin = .ignore,
        .stdout = .ignore,
        .stderr = .ignore,
    });
}
