# folk-around

Zig MCP agent for computer control. Shell execution, accessibility automation, vision, clipboard, file ops, and osascript — all over standard MCP stdio transport.

## what it is

folk-around is a self-contained binary that speaks the [Model Context Protocol](https://modelcontextprotocol.io) over stdio. Any MCP client (Claude, Cursor, any agent framework) can connect to it and get 9 tools for controlling a computer:

| tool | does |
|------|------|
| shell | run commands (mode-restricted) |
| a11y | query macOS accessibility tree |
| osascript | run AppleScript |
| screenshot | capture screen regions |
| clipboard | read/write system clipboard |
| system_info | hardware, OS, uptime |
| file_read | read files |
| file_write | write files |
| script | run multi-step scripts |

## quick start

```bash
# download
curl -L -o /usr/local/bin/folk-around https://github.com/undivisible/folk-around/releases/latest/download/folk-around
chmod +x /usr/local/bin/folk-around

# run (stdio MCP mode)
folk-around
```

connect any MCP client to the stdio transport and you're set.

## one-liner

```bash
curl -fsSL https://raw.githubusercontent.com/undivisible/folk-around/main/scripts/install.sh | bash
```

## building from source

requires zig 0.14.0

```bash
zig build -Doptimize=ReleaseFast
sudo cp zig-out/bin/folk-around /usr/local/bin/
```

## security

folk-around uses a `--mode` flag:

- `full` — unrestricted shell access, same trust model as giving someone SSH
- `limited` — safe commands only (read-only, no exec)
- `sandbox` — no shell, a11y + file ops only

## future

- HTTP SSE transport for remote connections (Tailscale tunnel)
- WebRTC P2P with e2e encryption
- macOS menu bar companion (SwiftUI)
- Homebrew formula
- `brew install folk-around`

## license

MIT