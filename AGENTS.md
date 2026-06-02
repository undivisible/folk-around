# folk-around

## identity

zig mcp agent for computer control. speaks mcp over stdio, http sse, or p2p encrypted tunnel.
self-contained binary, no external dependencies. native macos menu bar app in zig (appkit via @cimport).

## build

```
requires zig 0.14.0
zig build                 # daemon only (zig-out/bin/folk-around)
zig build -Dapp           # mac app only (zig-out/bin/FolkAround)
zig build all             # both
zig build -Doptimize=ReleaseFast  # release build
```

## source layout

```
src/
├── main.zig        entry, cli args (--mode, --http, --p2p, --signal-server, --room)
├── mcp.zig         stdio mcp transport (init, tools/list, tools/call, ping, notifications)
├── http.zig        http sse transport (GET /sse, POST /message, GET /health)
├── p2p.zig         p2p: noise_xk handshake + xchacha20-poly1305 frames,
│                   Cloudflare Workers signaling client
├── shell.zig       shell execution engine (fork/exec, pipe)
├── tools.zig       tool table (9 tools), access mode gating, safe cmd list
└── mac_app.zig     macOS menu bar app (AppKit: NSStatusBar, NSMenu, NSImageView)
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
folk_clipboard_read, folk_clipboard_write, folk_osascript,
folk_tell, folk_screenshot

## security modes

--mode full: unrestricted shell, full access
--mode limited: read-only cmds (ls, cat, grep, find, head, tail, wc, curl, echo, date, whoami, hostname, uname, which, pwd, ps, uptime, df, du)
--mode sandbox: no shell, a11y + file ops only

## transports

- stdio: default. standard mcp over stdin/stdout with content-length framing
- http sse: --http <port>. GET /sse (events), POST /message (calls), GET /health
- p2p: --p2p. websocket -> cf signaling server -> noise_xk handshake -> encrypted frames.
  --signal-server <url> for custom server, --room <name> for room selection.

## signaling server (Cloudflare)

signal-server/ is a standalone TypeScript project. deploy:
  cd signal-server && npm install && npx wrangler deploy

The worker creates Durable Objects per room. WebSocket-based signaling:
- join/leave broadcast
- offer/answer relay for connection metadata
- relay fallback for NAT-traversed encrypted data

## development notes

- zig 0.14 api only. 0.15+ has breaking std changes.
- no package manager. all deps inline.
- macos target primary (osascript, screencapture, pbcopy/pbpaste, appkit).
- mac_app.zig uses @cImport with Cocoa/ApplicationServices frameworks.
- p2p.zig stub: WebSocket client + Noise handshake not yet wired (needs Zig std.crypto).
- signal-server fully functional: deploy and use --p2p or --signal-server.
- linux fallback possible via xdotool/etc (no menu bar app).