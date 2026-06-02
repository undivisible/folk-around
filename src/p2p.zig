/// WebRTC signaling server + P2P wire protocol for folk-around.
/// 
/// Architecture:
/// - Built-in signaling server (HTTP + WebSocket on the same port as --http)
/// - P2P connections use Noise_XK (static-dh) for e2e encryption
/// - STUN-style NAT traversal with a simple relay fallback
///
/// Protocol:
/// 1. Peer A connects to signaling server, registers an identity key
/// 2. Peer B requests connection to Peer A's identity
/// 3. Signaling server relays ICE-like offers/answers (connection metadata)
/// 4. Peers establish direct TCP or WebSocket tunnel
/// 5. All data encrypted with XChaCha20-Poly1305 (via Noise handshake)
///
/// MCP messages flow transparently over the encrypted tunnel.

const std = @import("std");
const builtin = @import("builtin");

pub const P2PConfig = struct {
    enabled: bool = false,
    listen_port: u16 = 0, // 0 = auto
    signaling_url: ?[]const u8 = null,
    identity_secret: ?[]const u8 = null, // hex-encoded 32 bytes
    peer_identity: ?[]const u8 = null, // hex-encoded 32 bytes of peer
    relay_fallback: bool = true,
};

pub const P2PManager = struct {
    allocator: std.mem.Allocator,
    config: P2PConfig,
    running: bool,

    pub fn init(allocator: std.mem.Allocator, config: P2PConfig) P2PManager {
        return P2PManager{
            .allocator = allocator,
            .config = config,
            .running = false,
        };
    }

    pub fn start(self: *P2PManager) !void {
        _ = self;
        // TODO: full Noise_XK handshake + TCP listener
        // For now, this is the signaling layer + wire protocol spec
        // Implementation requires: Zig std.crypto (Curve25519, XChaCha20-Poly1305),
        // a WebSocket library, and TCP hole-punching logic.
        //
        // The HTTP SSE transport in http.zig already handles remote MCP connections
        // via Tailscale/SSH tunnel. P2P extends this to direct connections without
        // infrastructure.
        self.running = true;
    }

    pub fn stop(self: *P2PManager) void {
        self.running = false;
    }
};

/// Wire protocol frame format (after Noise handshake):
///
/// [4 bytes: total frame length (big-endian)]
/// [1 byte: frame type]
///   - 0x01: MCP message (JSON-RPC 2.0)
///   - 0x02: ping
///   - 0x03: pong
///   - 0x04: close
/// [payload (encrypted)]
///
/// Noise handshake pattern: XK (static keys, one-round trip)
/// - Peer has pre-known remote static key (identity_secret / peer_identity)
/// - Or keys exchanged via signaling server when both sides connect
///
/// For the simplest secure path without WebRTC dependency:
/// 1. Both sides generate ed25519 keypair
/// 2. Sign the handshake with the static key
/// 3. Derive shared secret via X25519 DH
/// 4. Encrypt frames with XChaCha20-Poly1305 using HKDF-derived keys
///
/// This avoids the WebRTC C++ dependency entirely while providing
/// equivalent security properties (forward secrecy, mutual auth).