const std = @import("std");
const builtin = @import("builtin");

const Allocator = std.mem.Allocator;

pub const Result = struct {
    stdout: []u8,
    stderr: []u8,
    exit_code: i32,
};

pub fn exec(allocator: Allocator, command: []const u8, cwd: ?[]const u8) !Result {
    var threaded = std.Io.Threaded.init(allocator, .{});
    defer threaded.deinit();
    const io = threaded.io();
    var nonce: [16]u8 = undefined;
    io.random(&nonce);
    const out_path = try std.fmt.allocPrint(allocator, "/tmp/folk-around-{x}.out", .{&nonce});
    defer allocator.free(out_path);
    const err_path = try std.fmt.allocPrint(allocator, "/tmp/folk-around-{x}.err", .{&nonce});
    defer allocator.free(err_path);

    const wrapped = try std.fmt.allocPrint(allocator, "{{ {s}; }} > {s} 2> {s}", .{ command, out_path, err_path });
    defer allocator.free(wrapped);

    var child = try std.process.spawn(io, .{
        .argv = &[_][]const u8{ "/bin/sh", "-c", wrapped },
        .cwd = if (cwd) |dir| .{ .path = dir } else .inherit,
        .stdin = .ignore,
        .stdout = .ignore,
        .stderr = .ignore,
    });
    const term = try child.wait(io);

    const out = std.Io.Dir.readFileAlloc(.cwd(), io, out_path, allocator, .limited(1024 * 1024)) catch |err| switch (err) {
        error.FileNotFound => try allocator.dupe(u8, ""),
        else => return err,
    };
    errdefer allocator.free(out);
    const err = std.Io.Dir.readFileAlloc(.cwd(), io, err_path, allocator, .limited(1024 * 1024)) catch |read_err| switch (read_err) {
        error.FileNotFound => try allocator.dupe(u8, ""),
        else => return read_err,
    };
    errdefer allocator.free(err);
    std.Io.Dir.deleteFileAbsolute(io, out_path) catch {};
    std.Io.Dir.deleteFileAbsolute(io, err_path) catch {};

    const code = switch (term) {
        .exited => |c| @as(i32, @intCast(c)),
        .signal => -1,
        .stopped => -1,
        .unknown => |c| @as(i32, @intCast(c)),
    };

    return Result{ .stdout = out, .stderr = err, .exit_code = code };
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
