const std = @import("std");
const mcp = @import("mcp.zig");
const tools = @import("tools.zig");

const Allocator = std.mem.Allocator;
const Value = std.json.Value;

fn sendResponse(writer: *std.Io.Writer, status: u16, reason: []const u8, content_type: []const u8, body: []const u8) !void {
    try writer.print(
        "HTTP/1.1 {d} {s}\r\ncontent-type: {s}\r\ncontent-length: {d}\r\naccess-control-allow-origin: *\r\naccess-control-allow-headers: content-type\r\naccess-control-allow-methods: GET, POST, OPTIONS\r\nconnection: close\r\n\r\n",
        .{ status, reason, content_type, body.len },
    );
    try writer.writeAll(body);
    try writer.flush();
}

fn sendSse(io: std.Io, writer: *std.Io.Writer) !void {
    try writer.writeAll(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\naccess-control-allow-origin: *\r\naccess-control-allow-headers: content-type\r\naccess-control-allow-methods: GET, POST, OPTIONS\r\nconnection: keep-alive\r\n\r\n",
    );
    try writer.writeAll("event: endpoint\ndata: /message\n\n");
    try writer.flush();
    while (true) {
        std.Io.sleep(io, .fromSeconds(15), .awake) catch return;
        writer.writeAll(": keepalive\n\n") catch return;
        writer.flush() catch return;
    }
}

fn findHeaderEnd(request: []const u8) ?struct { offset: usize, len: usize } {
    if (std.mem.indexOf(u8, request, "\r\n\r\n")) |offset| return .{ .offset = offset, .len = 4 };
    if (std.mem.indexOf(u8, request, "\n\n")) |offset| return .{ .offset = offset, .len = 2 };
    return null;
}

fn contentLength(headers: []const u8) usize {
    var lines = std.mem.splitScalar(u8, headers, '\n');
    while (lines.next()) |line| {
        const trimmed = std.mem.trim(u8, line, " \t\r\n");
        if (std.ascii.startsWithIgnoreCase(trimmed, "content-length:")) {
            return std.fmt.parseUnsigned(usize, std.mem.trim(u8, trimmed["content-length:".len..], " \t"), 10) catch 0;
        }
    }
    return 0;
}

fn readRequest(allocator: Allocator, stream: std.Io.net.Stream) ![]u8 {
    var request: std.ArrayList(u8) = .empty;
    errdefer request.deinit(allocator);

    var chunk: [4096]u8 = undefined;
    var expected_len: ?usize = null;

    while (true) {
        const n = try std.posix.read(stream.socket.handle, &chunk);
        if (n == 0) break;
        try request.appendSlice(allocator, chunk[0..n]);

        if (expected_len == null) {
            if (findHeaderEnd(request.items)) |end| {
                const len = contentLength(request.items[0..end.offset]);
                expected_len = end.offset + end.len + len;
            }
        }

        if (expected_len) |len| {
            if (request.items.len >= len) break;
        } else if (request.items.len > 64 * 1024) {
            return error.HeaderTooLarge;
        }
    }

    return try request.toOwnedSlice(allocator);
}

const ClientContext = struct {
    allocator: Allocator,
    verbose: bool,
    table: *tools.ToolTable,
    stream: std.Io.net.Stream,
};

fn handlePost(allocator: Allocator, verbose: bool, table: *tools.ToolTable, writer: *std.Io.Writer, request: []const u8) !void {
    const end = findHeaderEnd(request) orelse {
        try sendResponse(writer, 400, "Bad Request", "text/plain", "missing headers");
        return;
    };
    const body_start = end.offset + end.len;
    const len = contentLength(request[0..end.offset]);
    if (len == 0 or body_start + len > request.len) {
        try sendResponse(writer, 400, "Bad Request", "text/plain", "missing body");
        return;
    }

    var arena = std.heap.ArenaAllocator.init(allocator);
    defer arena.deinit();
    const message_allocator = arena.allocator();

    const msg = std.json.parseFromSliceLeaky(Value, message_allocator, request[body_start .. body_start + len], .{}) catch {
        try sendResponse(writer, 400, "Bad Request", "text/plain", "invalid json");
        return;
    };

    const out = try mcp.handleMessage(message_allocator, verbose, table, msg) orelse {
        try sendResponse(writer, 202, "Accepted", "text/plain", "accepted");
        return;
    };
    try sendResponse(writer, 200, "OK", "application/json", out);
}

fn handleClient(ctx: *ClientContext) void {
    const allocator = ctx.allocator;
    var threaded = std.Io.Threaded.init(allocator, .{});
    defer threaded.deinit();
    const io = threaded.io();
    defer ctx.stream.close(io);
    defer allocator.destroy(ctx);

    var write_buffer: [8192]u8 = undefined;
    var writer_state = ctx.stream.writer(io, &write_buffer);
    const writer = &writer_state.interface;

    const request = readRequest(allocator, ctx.stream) catch {
        sendResponse(writer, 400, "Bad Request", "text/plain", "bad request") catch {};
        return;
    };
    defer allocator.free(request);

    var lines = std.mem.splitScalar(u8, request, '\n');
    const request_line = std.mem.trim(u8, lines.next() orelse "", " \t\r\n");
    var parts = std.mem.splitScalar(u8, request_line, ' ');
    const method = parts.next() orelse "";
    const path = parts.next() orelse "";

    if (std.mem.eql(u8, method, "OPTIONS")) {
        sendResponse(writer, 204, "No Content", "text/plain", "") catch {};
    } else if (std.mem.eql(u8, method, "GET") and std.mem.eql(u8, path, "/health")) {
        sendResponse(writer, 200, "OK", "text/plain", "ok") catch {};
    } else if (std.mem.eql(u8, method, "GET") and std.mem.eql(u8, path, "/sse")) {
        sendSse(io, writer) catch {};
    } else if (std.mem.eql(u8, method, "POST") and std.mem.eql(u8, path, "/message")) {
        handlePost(allocator, ctx.verbose, ctx.table, writer, request) catch {};
    } else {
        sendResponse(writer, 404, "Not Found", "text/plain", "not found") catch {};
    }
}

pub fn run(allocator: std.mem.Allocator, verbose: bool, table: *tools.ToolTable, port: u16) !void {
    var threaded = std.Io.Threaded.init(allocator, .{});
    defer threaded.deinit();
    const io = threaded.io();
    var address = listenAddress(port);
    var server = try address.listen(io, .{ .reuse_address = true });
    defer server.deinit(io);

    if (verbose) std.debug.print("[folk] HTTP listening on http://127.0.0.1:{d}/\n", .{port});

    while (true) {
        const stream = try server.accept(io);
        const ctx = try allocator.create(ClientContext);
        ctx.* = .{
            .allocator = allocator,
            .verbose = verbose,
            .table = table,
            .stream = stream,
        };
        const thread = std.Thread.spawn(.{}, handleClient, .{ctx}) catch {
            stream.close(io);
            allocator.destroy(ctx);
            continue;
        };
        thread.detach();
    }
}

fn listenAddress(port: u16) std.Io.net.IpAddress {
    return .{ .ip4 = std.Io.net.Ip4Address.loopback(port) };
}

test "http listen address is loopback only" {
    const address = listenAddress(8080);
    try std.testing.expect(address == .ip4);
    try std.testing.expectEqualSlices(u8, &.{ 127, 0, 0, 1 }, &address.ip4.bytes);
    try std.testing.expectEqual(@as(u16, 8080), address.ip4.port);
}
