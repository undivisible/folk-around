# folk-around

https://folkaround.undivisible.dev

Rust MCP agent for computer control. Shell, accessibility, clipboard, files, and `rs_peekaboo`-backed macOS automation over stdio, HTTP SSE, or Cloudflare signaling plus local HTTP. The Zig daemon remains as a legacy compatibility module, and the native macOS menu bar app is still Zig/AppKit.

## what it is

Self-contained release binary that speaks the [Model Context Protocol](https://modelcontextprotocol.io). Any MCP client (Claude Desktop, Cursor, any agent) connects and gets structured tools for observing and controlling a computer.

```bash
folk-around                              # reuse saved HTTP port, or stdio if none is saved
folk-around --stdio                      # force stdio, full access
folk-around --http 8080                  # HTTP SSE, remote via SSH/Tailscale
folk-around --mode sandbox               # safe-shell compatibility mode
folk-around --p2p                        # print pairing code, join signaling, serve local HTTP
```

## tools

| tool | does |
|------|------|
| folk_shell | run commands (mode-restricted) |
| folk_system_info | hardware, OS, arch |
| folk_list_apps | running processes |
| folk_spawn | spawn background (full mode) |
| folk_clipboard_read | read clipboard |
| folk_clipboard_write | write to clipboard |
| folk_screen_capture | capture display, window, or region |
| folk_ui_snapshot | inspect app/window context |
| folk_click | click coordinates |
| folk_type | type text |
| folk_hotkey | press key combinations |
| folk_scroll | scroll |
| folk_window | list/focus/window actions |
| folk_app | list/launch/activate/quit apps |
| folk_menu | inspect/click menu items |

## transports

### stdio
Standard MCP over stdin/stdout. Pipe to any MCP client. Use `folk-around --stdio` to force stdio when a previous HTTP or signalling transport is saved.

### saved transport
Running `folk-around` with no transport flags reuses the saved HTTP port under `~/.config/folk-around/config`. If no HTTP port is saved, it falls back to stdio. Use `--p2p` to reuse the saved signaling server and pairing code.

### HTTP SSE
`folk-around --http 8080` — runs an HTTP server with SSE. Clients connect at `http://localhost:8080/sse`. Tunnel via `ssh -L` or Tailscale for remote access.

### P2P
`folk-around --p2p` or `folk-around --signal-server <host>` prints a pairing code, joins that signaling room over WebSocket, announces this daemon identity, and starts local HTTP MCP on port 8080 by default. Give the pairing code and signaling server to the client. Peers complete an offer/answer handshake, exchange X25519 public identities, derive relay session keys from the X25519 shared secret with HKDF, and encrypt relayed MCP payloads with XChaCha20-Poly1305 before command execution.

Hosted signalling server and status page: `https://folkaround.undivisible.dev`

```bash
cd signal-server
bun install
bunx wrangler deploy
```

Point folk-around at a custom signaling host:
```bash
folk-around --signal-server https://your-worker.workers.dev --room my-room
```

Use the hosted Folk Around signalling server:
```bash
folk-around --signal-server https://folkaround.undivisible.dev --room my-room
```

The tools exposed through MCP always act on the computer running `folk-around`. Their tool descriptions say this explicitly so a remote agent understands that shell commands, clipboard access, process listing, screenshots, input events, app/window actions, and menu actions happen on the Folk Around host computer, not on the agent provider's own runtime.

The last signaling server, pairing code, HTTP port, and mode are saved under `~/.config/folk-around/config`. Running `folk-around` with no transport flags reuses the saved HTTP port if one exists. Running `folk-around --p2p` reuses the last saved signaling server and pairing code unless you pass `--signal-server` or `--room <code>`.

## macOS menu bar app and Zig legacy module

The main CLI runtime is Rust after the migration. The Zig tree remains buildable for the legacy daemon module and for the menu bar companion, which uses the Objective-C runtime directly. No Swift, no Xcode project required.

```bash
# build it
zig build -Dapp   # outputs FolkAround in zig-out/bin/

# or build both
zig build all
```

Shows daemon status, start/stop daemon, run app at login, and run daemon at login.

## install

```bash
# one-liner
curl -fsSL https://raw.githubusercontent.com/undivisible/folk-around/main/scripts/install.sh | bash

# or build from source
git clone https://github.com/undivisible/folk-around
cd folk-around
cargo build --release
sudo cp target/release/folk-around /usr/local/bin/
```

Legacy Zig compatibility build:
```bash
zig build -Doptimize=ReleaseFast
sudo cp zig-out/bin/folk-around /usr/local/bin/
```

## security modes

| mode | shell | clipboard | observation | mutation |
|------|-------|-----------|-------------|----------|
| full | unrestricted | all | all | all |
| limited | safe command allowlist | all | all | focus/list allowed |
| sandbox | safe command allowlist | all | all | blocked |

Safe cmds (limited and sandbox modes): ls, cat, grep, find, head, tail, wc, curl, echo, date, whoami, hostname, uname, which, pwd, ps, uptime, df, du

`folk_spawn` remains full-mode only.

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
├── folk-computer-use  shell, clipboard, and rs_peekaboo-backed computer-use tools
└── folk-zig-legacy    temporary Zig C ABI bridge through ../equilibrium
src/
├── main.zig        legacy entry, cli args (--mode, --http, --p2p, --signal-server, --room)
├── mcp.zig         legacy stdio MCP transport
├── http.zig        legacy HTTP SSE transport
├── p2p.zig         legacy relay wire protocol + CF signaling client
├── shell.zig       legacy shell execution engine
├── tools.zig       legacy tool table, access mode gating
├── legacy_bridge.zig temporary C ABI bridge for legacy tool calls
└── mac_app.zig     macOS menu bar app (AppKit via cached Objective-C runtime calls)
signal-server/
├── src/index.ts    Cloudflare Worker + Durable Object (WebSocket signaling)
├── wrangler.toml
├── package.json
└── tsconfig.json
scripts/
├── install.sh      one-liner installer
└── folk-around.1   man page
```

## license

MPL-2.0
