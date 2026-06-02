# folk-around

## identity

zig mcp agent for computer control. speaks mcp over stdio or http sse.
self-contained binary, no external dependencies.

## build

```
requires zig 0.14.0
zig build -Doptimize=ReleaseFast
binary at zig-out/bin/folk-around
```

## source layout

```
src/
├── main.zig       entry, cli args (--mode, --http, --verbose)
├── mcp.zig        stdio mcp transport (init, tools/list, tools/call, ping)
├── http.zig       http sse transport (GET /sse, POST /message)
├── shell.zig      shell execution engine
└── tools.zig      tool table (9 tools), access mode gating, safe cmds
scripts/
├── install.sh     one-liner installer
└── folk-around.1  man page
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

## development notes

- zig 0.14 api only. 0.15+ has breaking std changes.
- no package manager. all deps inline.
- macos target primary (osascript, screencapture, pbcopy/pbpaste).
- linux fallback possible via xdotool/etc.

## roadmap

- webrtc p2p with e2e encryption
- swiftui menu bar companion
- homebrew tap