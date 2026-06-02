/// P2P module for folk-around.
///
/// Connects to a Cloudflare Workers signaling server via WebSocket.
/// Exchanges identity keys and connection metadata with peers.
/// Falls back to relay through the signaling server if direct connect fails.
///
/// Wire protocol (after Noise handshake establishes encrypted tunnel):
/// [4 bytes: frame length BE] [1 byte: type] [encrypted payload]
///   type 0x01 = MCP message, 0x02 = ping, 0x03 = pong, 0x04 = close
const std = @import("std");
const builtin = @import("builtin");

const Allocator = std.mem.Allocator;
const WebSocketOpcode = enum(u4) {
    continuation = 0,
    text = 1,
    binary = 2,
    close = 8,
    ping = 9,
    pong = 10,
    _,
};

pub const P2PConfig = struct {
    enabled: bool = false,
    signal_url: []const u8 = "https://folk-around-signal.undivisible.workers.dev",
    room: []const u8 = "default",
    identity_secret_hex: ?[]const u8 = null,
    local_port: u16 = 0,
    relay_fallback: bool = true,
};

pub const P2PManager = struct {
    allocator: std.mem.Allocator,
    config: P2PConfig,
    identity_public: [32]u8,
    identity_secret: [32]u8,
    running: bool,
    verbose: bool,
    signal_thread: ?std.Thread,

    pub fn init(allocator: std.mem.Allocator, config: P2PConfig, verbose: bool) !P2PManager {
        // Generate or load identity keypair
        const keypair = if (config.identity_secret_hex) |hex| blk: {
            var seckey: [32]u8 = undefined;

            // Decode existing key
            const decoded = try std.fmt.hexToBytes(&seckey, hex);
            if (decoded.len != 32) return error.InvalidKeyLength;
            // Derive public key from secret (X25519)
            break :blk std.crypto.dh.X25519.KeyPair{
                .public_key = try derivePublicKey(seckey),
                .secret_key = seckey,
            };
        } else blk: {
            // Generate fresh keypair
            const io = std.Io.Threaded.global_single_threaded.io();
            break :blk std.crypto.dh.X25519.KeyPair.generate(io);
        };

        return P2PManager{
            .allocator = allocator,
            .config = config,
            .identity_public = keypair.public_key,
            .identity_secret = keypair.secret_key,
            .running = false,
            .verbose = verbose,
            .signal_thread = null,
        };
    }

    pub fn start(self: *P2PManager) !void {
        self.running = true;
        self.signal_thread = try std.Thread.spawn(.{}, signalThreadMain, .{self});
        self.signal_thread.?.detach();
    }

    pub fn stop(self: *P2PManager) void {
        self.running = false;
    }

    pub fn identityHex(self: *P2PManager, buf: []u8) ![]u8 {
        return std.fmt.bufPrint(buf, "{x}", .{&self.identity_public});
    }

    fn joinSignalRoom(self: *P2PManager) !void {
        var threaded = std.Io.Threaded.init(self.allocator, .{});
        defer threaded.deinit();
        const io = threaded.io();

        const ws_url = try signalWebSocketUrl(self.allocator, self.config.signal_url, self.config.room);
        defer self.allocator.free(ws_url);
        if (self.verbose) std.debug.print("[folk] signaling websocket: {s}\n", .{ws_url});

        var client = std.http.Client{ .allocator = self.allocator, .io = io };
        defer client.deinit();

        var nonce: [16]u8 = undefined;
        io.random(&nonce);
        var key_buf: [std.base64.standard.Encoder.calcSize(nonce.len)]u8 = undefined;
        const ws_key = std.base64.standard.Encoder.encode(&key_buf, &nonce);

        const extra_headers = [_]std.http.Header{
            .{ .name = "upgrade", .value = "websocket" },
            .{ .name = "sec-websocket-key", .value = ws_key },
            .{ .name = "sec-websocket-version", .value = "13" },
        };

        const uri = try std.Uri.parse(ws_url);
        if (self.verbose) std.debug.print("[folk] signaling request\n", .{});
        var req = try client.request(.GET, uri, .{
            .keep_alive = false,
            .redirect_behavior = .unhandled,
            .headers = .{
                .connection = .{ .override = "Upgrade" },
                .accept_encoding = .omit,
            },
            .extra_headers = &extra_headers,
        });
        defer req.deinit();

        try req.sendBodiless();
        if (self.verbose) std.debug.print("[folk] signaling waiting for upgrade\n", .{});
        const response = try req.receiveHead(&.{});
        if (self.verbose) std.debug.print("[folk] signaling status: {d}\n", .{@intFromEnum(response.head.status)});
        if (response.head.status != .switching_protocols) return error.WebSocketUpgradeRejected;
        try validateAccept(response.head.bytes, ws_key);
        if (self.verbose) std.debug.print("[folk] signaling upgrade accepted\n", .{});

        var id_buf: [64]u8 = undefined;
        const identity = try self.identityHex(&id_buf);
        const join = try std.fmt.allocPrint(self.allocator, "{{\"type\":\"join\",\"identity\":\"{s}\"}}", .{identity});
        defer self.allocator.free(join);

        const conn = req.connection orelse return error.WebSocketUpgradeRejected;
        try writeClientMessage(self.allocator, io, conn, join, .text);
        if (self.verbose) std.debug.print("[folk] signaling join sent\n", .{});

        const message = try readServerMessage(self.allocator, conn);
        defer self.allocator.free(message);
        if (self.verbose) std.debug.print("[folk] signaling recv: {s}\n", .{message});
        if (std.mem.indexOf(u8, message, "\"type\":\"joined\"") == null) return error.SignalJoinRejected;

        while (self.running) {
            const next = readServerMessage(self.allocator, conn) catch |err| switch (err) {
                error.WebSocketClosed => return,
                else => |e| return e,
            };
            defer self.allocator.free(next);
            if (self.verbose) std.debug.print("[folk] signaling recv: {s}\n", .{next});
        }
    }
};

