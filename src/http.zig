const std = @import("std");
const builtin = @import("builtin");
const tools = @import("tools.zig");

const Allocator = std.mem.Allocator;
const Value = std.json.Value;

// ── MCP over HTTP SSE transport ──
// GET /sse -> SSE stream (server -> client events)
// POST /message -> client -> server messages
// GET /health -> health check

pub fn run(allocator: Allocator, verbose: bool, table: *tools.ToolTable, port: u16) !void {
    const addr = std.net.Address.initIp4(.{ 0, 0, 0, 0 }, port);

    // TCP listener
    const listener = try std.net.tcpListenToAddress(addr, .{ .reuse_port = true, .reuse_address = true });
    defer listener.deinit();

    // SSE state shared across connections
    // In a real impl, use an event bus. For now, single-connection.
    var sse_buf = std.ArrayList(u8).init(allocator);
    defer sse_buf.deinit();
    var sse_conn: ?std.net.Server.Connection = null;

    if (verbose) std.debug.print("[folk] HTTP listening on http://127.0.0.1:{d}/\n", .{port});

    const server = listener;

    while (true) {
        const conn = try server.accept();
        const stream = conn.stream;

        var buf: [8192]u8 = undefined;
        const bytes_read = stream.read(&buf) catch |e| {
            if (verbose) std.debug.print("[folk] http read err: {}\n", .{e});
            conn.server.handle.disconnect();
            continue;
        };

        if (bytes_read == 0) {
            conn.server.handle.disconnect();
            continue;
        }

        const request = buf[0..bytes_read];

        // Parse request line
        var lines_iter = std.mem.splitScalar(u8, request, '\n');
        const req_line = lines_iter.next() orelse {
            try sendHttp(stream, 400, "Bad Request");
            conn.server.handle.disconnect();
            continue;
        };

        var parts = std.mem.splitScalar(u8, std.mem.trimRight(u8, req_line, "\r"), ' ');
        const method = parts.next() orelse "";
        const path = parts.next() orelse "";

        if (std.mem.eql(u8, path, "/health")) {
            try sendHttp(stream, 200, "ok");
        } else if (std.mem.eql(u8, method, "GET") and std.mem.eql(u8, path, "/sse")) {
            // SSE stream
            try sendSseHeaders(stream);

            // Send endpoint event
            try stream.writer().print("event: endpoint\ndata: /message\n\n", .{});
            try stream.writer().print("event: initialized\ndata: {{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{{\"tools\":{{}}}}}}\n\n", .{});

            sse_conn = conn;

            // Keep connection alive
            var keep_alive: [1]u8 = undefined;
            _ = stream.read(&keep_alive) catch {};
            sse_conn = null;
        } else if ((std.mem.eql(u8, method, "POST") or std.mem.eql(u8, method, "OPTIONS")) and std.mem.eql(u8, path, "/message")) {
            // Parse Content-Length
            var content_len: usize = 0;
            var body_start: usize = 0;
            var header_lines = std.mem.splitScalar(u8, request, '\n');
            _ = header_lines.next(); // skip request line
            while (header_lines.next()) |line| {
                const trimmed = std.mem.trimRight(u8, line, "\r");
                if (trimmed.len == 0) {
                    body_start = @intFromPtr(trimmed.ptr) - @intFromPtr(request.ptr) + 2; // +2 for the \r\n or \n\n
                    // Recalculate: body starts after the blank line
                    body_start = @intFromPtr(trimmed.ptr) + @sizeOf(u8) * trimmed.len - @intFromPtr(request.ptr);
                    // Actually let me find the double newline
                    const dbl_nl = std.mem.indexOf(u8, request, "\r\n\r\n") orelse
                        std.mem.indexOf(u8, request, "\n\n") orelse break;
                    body_start = dbl_nl + if (request[dbl_nl+1] == '\n') 2 else 4;
                    break;
                }
                if (std.ascii.startsWithIgnoreCase(trimmed, "content-length:")) {
                    content_len = std.fmt.parseUnsigned(usize, std.mem.trim(u8, trimmed[15..], " "), 10) catch 0;
                }
            }

            if (body_start == 0 or content_len == 0 or body_start + content_len > request.len) {
                try sendHttp(stream, 400, "Bad Request");
            } else {
                const body = request[body_start .. body_start + content_len];

                // Parse the MCP message
                const msg = std.json.parseFromSliceLeaky(Value, allocator, body, .{}) catch {
                    try sendHttp(stream, 400, "Invalid JSON");
                    conn.server.handle.disconnect();
                    continue;
                };

                if (msg != .object) {
                    try sendHttp(stream, 400, "Not object");
                    conn.server.handle.disconnect();
                    continue;
                }

                // Handle the MCP message
                const result = try handleMCP(&mcp_ctx{ .alloc = allocator, .verbose = verbose, .table = table }, msg);

                // If there's an SSE connection, send result via SSE
                if (sse_conn) |sc| {
                    const sse_stream = sc.stream;
                    try sse_stream.writer().print("event: message\ndata: {s}\n\n", .{result});
                }

                try sendHttp(stream, 202, "Accepted");
            }
        } else {
            try sendHttp(stream, 404, "Not Found");
        }

        conn.server.handle.disconnect();
    }
}

