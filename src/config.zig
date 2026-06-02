const std = @import("std");

pub const AppConfig = struct {
    signal_url: ?[]const u8 = null,
    room: ?[]const u8 = null,
    http_port: ?u16 = null,
    mode: ?[]const u8 = null,

    pub fn deinit(self: *AppConfig, allocator: std.mem.Allocator) void {
        if (self.signal_url) |value| allocator.free(value);
        if (self.room) |value| allocator.free(value);
        if (self.mode) |value| allocator.free(value);
    }
};

pub fn load(allocator: std.mem.Allocator) !AppConfig {
    const path = try configPath(allocator);
    defer allocator.free(path);

    const io = std.Io.Threaded.global_single_threaded.io();
    const contents = std.Io.Dir.cwd().readFileAlloc(io, path, allocator, .limited(4096)) catch |err| switch (err) {
        error.FileNotFound => return .{},
        else => return err,
    };
    defer allocator.free(contents);

    var result: AppConfig = .{};
    errdefer result.deinit(allocator);

    var lines = std.mem.splitScalar(u8, contents, '\n');
    while (lines.next()) |raw_line| {
        const line = std.mem.trim(u8, raw_line, " \t\r\n");
        if (line.len == 0) continue;
        const eq = std.mem.indexOfScalar(u8, line, '=') orelse continue;
        const key = std.mem.trim(u8, line[0..eq], " \t");
        const value = std.mem.trim(u8, line[eq + 1 ..], " \t");
        if (value.len == 0) continue;

        if (std.mem.eql(u8, key, "signal_url")) {
            if (result.signal_url) |old| allocator.free(old);
            result.signal_url = try allocator.dupe(u8, value);
        } else if (std.mem.eql(u8, key, "room")) {
            if (result.room) |old| allocator.free(old);
            result.room = try allocator.dupe(u8, value);
        } else if (std.mem.eql(u8, key, "http_port")) {
            result.http_port = std.fmt.parseUnsigned(u16, value, 10) catch null;
        } else if (std.mem.eql(u8, key, "mode")) {
            if (result.mode) |old| allocator.free(old);
            result.mode = try allocator.dupe(u8, value);
        }
    }

    return result;
}

pub fn save(allocator: std.mem.Allocator, app_config: AppConfig) !void {
    const dir = try configDir(allocator);
    defer allocator.free(dir);
    const io = std.Io.Threaded.global_single_threaded.io();
    try std.Io.Dir.cwd().createDirPath(io, dir);

    const path = try configPath(allocator);
    defer allocator.free(path);

    var file = try std.Io.Dir.cwd().createFile(io, path, .{ .truncate = true });
    defer file.close(io);

    var contents: std.ArrayList(u8) = .empty;
    defer contents.deinit(allocator);
    if (app_config.signal_url) |value| try contents.print(allocator, "signal_url={s}\n", .{value});
    if (app_config.room) |value| try contents.print(allocator, "room={s}\n", .{value});
    if (app_config.http_port) |value| try contents.print(allocator, "http_port={d}\n", .{value});
    if (app_config.mode) |value| try contents.print(allocator, "mode={s}\n", .{value});
    try file.writeStreamingAll(io, contents.items);
}

pub fn generatePairingCode(allocator: std.mem.Allocator) ![]u8 {
    const words = [_][]const u8{
        "amber",
        "cedar",
        "copper",
        "delta",
        "ember",
        "harbor",
        "indigo",
        "juno",
        "maple",
        "nova",
        "orbit",
        "pixel",
        "quartz",
        "river",
        "signal",
        "violet",
    };
    var seed: [8]u8 = undefined;
    std.Io.Threaded.global_single_threaded.io().random(&seed);
    const number = std.mem.readInt(u64, &seed, .little);
    return std.fmt.allocPrint(allocator, "{s}-{d:0>4}", .{ words[number % words.len], number % 10000 });
}

fn configDir(allocator: std.mem.Allocator) ![]u8 {
    const home = std.mem.span(std.c.getenv("HOME") orelse return error.HomeMissing);
    return std.fs.path.join(allocator, &.{ home, ".config", "folk-around" });
}

fn configPath(allocator: std.mem.Allocator) ![]u8 {
    const dir = try configDir(allocator);
    defer allocator.free(dir);
    return std.fs.path.join(allocator, &.{ dir, "config" });
}
