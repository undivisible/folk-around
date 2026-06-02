const std = @import("std");
const tools = @import("tools.zig");

const Allocator = std.mem.Allocator;
const Value = std.json.Value;

fn writeMessage(writer: anytype, msg: []const u8) !void {
    try writer.print("Content-Length: {d}\r\n\r\n{s}", .{ msg.len, msg });
}

fn sendObj(allocator: Allocator, writer: anytype, id: i64, result: Value) !void {
    var buf = std.ArrayList(u8).init(allocator);
    defer buf.deinit();
    try buf.writer().print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"result\":", .{id});
    try std.json.stringify(result, .{}, buf.writer());
    try buf.writer().writeByte('}');
    try writeMessage(writer, buf.items);
}

fn sendErr(allocator: Allocator, writer: anytype, id: i64, code: i32, msg: []const u8) !void {
    var buf = std.ArrayList(u8).init(allocator);
    defer buf.deinit();
    try buf.writer().print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"error\":{{\"code\":{d},\"message\":\"{s}\"}}}}", .{id, code, msg});
    try writeMessage(writer, buf.items);
}

fn makeMap(allocator: Allocator) Value {
    return Value{ .object = std.json.ObjectMap.init(allocator) };
}

fn makeArr(allocator: Allocator) Value {
    return Value{ .array = std.json.Array.init(allocator) };
}

fn putObj(map: *Value, key: []const u8, val: Value) !void {
    try map.object.put(key, val);
}

fn readMsg(allocator: Allocator) !?Value {
    var buf: [4096]u8 = undefined;
    const line = try std.io.getStdIn().reader().readUntilDelimiterOrEof(&buf, '\n') orelse return null;
    if (!std.mem.startsWith(u8, line, "Content-Length: ")) return null;
    const len = try std.fmt.parseInt(usize, std.mem.trim(u8, line[16..], " \r"), 10);
    _ = try std.io.getStdIn().reader().readUntilDelimiterOrEof(&buf, '\n'); // blank line

    const body = try allocator.alloc(u8, len);
    defer allocator.free(body);
    _ = try std.io.getStdIn().reader().readNoEof(body);

    return try std.json.parseFromSliceLeaky(Value, allocator, body, .{});
}

pub fn run(allocator: Allocator, verbose: bool, table: *tools.ToolTable) !void {
    const writer = std.io.getStdOut().writer();

    while (true) {
        const msg = readMsg(allocator) catch |err| {
            if (err == error.EndOfStream) break;
            if (verbose) std.debug.print("[folk] err: {}\n", .{err});
            continue;
        } orelse break;

        if (msg != .object) continue;
        const method = msg.object.get("method") orelse continue;
        const method_str = method.string;

        const id_val = msg.object.get("id");
        const is_notif = (id_val == null or id_val.? == .null);

        if (verbose) {
            const id_str: []const u8 = if (id_val) |v| blk: {
                if (v == .integer) break :blk (std.fmt.allocPrint(allocator, "{d}", .{v.integer}) catch "?")
                else break :blk "null";
            } else "null";
            defer if (id_val != null) {
                if (id_val.? == .integer) allocator.free(id_str);
            };
            std.debug.print("[folk] <- {s} id={s}\n", .{method_str, id_str});
        }

        if (std.mem.eql(u8, method_str, "initialize")) {
            if (!is_notif) {
                const id = id_val.?.integer;
                var caps = makeMap(allocator);
                var tcaps = makeMap(allocator);
                try putObj(&tcaps, "listChanged", Value{ .bool = false });
                try putObj(&caps, "tools", tcaps);
                var info = makeMap(allocator);
                try putObj(&info, "name", Value{ .string = "folk-around" });
                try putObj(&info, "version", Value{ .string = "0.1.0" });
                var res = makeMap(allocator);
                try putObj(&res, "protocolVersion", Value{ .string = "2024-11-05" });
                try putObj(&res, "capabilities", caps);
                try putObj(&res, "serverInfo", info);
                try sendObj(allocator, writer, id, res);
            }
        } else if (std.mem.eql(u8, method_str, "ping")) {
            if (!is_notif) try sendObj(allocator, writer, id_val.?.integer, Value{ .null = {} });
        } else if (std.mem.eql(u8, method_str, "tools/list")) {
            if (!is_notif) {
                const id = id_val.?.integer;
                var arr = makeArr(allocator);
                for (table.tools.items) |tool| {
                    var entry = makeMap(allocator);
                    try putObj(&entry, "name", Value{ .string = tool.name });
                    try putObj(&entry, "description", Value{ .string = tool.description });
                    try putObj(&entry, "inputSchema", tool.input_schema);
                    try arr.array.append(entry);
                }
                var res = makeMap(allocator);
                try putObj(&res, "tools", arr);
                try sendObj(allocator, writer, id, res);
            }
        } else if (std.mem.eql(u8, method_str, "tools/call")) {
            if (is_notif) continue;
            const id = id_val.?.integer;
            const params = msg.object.get("params") orelse {
                try sendErr(allocator, writer, id, -32602, "Missing params");
                continue;
            };
            if (params != .object) { try sendErr(allocator, writer, id, -32602, "Params not object"); continue; }
            const name_val = params.object.get("name") orelse {
                try sendErr(allocator, writer, id, -32602, "Missing name"); continue;
            };
            const args = params.object.get("arguments") orelse Value{ .null = {} };

            const result = table.call(name_val.string, args) catch |err| {
                try sendErr(allocator, writer, id, -32603, @errorName(err));
                continue;
            };
            try sendObj(allocator, writer, id, result);
        } else if (!is_notif) {
            try sendErr(allocator, writer, id_val.?.integer, -32601, method_str);
        }
    }
}