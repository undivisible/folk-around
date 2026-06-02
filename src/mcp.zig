const std = @import("std");
const tools = @import("tools.zig");

const Allocator = std.mem.Allocator;
const Value = std.json.Value;

fn writeStdout(bytes: []const u8) !void {
    const io = std.Io.Threaded.global_single_threaded.io();
    var buffer: [4096]u8 = undefined;
    var writer_state = std.Io.File.stdout().writer(io, &buffer);
    try writer_state.interface.writeAll(bytes);
    try writer_state.interface.flush();
}

fn readByte(fd: std.posix.fd_t) !?u8 {
    var byte: [1]u8 = undefined;
    const n = try std.posix.read(fd, &byte);
    if (n == 0) return null;
    return byte[0];
}

fn readLine(allocator: Allocator, fd: std.posix.fd_t) !?[]u8 {
    var line: std.ArrayList(u8) = .empty;
    errdefer line.deinit(allocator);

    while (true) {
        const byte = try readByte(fd) orelse {
            if (line.items.len == 0) return null;
            break;
        };
        try line.append(allocator, byte);
        if (byte == '\n') break;
        if (line.items.len > 8192) return error.HeaderTooLarge;
    }

    return try line.toOwnedSlice(allocator);
}

fn readExact(fd: std.posix.fd_t, dest: []u8) !void {
    var offset: usize = 0;
    while (offset < dest.len) {
        const n = try std.posix.read(fd, dest[offset..]);
        if (n == 0) return error.EndOfStream;
        offset += n;
    }
}

fn readMessage(allocator: Allocator) !?[]u8 {
    const stdin = std.posix.STDIN_FILENO;
    const first_line = try readLine(allocator, stdin) orelse return null;
    defer allocator.free(first_line);

    const trimmed = std.mem.trim(u8, first_line, " \t\r\n");
    if (trimmed.len == 0) return readMessage(allocator);

    if (!std.ascii.startsWithIgnoreCase(trimmed, "content-length:")) {
        return try allocator.dupe(u8, trimmed);
    }

    const len_text = std.mem.trim(u8, trimmed["content-length:".len..], " \t");
    const len = try std.fmt.parseUnsigned(usize, len_text, 10);

    while (true) {
        const line = try readLine(allocator, stdin) orelse return error.EndOfStream;
        defer allocator.free(line);
        if (std.mem.trim(u8, line, " \t\r\n").len == 0) break;
    }

    const body = try allocator.alloc(u8, len);
    errdefer allocator.free(body);
    try readExact(stdin, body);
    return body;
}

fn writeMessage(allocator: Allocator, json: []const u8) !void {
    const header = try std.fmt.allocPrint(allocator, "Content-Length: {d}\r\n\r\n", .{json.len});
    defer allocator.free(header);
    try writeStdout(header);
    try writeStdout(json);
}

fn writeJsonValue(allocator: Allocator, value: Value) ![]u8 {
    var buf: std.ArrayList(u8) = .empty;
    errdefer buf.deinit(allocator);
    var writer: std.Io.Writer.Allocating = .fromArrayList(allocator, &buf);
    try std.json.Stringify.value(value, .{}, &writer.writer);
    buf = writer.toArrayList();
    return try buf.toOwnedSlice(allocator);
}

fn response(allocator: Allocator, id: Value, result: Value) ![]u8 {
    const id_json = try writeJsonValue(allocator, id);
    defer allocator.free(id_json);
    const result_json = try writeJsonValue(allocator, result);
    defer allocator.free(result_json);
    return try std.fmt.allocPrint(allocator, "{{\"jsonrpc\":\"2.0\",\"id\":{s},\"result\":{s}}}", .{ id_json, result_json });
}

fn errorResponse(allocator: Allocator, id: Value, code: i32, msg: []const u8) ![]u8 {
    const id_json = try writeJsonValue(allocator, id);
    defer allocator.free(id_json);
    const msg_json = try writeJsonValue(allocator, Value{ .string = msg });
    defer allocator.free(msg_json);
    return try std.fmt.allocPrint(allocator, "{{\"jsonrpc\":\"2.0\",\"id\":{s},\"error\":{{\"code\":{d},\"message\":{s}}}}}", .{ id_json, code, msg_json });
}

