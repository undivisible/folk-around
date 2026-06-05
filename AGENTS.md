# folk-around

## identity

rust mcp agent for computer control. speaks mcp over stdio, http sse, or Cloudflare signaling plus local http.
self-contained release binary. Rust source uses crates.io dependencies, including rs_peekaboo for computer-use.

## build

```
requires rust stable for the main runtime when Cargo.toml is present
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
```

## source layout

```
Rust main runtime
├── CLI daemon
├── MCP tool surface
├── stdio and HTTP SSE transports
└── Cloudflare signaling relay client
crates/
├── folk-around        Rust binary
├── folk-core          config and access modes
├── folk-mcp           JSON-RPC MCP handling
├── folk-transport     stdio, HTTP SSE, and signaling relay
└── folk-computer-use  shell, clipboard, and rs_peekaboo-backed computer-use tools
src/
├── main.zig        archived legacy entry, cli args (--mode, --http, --p2p, --signal-server, --room)
├── mcp.zig         legacy stdio mcp transport (init, tools/list, tools/call, ping, notifications)
├── http.zig        legacy http sse transport (GET /sse, POST /message, GET /health)
├── p2p.zig         legacy Cloudflare Workers signaling client, WebSocket join, frame helpers
├── shell.zig       legacy shell execution engine (fork/exec, pipe)
├── tools.zig       legacy tool table, access mode gating, safe cmd list
├── legacy_bridge.zig temporary C ABI bridge for legacy tool calls
└── mac_app.zig     macOS menu bar app (AppKit: NSStatusBar, NSMenu, cached Objective-C runtime calls)
signal-server/
├── src/index.ts    Cloudflare Worker + Durable Object for WebSocket signaling
├── wrangler.toml
├── package.json
└── tsconfig.json
scripts/
├── install.sh      one-liner installer (detects os/arch)
└── folk-around.1   man page
```

## tools

folk_shell, folk_system_info, folk_list_apps, folk_spawn,
folk_clipboard_read, folk_clipboard_write, folk_screen_capture,
folk_ui_snapshot, folk_click, folk_type, folk_hotkey, folk_scroll,
folk_window, folk_app, folk_menu

## security modes

--mode full: unrestricted shell, full access
--mode limited: safe-shell cmds (ls, cat, grep, find, head, tail, wc, curl, echo, date, whoami, hostname, uname, which, pwd, ps, uptime, df, du)
--mode sandbox: current compatibility behavior uses the same safe-shell cmds as limited; folk_spawn remains full-only

## transports

- no args: reuse the saved HTTP port, or stdio if none is saved.
- stdio: --stdio forces standard mcp over stdin/stdout with content-length framing
- http sse: --http <port>. GET /sse (events), POST /message (calls), GET /health
- p2p: --p2p. websocket -> cf signaling server, then local HTTP MCP on 8080 by default.
  --signal-server <url> for custom server, --room <name> for room selection.

## signaling server (Cloudflare)

signal-server/ is a standalone TypeScript project. deploy:
  cd signal-server && bun install && bunx wrangler deploy

The worker creates Durable Objects per room. WebSocket-based signaling:
- join/leave broadcast
- offer/answer relay for connection metadata
- relay encrypted MCP messages after offer/answer handshake

## development notes

- Zig files under src/ and build.zig are archived legacy source only. They are not active CI, release, install, or Cargo workspace paths.
- p2p signaling is wired; encrypted MCP relay payloads use X25519 shared-secret-derived keys over the signaling relay.
- signal-server fully functional: deploy for signaling; use --http for the local MCP endpoint.
- macos target primary through rs_peekaboo-backed Rust computer-use tools.
