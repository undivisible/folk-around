/// folk-around P2P signaling server
/// Cloudflare Worker + Durable Object for WebSocket-based peer discovery.
///
/// Architecture:
///   Client connects via WebSocket to /signal/:room
///   Durable Object per room tracks connected peers
///   Peers exchange identity keys and connection metadata through the room
///   Once both peers have each other's info, they connect directly
///   (or fall back to relay through the signaling server if NAT punch fails)
///
/// Protocol (JSON messages over WebSocket):
///   Client -> Server:
///     { type: "join", identity: "<ed25519-pubkey-hex>" }
///     { type: "offer", to: "<peer-identity>", data: { host, port, key } }
///     { type: "answer", to: "<peer-identity>", data: { host, port, key } }
///     { type: "relay", to: "<peer-identity>", data: <encrypted-bytes> }
///
///   Server -> Client:
///     { type: "joined", room: "<room>", peers: ["<identity>", ...] }
///     { type: "peer_joined", identity: "<ed25519-pubkey-hex>" }
///     { type: "peer_left", identity: "<ed25519-pubkey-hex>" }
///     { type: "offer", from: "<identity>", data: { host, port, key } }
///     { type: "answer", from: "<identity>", data: { host, port, key } }
///     { type: "relay", from: "<identity>", data: <encrypted-bytes> }

export interface Env {
  SIGNAL_ROOM: DurableObjectNamespace;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);

    // WebSocket signaling
    if (url.pathname.startsWith("/signal/")) {
      const room = url.pathname.slice("/signal/".length);
      if (!room) {
        return new Response("Missing room name", { status: 400 });
      }

      // Get or create Durable Object for this room
      const id = env.SIGNAL_ROOM.idFromName(room);
      const stub = env.SIGNAL_ROOM.get(id);

      return stub.fetch(request);
    }

    // Health check
    if (url.pathname === "/health") {
      return Response.json({ ok: true, service: "folk-around-signal" });
    }

    // Room list (public, no auth)
    if (url.pathname === "/rooms") {
      return Response.json({ rooms: [] }); // DOs don't support listing
    }

    return new Response("Not found", { status: 404 });
  },
};

/// Durable Object that manages one signaling room.
/// All WebSocket connections to this room route through this DO.
export class SignalRoom implements DurableObject {
  private peers = new Map<string, WebSocket>();

  async fetch(request: Request): Promise<Response> {
    const pair = new WebSocketPair();
    const [client, server] = [pair[0], pair[1]];

    server.accept();

    server.addEventListener("message", (event: MessageEvent) => {
      try {
        const msg = JSON.parse(event.data as string);
        this.handleMessage(server, msg);
      } catch (e) {
        server.send(
          JSON.stringify({ type: "error", message: "Invalid message" }),
        );
      }
    });

    server.addEventListener("close", () => {
      // Find and remove this peer
      for (const [identity, ws] of this.peers) {
        if (ws === server) {
          this.peers.delete(identity);
          this.broadcast({ type: "peer_left", identity }, server);
          break;
        }
      }
    });

    return new Response(null, { status: 101, webSocket: client });
  }

  private handleMessage(server: WebSocket, msg: any) {
    switch (msg.type) {
      case "join": {
        const identity = msg.identity as string;
        if (!identity) {
          server.send(
            JSON.stringify({ type: "error", message: "Missing identity" }),
          );
          return;
        }

        // Store peer
        this.peers.set(identity, server);

        // Confirm join with current peer list
        const peerList = Array.from(this.peers.keys()).filter(
          (id) => id !== identity,
        );
        server.send(
          JSON.stringify({ type: "joined", room: "", peers: peerList }),
        );

        // Notify other peers
        this.broadcast({ type: "peer_joined", identity }, server);
        break;
      }

      case "offer":
      case "answer": {
        const targetPeer = this.peers.get(msg.to);
        if (targetPeer) {
          targetPeer.send(
            JSON.stringify({
              type: msg.type,
              from: msg.from,
              data: msg.data,
            }),
          );
        }
        break;
      }

      case "relay": {
        // Relay encrypted data to a specific peer (NAT fallback)
        const targetPeer = this.peers.get(msg.to);
        if (targetPeer) {
          targetPeer.send(
            JSON.stringify({
              type: "relay",
              from: msg.from,
              data: msg.data,
            }),
          );
        }
        break;
      }

      default:
        server.send(
          JSON.stringify({
            type: "error",
            message: `Unknown type: ${msg.type}`,
          }),
        );
    }
  }

  private broadcast(msg: any, exclude?: WebSocket) {
    for (const [, ws] of this.peers) {
      if (ws !== exclude) {
        ws.send(JSON.stringify(msg));
      }
    }
  }
}
