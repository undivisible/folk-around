/// folk-around P2P signaling server
/// Cloudflare Worker + Durable Object for WebSocket-based peer discovery.
///
/// Architecture:
///   Client connects via WebSocket to /signal/:room
///   Durable Object per room tracks connected peers
///   Peers exchange X25519 identities through the room
///   MCP relay payloads are end-to-end encrypted by the peers and forwarded through the signaling server
///
/// Protocol (JSON messages over WebSocket):
///   Client -> Server:
///     { type: "join", identity: "<x25519-pubkey-hex>" }
///     { type: "offer", from: "<peer-identity>", to: "<peer-identity>", data: { type: "mcp_relay" } }
///     { type: "answer", from: "<peer-identity>", to: "<peer-identity>", data: { type: "mcp_relay", accepted: true } }
///     { type: "relay", to: "<peer-identity>", data: <encrypted-bytes> }
///
///   Server -> Client:
///     { type: "joined", room: "<room>", peers: ["<identity>", ...] }
///     { type: "peer_joined", identity: "<x25519-pubkey-hex>" }
///     { type: "peer_left", identity: "<x25519-pubkey-hex>" }
///     { type: "offer", from: "<identity>", data: { type: "mcp_relay" } }
///     { type: "answer", from: "<identity>", data: { type: "mcp_relay", accepted: true } }
///     { type: "relay", from: "<identity>", data: <encrypted-bytes> }

export interface Env {
  SIGNAL_ROOM: DurableObjectNamespace;
  ASSETS?: Fetcher;
}

const identityPattern = /^[0-9a-f]{64}$/i;
const maxMessageBytes = 256 * 1024;
const identityGracePeriodMs = 5_000;
const relayRateLimitWindowMs = 1_000;
const maxRelayPerWindow = 60;

function isIdentity(value: unknown): value is string {
  return typeof value === "string" && identityPattern.test(value);
}

function messageSize(value: unknown): number {
  return new TextEncoder().encode(JSON.stringify(value)).length;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);

    if (url.pathname === "/") {
      return env.ASSETS?.fetch(request) ?? fallbackHomePage();
    }

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

    return (
      env.ASSETS?.fetch(request) ?? new Response("Not found", { status: 404 })
    );
  },
};

function fallbackHomePage(): Response {
  return new Response(
    `<!doctype html><title>Folk Around Signalling</title><meta name="viewport" content="width=device-width, initial-scale=1"><body><main><p>online</p><h1>Folk Around Signalling</h1><p>Hosted signalling for Folk Around peer discovery.</p><p><a href="/health">Health</a></p></main></body>`,
    {
      headers: { "content-type": "text/html; charset=utf-8" },
    },
  );
}

/// Durable Object that manages one signaling room.
/// All WebSocket connections to this room route through this DO.
export class SignalRoom implements DurableObject {
  private peers = new Map<string, { ws: WebSocket; joinedAt: number }>();
  private sockets = new Map<WebSocket, string>();
  private rateLimit = new Map<string, { count: number; resetAt: number }>();

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
      const identity = this.sockets.get(server);
      if (identity) {
        const entry = this.peers.get(identity);
        if (entry && entry.ws === server) {
          this.peers.delete(identity);
          this.broadcast({ type: "peer_left", identity }, server);
        }
      }
      this.sockets.delete(server);
      this.rateLimit.delete(server as unknown as string); // clean rate limit entry
    });

    return new Response(null, { status: 101, webSocket: client });
  }

  private handleMessage(server: WebSocket, msg: any) {
    switch (msg.type) {
      case "join": {
        const identity = msg.identity as string;
        if (!isIdentity(identity)) {
          server.send(
            JSON.stringify({ type: "error", message: "Invalid identity" }),
          );
          return;
        }

        const existingEntry = this.peers.get(identity);
        if (existingEntry && existingEntry.ws !== server) {
          // Grace period: if the existing connection is recent, reject the new one
          if (Date.now() - existingEntry.joinedAt < identityGracePeriodMs) {
            server.send(
              JSON.stringify({
                type: "error",
                message: "Identity recently joined, try later",
              }),
            );
            server.close(1008, "Identity recently joined");
            return;
          }
          existingEntry.ws.close(1000, "Replaced by newer connection");
          this.sockets.delete(existingEntry.ws);
        }

        this.peers.set(identity, { ws: server, joinedAt: Date.now() });
        this.sockets.set(server, identity);

        const peerList = Array.from(this.peers.keys()).filter(
          (id) => id !== identity,
        );
        server.send(
          JSON.stringify({ type: "joined", room: "", peers: peerList }),
        );

        this.broadcast({ type: "peer_joined", identity }, server);
        break;
      }

      case "offer":
      case "answer": {
        if (!this.isSender(server, msg.from)) return;
        if (!isIdentity(msg.from) || !isIdentity(msg.to)) {
          server.send(
            JSON.stringify({ type: "error", message: "Invalid peer identity" }),
          );
          return;
        }
        if (messageSize(msg.data) > maxMessageBytes) {
          server.send(
            JSON.stringify({ type: "error", message: "Message too large" }),
          );
          return;
        }
        const targetEntry = this.peers.get(msg.to);
        if (targetEntry) {
          targetEntry.ws.send(
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
        if (!this.isSender(server, msg.from)) return;
        if (!this.checkRateLimit(server)) return;
        if (!isIdentity(msg.from) || !isIdentity(msg.to)) {
          server.send(
            JSON.stringify({ type: "error", message: "Invalid peer identity" }),
          );
          return;
        }
        if (messageSize(msg.data) > maxMessageBytes) {
          server.send(
            JSON.stringify({ type: "error", message: "Message too large" }),
          );
          return;
        }
        const targetEntry = this.peers.get(msg.to);
        if (targetEntry) {
          targetEntry.ws.send(
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

  private isSender(server: WebSocket, identity: unknown): boolean {
    const joinedIdentity = this.sockets.get(server);
    if (!joinedIdentity) {
      server.send(JSON.stringify({ type: "error", message: "Join required" }));
      return false;
    }
    if (identity !== joinedIdentity) {
      server.send(
        JSON.stringify({ type: "error", message: "Sender identity mismatch" }),
      );
      return false;
    }
    return true;
  }

  private checkRateLimit(server: WebSocket): boolean {
    const key = this.sockets.get(server) || (server as unknown as string);
    const now = Date.now();
    let entry = this.rateLimit.get(key);
    if (!entry || now >= entry.resetAt) {
      entry = { count: 0, resetAt: now + relayRateLimitWindowMs };
      this.rateLimit.set(key, entry);
    }
    entry.count++;
    if (entry.count > maxRelayPerWindow) {
      server.send(
        JSON.stringify({ type: "error", message: "Rate limit exceeded" }),
      );
      return false;
    }
    return true;
  }

  private broadcast(msg: any, exclude?: WebSocket) {
    for (const [, entry] of this.peers) {
      if (entry.ws !== exclude) {
        entry.ws.send(JSON.stringify(msg));
      }
    }
  }
}
