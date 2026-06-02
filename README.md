# folk-around

Zig MCP agent for computer control. Shell execution, accessibility automation, vision, clipboard, file ops, and osascript — over stdio, HTTP SSE, or P2P encrypted tunnel.

## what it is

folk-around is a self-contained binary that speaks the [Model Context Protocol](https://modelcontextprotocol.io). Any MCP client (Claude Desktop, Cursor, any agent framework) can connect and get tools for controlling a computer.

## tools

| tool | does |
|------|------|
| folk_shell | run commands (mode-restricted) |
| folk_system_info | hardware, OS, arch |
| folk_list_apps | running processes |
| folk_spawn | spawn background process (full mode) |
| folk_clipboard_read | read system clipboard |
| folk_clipboard_write | write to clipboard |
| folk_osascript | execute AppleScript |
| folk_tell | tell an app (AppleScript) |
| folk_screenshot | capture screen |

## quick start

```bash
# one-liner
curl -fsSL https://raw.githubusercontent.com/undivisible/folk-around/main/scripts/install.sh | bash

# or manually
curl -L -o /usr/local/bin/folk-around https://github.com/undivisible/folk-around/releases/latest/download/folk-around
chmod +x /usr/local/bin/folk-around
folk-around
```

## usage

```bash
folk-around                              # stdio mode, full access
folk-around --mode sandbox               # restricted mode
folk-around --http 8080                  # HTTP SSE on port 8080
folk-around --http 8080 --mode limited   # combined
folk-around -v                           # verbose
```

## transports

### stdio (default)
Standard MCP protocol over stdin/stdout. Pipe to any MCP client.

```bash
folk-around | your-mcp-client
```

### HTTP SSE
Runs an HTTP server with Server-Sent Events. Clients connect via SSE for events and POST JSON-RPC messages.

```bash
folk-around --http 8080
```

Then point your MCP client at `http://localhost:8080/sse`.

For remote access, tunnel through Tailscale or SSH:
```bash
ssh user@machine -L 8080:localhost:8080
```

### P2P (experimental)
Direct encrypted tunnel between peers using Noise_XK handshake + XChaCha20-Poly1305. No infrastructure needed — just two folk-around instances with shared identity keys.

```bash
# Peer A (runs signaling)
folk-around --p2p

# Peer B connects to A
folk-around --p2p --peer <A-identity-hex>
```

## menu bar companion (macOS)

A SwiftUI app that lives in your menu bar. Start/stop the daemon, switch modes, see transport status.

```bash
# via Homebrew
brew install --cask folk-around

# or build from source
cd FolkAround && swift build
```

## homebrew

```bash
# tap
brew tap undivisible/tap

# install the CLI
brew install folk-around

# install the menu bar app
brew install --cask folk-around
```

## building from source

requires zig 0.14.0

```bash
git clone https://github.com/undivisible/folk-around
cd folk-around
zig build -Doptimize=ReleaseFast
sudo cp zig-out/bin/folk-around /usr/local/bin/
```

## security modes

| mode | shell | file read | file write | other |
|------|-------|-----------|------------|-------|
| full | unrestricted | yes | yes | all |
| limited | read-only cmds | yes | no | all |
| sandbox | blocked | yes | yes | clipboard, a11y |

Safe command list in limited mode: ls, cat, grep, find, head, tail, wc, curl, echo, date, whoami, hostname, uname, which, pwd, ps, uptime, df, du

## source layout

```
src/
├── main.zig       entry, cli args (--mode, --http, --p2p)
├── mcp.zig        stdio MCP transport
├── http.zig       HTTP SSE transport
├── p2p.zig        P2P signaling + wire protocol (Noise XK)
├── shell.zig      shell execution engine
└── tools.zig      tool table, access mode gating, safe cmds
scripts/
├── install.sh     one-liner installer
└── folk-around.1  man page
Formula/
└── folk-around.rb Homebrew formula
FolkAround.swift    Menu bar companion (SwiftUI)
Package.swift       Swift package manifest
```

## roadmap

- [x] stdio MCP transport
- [x] HTTP SSE transport (remote via Tailscale/SSH)
- [x] P2P wire protocol spec + signaling
- [x] macOS menu bar companion (SwiftUI)
- [x] Homebrew formula
- [ ] GitHub Actions CI + release binaries
- [ ] Linux a11y (xdotool, ydotool)
- [ ] Windows support

## license

MPL-2.0