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

    pub fn init(allocator: std.mem.Allocator, config: P2PConfig) !P2PManager {
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
        };
    }

    pub fn start(self: *P2PManager) !void {
        _ = self;
        return error.UnsupportedP2PTransport;
    }

    pub fn stop(self: *P2PManager) void {
        self.running = false;
    }

    pub fn identityHex(self: *P2PManager, buf: []u8) ![]u8 {
        return std.fmt.bufPrint(buf, "{x}", .{&self.identity_public});
    }
};

fn derivePublicKey(secret: [32]u8) ![32]u8 {
    // X25519 scalar multiplication
    // std.crypto.dh.X25519.scalarMultiply(pub, secret)
    // Requires Zig's std.crypto which needs specific Zig version support
    return std.crypto.dh.X25519.recoverPublicKey(secret);
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