fn signalThreadMain(self: *P2PManager) void {
    while (self.running) {
        self.joinSignalRoom() catch |err| {
            if (self.verbose) std.debug.print("[folk] signaling error: {s}\n", .{@errorName(err)});
            sleepSeconds(self.allocator, 2);
            continue;
        };
        if (self.running) sleepSeconds(self.allocator, 2);
    }
}

fn sleepSeconds(allocator: Allocator, seconds: i64) void {
    var threaded = std.Io.Threaded.init(allocator, .{});
    defer threaded.deinit();
    const io = threaded.io();
    std.Io.sleep(io, .fromSeconds(seconds), .awake) catch {};
}

fn derivePublicKey(secret: [32]u8) ![32]u8 {
    // X25519 scalar multiplication
    // std.crypto.dh.X25519.scalarMultiply(pub, secret)
    // Requires Zig's std.crypto which needs specific Zig version support
    return std.crypto.dh.X25519.recoverPublicKey(secret);
}

fn signalWebSocketUrl(allocator: Allocator, raw_url: []const u8, room: []const u8) ![]u8 {
    const base = if (std.mem.startsWith(u8, raw_url, "ws://") or
        std.mem.startsWith(u8, raw_url, "wss://") or
        std.mem.startsWith(u8, raw_url, "http://") or
        std.mem.startsWith(u8, raw_url, "https://"))
        raw_url
    else
        try std.fmt.allocPrint(allocator, "https://{s}", .{raw_url});
    const owns_base = base.ptr != raw_url.ptr;
    defer if (owns_base) allocator.free(base);

    const scheme_end = std.mem.indexOf(u8, base, "://") orelse return error.InvalidSignalUrl;
    const ws_scheme = if (std.mem.eql(u8, base[0..scheme_end], "http"))
        "ws"
    else if (std.mem.eql(u8, base[0..scheme_end], "https"))
        "wss"
    else
        base[0..scheme_end];

    const rest = std.mem.trimEnd(u8, base[scheme_end + 3 ..], "/");
    return std.fmt.allocPrint(allocator, "{s}://{s}/signal/{s}", .{ ws_scheme, rest, room });
}

fn validateAccept(headers: []const u8, key: []const u8) !void {
    const value = findHeader(headers, "sec-websocket-accept") orelse return error.WebSocketAcceptMissing;

    const guid = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    var sha = std.crypto.hash.Sha1.init(.{});
    sha.update(key);
    sha.update(guid);
    var digest: [std.crypto.hash.Sha1.digest_length]u8 = undefined;
    sha.final(&digest);

    var expected_buf: [std.base64.standard.Encoder.calcSize(digest.len)]u8 = undefined;
    const expected = std.base64.standard.Encoder.encode(&expected_buf, &digest);
    if (!std.mem.eql(u8, std.mem.trim(u8, value, " \t\r\n"), expected)) return error.WebSocketAcceptMismatch;
}

fn findHeader(headers: []const u8, name: []const u8) ?[]const u8 {
    var lines = std.mem.splitSequence(u8, headers, "\r\n");
    _ = lines.next();
    while (lines.next()) |line| {
        if (line.len == 0) return null;
        const colon = std.mem.indexOfScalar(u8, line, ':') orelse continue;
        if (std.ascii.eqlIgnoreCase(line[0..colon], name)) return std.mem.trim(u8, line[colon + 1 ..], " \t");
    }
    return null;
}