fn makeMap(allocator: Allocator) !Value {
    return Value{ .object = try std.json.ObjectMap.init(allocator, &.{}, &.{}) };
}

fn makeArr(allocator: Allocator) Value {
    return Value{ .array = std.json.Array.init(allocator) };
}

fn putObj(allocator: Allocator, map: *Value, key: []const u8, val: Value) !void {
    try map.object.put(allocator, key, val);
}

pub fn handleMessage(allocator: Allocator, verbose: bool, table: *tools.ToolTable, msg: Value) !?[]u8 {
    if (msg != .object) return null;
    const method = msg.object.get("method") orelse return null;
    if (method != .string) return null;
    const method_str = method.string;

    const id_val = msg.object.get("id");
    const is_notif = id_val == null or id_val.? == .null;

    if (verbose) std.debug.print("[folk] <- {s}\n", .{method_str});

    if (std.mem.eql(u8, method_str, "initialize")) {
        if (is_notif) return null;
        var caps = try makeMap(allocator);
        var tcaps = try makeMap(allocator);
        try putObj(allocator, &tcaps, "listChanged", Value{ .bool = false });
        try putObj(allocator, &caps, "tools", tcaps);
        var info = try makeMap(allocator);
        try putObj(allocator, &info, "name", Value{ .string = "folk-around" });
        try putObj(allocator, &info, "version", Value{ .string = "0.1.0" });
        var res = try makeMap(allocator);
        try putObj(allocator, &res, "protocolVersion", Value{ .string = "2024-11-05" });
        try putObj(allocator, &res, "capabilities", caps);
        try putObj(allocator, &res, "serverInfo", info);
        return try response(allocator, id_val.?, res);
    }

    if (std.mem.eql(u8, method_str, "notifications/initialized")) return null;

    if (std.mem.eql(u8, method_str, "ping")) {
        if (is_notif) return null;
        return try response(allocator, id_val.?, Value{ .object = try std.json.ObjectMap.init(allocator, &.{}, &.{}) });
    }

    if (std.mem.eql(u8, method_str, "tools/list")) {
        if (is_notif) return null;
        var arr = makeArr(allocator);
        for (table.tools.items) |tool| {
            var entry = try makeMap(allocator);
            try putObj(allocator, &entry, "name", Value{ .string = tool.name });
            try putObj(allocator, &entry, "description", Value{ .string = tool.description });
            try putObj(allocator, &entry, "inputSchema", tool.input_schema);
            try arr.array.append(entry);
        }
        var res = try makeMap(allocator);
        try putObj(allocator, &res, "tools", arr);
        return try response(allocator, id_val.?, res);
    }

    if (std.mem.eql(u8, method_str, "tools/call")) {
        if (is_notif) return null;
        const params = msg.object.get("params") orelse return try errorResponse(allocator, id_val.?, -32602, "Missing params");
        if (params != .object) return try errorResponse(allocator, id_val.?, -32602, "Params not object");
        const name_val = params.object.get("name") orelse return try errorResponse(allocator, id_val.?, -32602, "Missing name");
        if (name_val != .string) return try errorResponse(allocator, id_val.?, -32602, "Name not string");
        const args = params.object.get("arguments") orelse Value{ .object = try std.json.ObjectMap.init(allocator, &.{}, &.{}) };

        const result = table.call(name_val.string, args) catch |err| {
            return try errorResponse(allocator, id_val.?, -32603, @errorName(err));
        };
        return try response(allocator, id_val.?, result);
    }

    if (!is_notif) return try errorResponse(allocator, id_val.?, -32601, method_str);
    return null;
}

pub fn run(allocator: std.mem.Allocator, verbose: bool, table: *tools.ToolTable) !void {
    while (true) {
        const body = readMessage(allocator) catch |err| {
            if (err == error.EndOfStream) break;
            if (verbose) std.debug.print("[folk] read error: {s}\n", .{@errorName(err)});
            continue;
        } orelse break;
        defer allocator.free(body);

        const msg = std.json.parseFromSliceLeaky(Value, allocator, body, .{}) catch |err| {
            if (verbose) std.debug.print("[folk] json error: {s}\n", .{@errorName(err)});
            continue;
        };

        const out = try handleMessage(allocator, verbose, table, msg) orelse continue;
        defer allocator.free(out);
        try writeMessage(allocator, out);
    }
}
