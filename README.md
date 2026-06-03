# folk-around

https://folkaround.undivisible.dev

Zig MCP agent for computer control. Shell, accessibility, clipboard, files, osascript — over stdio, HTTP SSE, or Cloudflare signaling plus local HTTP. Native macOS menu bar app, also in Zig.

## what it is

Self-contained binary that speaks the [Model Context Protocol](https://modelcontextprotocol.io). Any MCP client (Claude Desktop, Cursor, any agent) connects and gets 9 tools for controlling a computer.

```bash
folk-around                              # reuse saved transport, or stdio if none is saved
folk-around --stdio                      # force stdio, full access
folk-around --http 8080                  # HTTP SSE, remote via SSH/Tailscale
folk-around --mode sandbox               # restricted mode
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
| folk_osascript | AppleScript execution |
| folk_tell | tell an app (macOS) |
| folk_screenshot | capture screen |

## transports

### stdio
Standard MCP over stdin/stdout. Pipe to any MCP client. Use `folk-around --stdio` to force stdio when a previous HTTP or signalling transport is saved.

### saved transport
Running `folk-around` with no transport flags reuses the last saved transport settings under `~/.config/folk-around/config`. If no transport is saved, it falls back to stdio.

### HTTP SSE
`folk-around --http 8080` — runs an HTTP server with SSE. Clients connect at `http://localhost:8080/sse`. Tunnel via `ssh -L` or Tailscale for remote access.

### P2P
`folk-around --p2p` or `folk-around --signal-server <host>` prints a pairing code, joins that signaling room over WebSocket, announces this daemon identity, and starts local HTTP MCP on port 8080 by default. Give the pairing code and signaling server to the client. Peers complete an offer/answer handshake, derive a shared X25519 session key, and encrypt relayed MCP payloads with XChaCha20-Poly1305 before command execution.

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

The tools exposed through MCP always act on the computer running `folk-around`. Their tool descriptions say this explicitly so a remote agent understands that shell commands, clipboard access, AppleScript, process listing, and screenshots happen on the Folk Around host computer, not on the agent provider's own runtime.

The last signaling server, pairing code, HTTP port, and mode are saved under `~/.config/folk-around/config`. Running `folk-around` with no transport flags reuses saved transport settings. Running `folk-around --p2p` reuses the last saved pairing code unless you pass `--room <code>`.

## macOS menu bar app (Zig, no Xcode)

The menu bar companion is also written in Zig using the Objective-C runtime directly. No Swift, no Xcode project required.

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

# or build from source (zig 0.16.0)
git clone https://github.com/undivisible/folk-around
cd folk-around
zig build -Doptimize=ReleaseFast
sudo cp zig-out/bin/folk-around /usr/local/bin/
```

## security modes

| mode | shell | files | clipboard | a11y |
|------|-------|-------|-----------|------|
| full | unrestricted | read+write | all | all |
| limited | read-only cmds | read | all | all |
| sandbox | blocked | read+write | all | all |

Safe cmds (limited mode): ls, cat, grep, find, head, tail, wc, curl, echo, date, whoami, hostname, uname, which, pwd, ps, uptime, df, du

## source layout

```
src/
├── main.zig        entry, cli args (--mode, --http, --p2p, --signal-server, --room)
├── mcp.zig         stdio MCP transport
├── http.zig        HTTP SSE transport
├── p2p.zig         P2P wire protocol + CF signaling client
├── shell.zig       shell execution engine
├── tools.zig       tool table (9 tools), access mode gating
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