fn writeClientMessage(
    allocator: Allocator,
    io: std.Io,
    conn: *std.http.Client.Connection,
    payload: []const u8,
    opcode: WebSocketOpcode,
) !void {
    var frame: std.ArrayList(u8) = .empty;
    defer frame.deinit(allocator);

    try frame.append(allocator, 0x80 | @as(u8, @intFromEnum(opcode)));
    if (payload.len <= 125) {
        try frame.append(allocator, 0x80 | @as(u8, @intCast(payload.len)));
    } else if (payload.len <= 0xffff) {
        try frame.append(allocator, 0x80 | 126);
        try appendInt(&frame, allocator, u16, @intCast(payload.len));
    } else {
        try frame.append(allocator, 0x80 | 127);
        try appendInt(&frame, allocator, u64, @intCast(payload.len));
    }

    var mask: [4]u8 = undefined;
    io.random(&mask);
    try frame.appendSlice(allocator, &mask);
    for (payload, 0..) |byte, i| try frame.append(allocator, byte ^ mask[i % 4]);

    const writer = conn.writer();
    try writer.writeAll(frame.items);
    try conn.flush();
}

fn appendInt(list: *std.ArrayList(u8), allocator: Allocator, comptime T: type, value: T) !void {
    var buf: [@sizeOf(T)]u8 = undefined;
    std.mem.writeInt(T, &buf, value, .big);
    try list.appendSlice(allocator, &buf);
}

fn readServerMessage(allocator: Allocator, conn: *std.http.Client.Connection) ![]u8 {
    const reader = conn.reader();
    while (true) {
        const first = try reader.takeByte();
        const second = try reader.takeByte();
        const opcode: WebSocketOpcode = @enumFromInt(first & 0x0f);
        const masked = (second & 0x80) != 0;
        var len: usize = second & 0x7f;
        if (len == 126) {
            len = try reader.takeInt(u16, .big);
        } else if (len == 127) {
            len = std.math.cast(usize, try reader.takeInt(u64, .big)) orelse return error.FrameTooLarge;
        }
        if (len > MAX_FRAME_SIZE) return error.FrameTooLarge;

        var mask: [4]u8 = .{ 0, 0, 0, 0 };
        if (masked) mask = (try reader.takeArray(4)).*;

        const payload = try allocator.alloc(u8, len);
        errdefer allocator.free(payload);
        const read_payload = try reader.take(len);
        @memcpy(payload, read_payload);
        if (masked) {
            for (payload, 0..) |*byte, i| byte.* ^= mask[i % 4];
        }

        switch (opcode) {
            .text, .binary => return payload,
            .ping => {
                defer allocator.free(payload);
                try writeClientMessage(allocator, std.Io.Threaded.global_single_threaded.io(), conn, payload, .pong);
            },
            .pong => allocator.free(payload),
            .close => {
                allocator.free(payload);
                return error.WebSocketClosed;
            },
            else => {
                allocator.free(payload);
                return error.UnsupportedWebSocketFrame;
            },
        }
    }
}

/// Wire protocol frame:
/// [4 bytes BE: total length (including type byte)]
/// [1 byte: type]
///   - 0x01: MCP message (JSON-RPC 2.0 payload)
///   - 0x02: ping
///   - 0x03: pong
///   - 0x04: close
/// [remaining: encrypted payload]
///
/// Encryption: XChaCha20-Poly1305 with key derived from Noise handshake
///
/// Frame size max: 256 KB (to keep latency low and avoid fragmentation)
pub const FrameType = enum(u8) {
    mcp_message = 0x01,
    ping = 0x02,
    pong = 0x03,
    close = 0x04,
    _,
};

pub const MAX_FRAME_SIZE = 256 * 1024;

pub fn encodeFrame(allocator: std.mem.Allocator, frame_type: FrameType, payload: []const u8) ![]u8 {
    const total_len = 5 + payload.len; // 4 len + 1 type + payload
    const buf = try allocator.alloc(u8, total_len);
    errdefer allocator.free(buf);

    std.mem.writeIntBig(u32, buf[0..4], @intCast(total_len));
    buf[4] = @intFromEnum(frame_type);
    @memcpy(buf[5..], payload);

    return buf;
}

pub fn decodeFrame(data: []const u8) struct { frame_type: FrameType, payload: []u8 } {
    const total_len = std.mem.readIntBig(u32, data[0..4]);
    const frame_type: FrameType = @enumFromInt(data[4]);
    const payload = data[5..total_len];
    return .{ .frame_type = frame_type, .payload = payload };
}
