/// P2P module for folk-around.
///
/// Connects to a Cloudflare Workers signaling server via WebSocket.
/// Exchanges identity keys and connection metadata with peers.
/// Processes incoming MCP relay messages from peers.
const std = @import("std");
const builtin = @import("builtin");

const Allocator = std.mem.Allocator;
const RelayAead = std.crypto.aead.chacha_poly.XChaCha20Poly1305;
const relay_ad = "folk-around p2p relay v1";
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
    signal_url: []const u8 = "https://folkaround.undivisible.dev",
    room: []const u8 = "default",
    identity_secret_hex: ?[]const u8 = null,
    local_port: u16 = 0,
    relay_fallback: bool = true,
};

const MCPHandler = *const fn (Allocator, []const u8) anyerror!?[]u8;
const HandshakeState = enum {
    none,
    offered,
    established,
};

pub const P2PManager = struct {
    allocator: Allocator,
    config: P2PConfig,
    identity_public: [32]u8,
    identity_secret: [32]u8,
    running: bool,
    verbose: bool,
    signal_thread: ?std.Thread,
    mcp_handler: ?MCPHandler,
    peer_identity: ?[]u8,
    session_key: ?[RelayAead.key_length]u8,
    handshake_state: HandshakeState,

    pub fn init(allocator: Allocator, config: P2PConfig, verbose: bool) !P2PManager {
        const keypair = if (config.identity_secret_hex) |hex| blk: {
            var seckey: [32]u8 = undefined;
            const decoded = try std.fmt.hexToBytes(&seckey, hex);
            if (decoded.len != 32) return error.InvalidKeyLength;
            break :blk std.crypto.dh.X25519.KeyPair{
                .public_key = try derivePublicKey(seckey),
                .secret_key = seckey,
            };
        } else blk: {
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
            .mcp_handler = null,
            .peer_identity = null,
            .session_key = null,
            .handshake_state = .none,
        };
    }

    pub fn start(self: *P2PManager) !void {
        self.running = true;
        self.signal_thread = try std.Thread.spawn(.{}, signalThreadMain, .{self});
        self.signal_thread.?.detach();
    }

    pub fn stop(self: *P2PManager) void {
        self.running = false;
        if (self.peer_identity) |p| self.allocator.free(p);
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
        try self.handleSignalMessage(conn, io, message);

        while (self.running) {
            const next = readServerMessage(self.allocator, conn) catch |err| switch (err) {
                error.WebSocketClosed => return,
                else => |e| return e,
            };
            defer self.allocator.free(next);
            self.handleSignalMessage(conn, io, next) catch |err| {
                if (self.verbose) std.debug.print("[folk] signal msg err: {s}\n", .{@errorName(err)});
            };
        }
    }

    fn handleSignalMessage(self: *P2PManager, conn: *std.http.Client.Connection, io: std.Io, raw: []const u8) !void {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const message_allocator = arena.allocator();

        const parsed = try std.json.parseFromSliceLeaky(std.json.Value, message_allocator, raw, .{});
        if (parsed != .object) return;
        const msg_type = (parsed.object.get("type") orelse return).string;

        if (std.mem.eql(u8, msg_type, "joined")) {
            const peers = parsed.object.get("peers") orelse return;
            if (peers != .array) return;
            for (peers.array.items) |peer| {
                if (peer != .string) continue;
                if (self.verbose) std.debug.print("[folk] existing peer: {s}\n", .{peer.string});
                try self.sendOffer(conn, io, peer.string);
            }
        }

        // Handle peer joining — send offer
        if (std.mem.eql(u8, msg_type, "peer_joined")) {
            const peer = parsed.object.get("identity") orelse return;
            if (peer != .string) return;
            if (self.verbose) std.debug.print("[folk] peer joined: {s}\n", .{peer.string});

            try self.sendOffer(conn, io, peer.string);
        }

        // Handle incoming offer — send answer
        if (std.mem.eql(u8, msg_type, "offer")) {
            const from = parsed.object.get("from") orelse return;
            if (from != .string) return;
            if (self.verbose) std.debug.print("[folk] got offer from {s}, accepting\n", .{from.string});

            try self.establishSession(from.string);
            try self.sendAnswer(conn, io, from.string);
        }

        if (std.mem.eql(u8, msg_type, "answer")) {
            const from = parsed.object.get("from") orelse return;
            if (from != .string) return;
            if (self.verbose) std.debug.print("[folk] got answer from {s}\n", .{from.string});

            try self.establishSession(from.string);
        }

        // Handle relay message — process MCP call
        if (std.mem.eql(u8, msg_type, "relay")) {
            const from = parsed.object.get("from") orelse return;
            if (from != .string) return;
            try self.ensureRelaySender(from.string);
            const data_val = parsed.object.get("data") orelse return;

            if (self.verbose) std.debug.print("[folk] relay from {s}\n", .{from.string});

            if (self.mcp_handler) |handler| {
                const mcp_json = self.decryptRelayDataValue(self.allocator, data_val) catch |err| {
                    if (self.verbose) std.debug.print("[folk] relay decrypt err: {s}\n", .{@errorName(err)});
                    return;
                };
                defer self.allocator.free(mcp_json);
                const result = handler(self.allocator, mcp_json) catch |err| {
                    if (self.verbose) std.debug.print("[folk] mcp handler err: {s}\n", .{@errorName(err)});
                    return;
                };
                if (result) |resp| {
                    defer self.allocator.free(resp);
                    const encrypted = try self.encryptRelayPayload(self.allocator, resp);
                    defer self.allocator.free(encrypted);
                    var id_hex_buf: [128]u8 = undefined;
                    const id_hex = try self.identityHex(&id_hex_buf);
                    const reply = try std.fmt.allocPrint(self.allocator, "{{\"type\":\"relay\",\"from\":\"{s}\",\"to\":\"{s}\",\"data\":{s}}}", .{ id_hex, from.string, encrypted });
                    defer self.allocator.free(reply);
                    try writeClientMessage(self.allocator, io, conn, reply, .text);
                }
            }
        }
    }

    pub fn establishSession(self: *P2PManager, peer: []const u8) !void {
        const peer_public = try parseIdentityHex(peer);
        self.session_key = try deriveSessionKey(self.identity_secret, self.identity_public, peer_public);
        self.handshake_state = .established;
        try self.setPeer(peer);
    }

    fn setPeer(self: *P2PManager, peer: []const u8) !void {
        if (self.peer_identity) |old| self.allocator.free(old);
        self.peer_identity = try self.allocator.dupe(u8, peer);
    }

    fn ensureRelaySender(self: *P2PManager, from: []const u8) !void {
        const peer = self.peer_identity orelse return error.InvalidPeerIdentity;
        if (!std.mem.eql(u8, peer, from)) return error.InvalidPeerIdentity;
    }

    fn sendOffer(self: *P2PManager, conn: *std.http.Client.Connection, io: std.Io, peer: []const u8) !void {
        try self.setPeer(peer);
        self.handshake_state = .offered;
        var id_hex_buf: [128]u8 = undefined;
        const id_hex = try self.identityHex(&id_hex_buf);
        const offer = try std.fmt.allocPrint(self.allocator, "{{\"type\":\"offer\",\"from\":\"{s}\",\"to\":\"{s}\",\"data\":{{\"type\":\"mcp_relay\"}}}}", .{ id_hex, peer });
        defer self.allocator.free(offer);
        try writeClientMessage(self.allocator, io, conn, offer, .text);
        if (self.verbose) std.debug.print("[folk] sent offer to {s}\n", .{peer});
    }

    fn sendAnswer(self: *P2PManager, conn: *std.http.Client.Connection, io: std.Io, peer: []const u8) !void {
        if (self.handshake_state != .established) try self.establishSession(peer);
        var id_hex_buf: [128]u8 = undefined;
        const id_hex = try self.identityHex(&id_hex_buf);
        const answer = try std.fmt.allocPrint(self.allocator, "{{\"type\":\"answer\",\"from\":\"{s}\",\"to\":\"{s}\",\"data\":{{\"type\":\"mcp_relay\",\"accepted\":true}}}}", .{ id_hex, peer });
        defer self.allocator.free(answer);
        try writeClientMessage(self.allocator, io, conn, answer, .text);
        if (self.verbose) std.debug.print("[folk] sent answer to {s}\n", .{peer});
    }

    pub fn encryptRelayPayload(self: *P2PManager, allocator: Allocator, plaintext: []const u8) ![]u8 {
        const key = self.session_key orelse return error.HandshakeRequired;
        if (self.handshake_state != .established) return error.HandshakeRequired;

        var threaded = std.Io.Threaded.init(allocator, .{});
        defer threaded.deinit();
        const io = threaded.io();
        var nonce: [RelayAead.nonce_length]u8 = undefined;
        io.random(&nonce);

        var ciphertext = try allocator.alloc(u8, plaintext.len + RelayAead.tag_length);
        errdefer allocator.free(ciphertext);
        RelayAead.encrypt(ciphertext[0..plaintext.len], ciphertext[plaintext.len..][0..RelayAead.tag_length], plaintext, relay_ad, nonce, key);

        const nonce_hex = try hexAlloc(allocator, &nonce);
        defer allocator.free(nonce_hex);
        const ciphertext_hex = try hexAlloc(allocator, ciphertext);
        defer allocator.free(ciphertext_hex);
        allocator.free(ciphertext);

        return try std.fmt.allocPrint(allocator, "{{\"v\":1,\"alg\":\"xchacha20poly1305\",\"nonce\":\"{s}\",\"ciphertext\":\"{s}\"}}", .{ nonce_hex, ciphertext_hex });
    }

    pub fn decryptRelayPayload(self: *P2PManager, allocator: Allocator, encrypted: []const u8) ![]u8 {
        const key = self.session_key orelse return error.HandshakeRequired;
        if (self.handshake_state != .established) return error.HandshakeRequired;

        var arena = std.heap.ArenaAllocator.init(allocator);
        defer arena.deinit();
        const message_allocator = arena.allocator();
        const parsed = try std.json.parseFromSliceLeaky(std.json.Value, message_allocator, encrypted, .{});
        if (parsed != .object) return error.InvalidEncryptedPayload;
        const version = parsed.object.get("v") orelse return error.InvalidEncryptedPayload;
        if (version != .integer or version.integer != 1) return error.InvalidEncryptedPayload;
        const alg = parsed.object.get("alg") orelse return error.InvalidEncryptedPayload;
        if (alg != .string or !std.mem.eql(u8, alg.string, "xchacha20poly1305")) return error.InvalidEncryptedPayload;
        const nonce_value = parsed.object.get("nonce") orelse return error.InvalidEncryptedPayload;
        const ciphertext_value = parsed.object.get("ciphertext") orelse return error.InvalidEncryptedPayload;
        if (nonce_value != .string or ciphertext_value != .string) return error.InvalidEncryptedPayload;

        const nonce_bytes = try hexToOwnedBytes(allocator, nonce_value.string);
        defer allocator.free(nonce_bytes);
        if (nonce_bytes.len != RelayAead.nonce_length) return error.InvalidEncryptedPayload;
        const ciphertext = try hexToOwnedBytes(allocator, ciphertext_value.string);
        defer allocator.free(ciphertext);
        if (ciphertext.len < RelayAead.tag_length) return error.InvalidEncryptedPayload;

        var nonce: [RelayAead.nonce_length]u8 = undefined;
        @memcpy(&nonce, nonce_bytes);
        const body_len = ciphertext.len - RelayAead.tag_length;
        const plaintext = try allocator.alloc(u8, body_len);
        errdefer allocator.free(plaintext);
        try RelayAead.decrypt(plaintext, ciphertext[0..body_len], ciphertext[body_len..][0..RelayAead.tag_length].*, relay_ad, nonce, key);
        return plaintext;
    }

    fn decryptRelayDataValue(self: *P2PManager, allocator: Allocator, data_val: std.json.Value) ![]u8 {
        if (data_val == .string) return try self.decryptRelayPayload(allocator, data_val.string);
        var buf: std.ArrayList(u8) = .empty;
        defer buf.deinit(allocator);
        var w: std.Io.Writer.Allocating = .fromArrayList(allocator, &buf);
        try std.json.Stringify.value(data_val, .{}, &w.writer);
        buf = w.toArrayList();
        const encrypted = try buf.toOwnedSlice(allocator);
        defer allocator.free(encrypted);
        return try self.decryptRelayPayload(allocator, encrypted);
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
    return std.crypto.dh.X25519.recoverPublicKey(secret);
}

fn parseIdentityHex(hex: []const u8) ![32]u8 {
    if (hex.len != 64) return error.InvalidPeerIdentity;
    var public: [32]u8 = undefined;
    const decoded = std.fmt.hexToBytes(&public, hex) catch return error.InvalidPeerIdentity;
    if (decoded.len != public.len) return error.InvalidPeerIdentity;
    return public;
}

fn deriveSessionKey(local_secret: [32]u8, local_public: [32]u8, peer_public: [32]u8) ![RelayAead.key_length]u8 {
    const shared = try std.crypto.dh.X25519.scalarmult(local_secret, peer_public);
    var salt: [64]u8 = undefined;
    if (std.mem.order(u8, &local_public, &peer_public) == .lt) {
        @memcpy(salt[0..32], &local_public);
        @memcpy(salt[32..64], &peer_public);
    } else {
        @memcpy(salt[0..32], &peer_public);
        @memcpy(salt[32..64], &local_public);
    }
    const prk = std.crypto.kdf.hkdf.HkdfSha256.extract(&salt, &shared);
    var key: [RelayAead.key_length]u8 = undefined;
    std.crypto.kdf.hkdf.HkdfSha256.expand(&key, relay_ad, prk);
    return key;
}

fn hexAlloc(allocator: Allocator, bytes: []const u8) ![]u8 {
    const out = try allocator.alloc(u8, bytes.len * 2);
    errdefer allocator.free(out);
    const charset = "0123456789abcdef";
    for (bytes, 0..) |byte, i| {
        out[i * 2] = charset[byte >> 4];
        out[i * 2 + 1] = charset[byte & 0x0f];
    }
    return out;
}

fn hexToOwnedBytes(allocator: Allocator, hex: []const u8) ![]u8 {
    if (hex.len % 2 != 0) return error.InvalidEncryptedPayload;
    const out = try allocator.alloc(u8, hex.len / 2);
    errdefer allocator.free(out);
    const decoded = std.fmt.hexToBytes(out, hex) catch return error.InvalidEncryptedPayload;
    if (decoded.len != out.len) return error.InvalidEncryptedPayload;
    return out;
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
    const ws_scheme = if (std.mem.eql(u8, base[0..scheme_end], "http")) "ws" else if (std.mem.eql(u8, base[0..scheme_end], "https")) "wss" else base[0..scheme_end];
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

fn writeClientMessage(allocator: Allocator, io: std.Io, conn: *std.http.Client.Connection, payload: []const u8, opcode: WebSocketOpcode) !void {
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
            for (payload, 0..) |*b, i| b.* ^= mask[i % 4];
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

pub const FrameType = enum(u8) {
    mcp_message = 0x01,
    ping = 0x02,
    pong = 0x03,
    close = 0x04,
    _,
};

pub const MAX_FRAME_SIZE = 256 * 1024;

pub fn encodeFrame(allocator: std.mem.Allocator, frame_type: FrameType, payload: []const u8) ![]u8 {
    const total_len = 5 + payload.len;
    const buf = try allocator.alloc(u8, total_len);
    errdefer allocator.free(buf);
    std.mem.writeInt(u32, buf[0..4], @intCast(total_len), .big);
    buf[4] = @intFromEnum(frame_type);
    @memcpy(buf[5..], payload);
    return buf;
}

pub fn decodeFrame(data: []const u8) !struct { frame_type: FrameType, payload: []const u8 } {
    if (data.len < 5) return error.InvalidFrame;
    const total_len = std.mem.readInt(u32, data[0..4], .big);
    if (total_len < 5 or total_len > data.len or total_len > MAX_FRAME_SIZE) return error.InvalidFrame;
    const frame_type: FrameType = @enumFromInt(data[4]);
    const payload = data[5..total_len];
    return .{ .frame_type = frame_type, .payload = payload };
}

test "decodeFrame rejects truncated input" {
    try std.testing.expectError(error.InvalidFrame, decodeFrame(&.{ 0, 0, 0 }));
}

test "decodeFrame rejects declared length past buffer" {
    const data = [_]u8{ 0, 0, 0, 8, @intFromEnum(FrameType.ping), 1 };
    try std.testing.expectError(error.InvalidFrame, decodeFrame(&data));
}

test "frame round trips payload" {
    const allocator = std.testing.allocator;
    const encoded = try encodeFrame(allocator, .mcp_message, "hello");
    defer allocator.free(encoded);

    const decoded = try decodeFrame(encoded);
    try std.testing.expectEqual(FrameType.mcp_message, decoded.frame_type);
    try std.testing.expectEqualStrings("hello", decoded.payload);
}

test "session keys match for peer identities" {
    var left_secret = [_]u8{0} ** 32;
    var right_secret = [_]u8{0} ** 32;
    left_secret[0] = 1;
    right_secret[0] = 2;

    const left_public = try derivePublicKey(left_secret);
    const right_public = try derivePublicKey(right_secret);
    const left_key = try deriveSessionKey(left_secret, left_public, right_public);
    const right_key = try deriveSessionKey(right_secret, right_public, left_public);

    try std.testing.expectEqualSlices(u8, &left_key, &right_key);
}

test "relay payload requires established handshake" {
    const allocator = std.testing.allocator;
    var manager = try P2PManager.init(allocator, .{}, false);
    defer manager.stop();

    try std.testing.expectError(error.HandshakeRequired, manager.decryptRelayPayload(allocator, "plain"));
}

test "relay payload encrypts and authenticates mcp json" {
    const allocator = std.testing.allocator;
    var left = try P2PManager.init(allocator, .{}, false);
    defer left.stop();
    var right = try P2PManager.init(allocator, .{}, false);
    defer right.stop();

    var left_id_buf: [64]u8 = undefined;
    var right_id_buf: [64]u8 = undefined;
    const left_id = try left.identityHex(&left_id_buf);
    const right_id = try right.identityHex(&right_id_buf);
    try left.establishSession(right_id);
    try right.establishSession(left_id);

    const plaintext = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}";
    const encrypted = try left.encryptRelayPayload(allocator, plaintext);
    defer allocator.free(encrypted);
    try std.testing.expect(std.mem.indexOf(u8, encrypted, "jsonrpc") == null);

    const decrypted = try right.decryptRelayPayload(allocator, encrypted);
    defer allocator.free(decrypted);
    try std.testing.expectEqualStrings(plaintext, decrypted);

    encrypted[encrypted.len - 3] = if (encrypted[encrypted.len - 3] == 'a') 'b' else 'a';
    try std.testing.expectError(error.AuthenticationFailed, right.decryptRelayPayload(allocator, encrypted));
}

test "relay sender must match established peer" {
    const allocator = std.testing.allocator;
    var manager = try P2PManager.init(allocator, .{}, false);
    defer manager.stop();

    const peer = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const other = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    try manager.establishSession(peer);

    try manager.ensureRelaySender(peer);
    try std.testing.expectError(error.InvalidPeerIdentity, manager.ensureRelaySender(other));
}
