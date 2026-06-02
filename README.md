# folk-around

Zig MCP agent for computer control. Shell execution, accessibility automation, vision, clipboard, file ops, and osascript — over stdio or HTTP SSE.

## what it is

folk-around is a self-contained binary that speaks the [Model Context Protocol](https://modelcontextprotocol.io). Any MCP client (Claude, Cursor, any agent framework) can connect and get tools for controlling a computer:

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
# stdio mode (default) — pipe to any MCP client
folk-around

# HTTP SSE mode — for remote connections via Tailscale/SSH tunnel
folk-around --http 8080

# security modes
folk-around --mode full      # unrestricted (default)
folk-around --mode limited   # safe commands only
folk-around --mode sandbox   # no shell, a11y+file ops only

# verbose
folk-around --verbose
```

## connecting remotely

Run in HTTP mode on your machine, then tunnel:

```bash
# on the target machine
folk-around --http 8080

# from your client machine (via Tailscale)
ssh user@machine -L 8080:localhost:8080

# connect any MCP client to http://localhost:8080/sse
```

## building from source

requires zig 0.14.0

```bash
git clone https://github.com/undivisible/folk-around
cd folk-around
zig build -Doptimize=ReleaseFast
sudo cp zig-out/bin/folk-around /usr/local/bin/
```

## homebrew

```bash
brew install undivisible/tap/folk-around
```

## security

| mode | shell | file read | file write | other |
|------|-------|-----------|------------|-------|
| full | unrestricted | yes | yes | all |
| limited | read-only cmds | yes | no | all |
| sandbox | blocked | yes | yes | clipboard, a11y |

Safe command list in limited mode: ls, cat, grep, find, head, tail, wc, curl, echo, date, whoami, hostname, uname, which, pwd, ps, uptime, df, du

## transport

- **stdio**: standard MCP protocol over stdin/stdout
- **HTTP SSE**: GET /sse for events, POST /message for calls, GET /health for health check
- **WebRTC P2P**: coming

## license

MPL-2.0