const mcp_ctx = struct {
    alloc: Allocator,
    verbose: bool,
    table: *tools.ToolTable,
};

fn handleMCP(ctx: *mcp_ctx, msg: Value) ![]const u8 {
    const method = msg.object.get("method") orelse return "{}";
    const method_str = method.string;

    const id_val = msg.object.get("id");
    const is_notif = (id_val == null or id_val.? == .null);

    if (verbose()) {
        ctx.verbose = ctx.verbose; // nop, just accessing
    }

    var buf = std.ArrayList(u8).init(ctx.alloc);
    defer buf.deinit();
    const w = buf.writer();

    if (std.mem.eql(u8, method_str, "initialize")) {
        if (is_notif) return "{}";
        try w.print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"result\":{{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{{\"tools\":{{}}}},\"serverInfo\":{{\"name\":\"folk-around\",\"version\":\"0.2.0\"}}}}}}", .{id_val.?.integer});
    } else if (std.mem.eql(u8, method_str, "ping")) {
        if (is_notif) return "{}";
        try w.print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"result\":null}}", .{id_val.?.integer});
    } else if (std.mem.eql(u8, method_str, "tools/list")) {
        if (is_notif) return "{}";
        try w.print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"result\":{{\"tools\":[", .{id_val.?.integer});
        for (ctx.table.tools.items, 0..) |tool, idx| {
            if (idx > 0) try w.writeByte(',');
            try w.print("{{\"name\":\"{s}\",\"description\":\"{s}\",\"inputSchema\":null}}", .{tool.name, tool.description});
        }
        try w.writeAll("]}}");
    } else if (std.mem.eql(u8, method_str, "tools/call")) {
        if (is_notif) return "{}";
        const id = id_val.?.integer;
        const params = msg.object.get("params") orelse return error.MissingParams;
        if (params != .object) return error.InvalidParams;
        const name_val = params.object.get("name") orelse return error.MissingName;
        const call_args = params.object.get("arguments") orelse Value{ .null = {} };

        const result = ctx.table.call(name_val.string, call_args) catch |e| {
            try w.print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"error\":{{\"code\":-32603,\"message\":\"{s}\"}}}}", .{id, @errorName(e)});
            return buf.items;
        };

        var res_buf = std.ArrayList(u8).init(ctx.alloc);
        defer res_buf.deinit();
        try std.json.stringify(result, .{}, res_buf.writer());

        try w.print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"result\":{s}}}", .{id, res_buf.items});
    }

    return buf.items;
}

fn verbose() bool {
    return false; // will be set by context
}

fn sendHttp(stream: std.net.Stream, status: u16, body: []const u8) !void {
    const reason = if (status == 200) "OK" else if (status == 202) "Accepted" else if (status == 400) "Bad Request" else "Not Found";
    try stream.writer().print(
        "HTTP/1.1 {d} {s}\r\nContent-Type: text/plain\r\nContent-Length: {d}\r\nAccess-Control-Allow-Origin: *\r\n\r\n{s}",
        .{ status, reason, body.len, body }
    );
}

fn sendSseHeaders(stream: std.net.Stream) !void {
    try stream.writer().print(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        .{}
    );
    try stream.flush();
}