# folk-around

Zig MCP agent for computer control. Shell, accessibility, clipboard, files, osascript — over stdio, HTTP SSE, or P2P encrypted tunnel. Native macOS menu bar app, also in Zig.

## what it is

Self-contained binary that speaks the [Model Context Protocol](https://modelcontextprotocol.io). Any MCP client (Claude Desktop, Cursor, any agent) connects and gets 9 tools for controlling a computer.

```bash
folk-around                              # stdio, full access
folk-around --http 8080                  # HTTP SSE, remote via SSH/Tailscale
folk-around --mode sandbox               # restricted mode
folk-around --p2p                        # P2P encrypted tunnel
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
Standard MCP over stdin/stdout. Pipe to any MCP client.

### HTTP SSE
`folk-around --http 8080` — runs an HTTP server with SSE. Clients connect at `http://localhost:8080/sse`. Tunnel via `ssh -L` or Tailscale for remote access.

### P2P
`folk-around --p2p` currently exits with an unavailable transport message. The Cloudflare Workers signaling server can still be deployed for future tunnel work:

```bash
cd signal-server
bun install
bunx wrangler deploy
```

Point folk-around at HTTP mode for remote use today:
```bash
folk-around --http 8080
```

## macOS menu bar app (Zig, no Xcode)

The menu bar companion is also written in Zig using the Objective-C runtime directly. No Swift, no Xcode project required.

```bash
# build it
zig build -Dapp   # outputs FolkAround in zig-out/bin/

# or build both
zig build all
```

Shows green/red status dot, start/stop daemon, mode selector, port display, logs.

## install

```bash
# one-liner
curl -fsSL https://raw.githubusercontent.com/undivisible/folk-around/main/scripts/install.sh | bash

# or build from source (zig 0.14.0)
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
└── mac_app.zig     macOS menu bar app (AppKit via @cImport)
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
