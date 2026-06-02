# folk-around

## identity

zig mcp agent for computer control. speaks mcp over stdio. self-contained binary, no external dependencies.

## build

```
zig 0.14.0 only
zig build -Doptimize=ReleaseFast
binary at zig-out/bin/folk-around
```

## source layout

```
src/
├── main.zig              entry, cli args, server startup
├── config.zig            access modes (full/limited/sandbox), safe command lists
├── mcp/
│   ├── types.zig         json-rpc 2.0 types, mcp protocol messages
│   ├── transport.zig     stdio transport with content-length framing
│   └── server.zig        mcp server (init, tools/list, tools/call, notifications)
├── engines/
│   └── shell.zig         shell execution engine (mode-restricted)
└── tools/
    └── all.zig           tool registrations (9 tools)
```

## protocol

standard mcp over stdio — json-rpc 2.0 with content-length headers. any mcp client can connect.

tools list:
- `shell` — run command (mode-gated)
- `a11y` — query macos accessibility tree
- `osascript` — run applescript
- `screenshot` — capture screen
- `clipboard` — read/write clipboard
- `system_info` — hardware/os/uptime
- `file_read` — read files
- `file_write` — write files
- `script` — multi-step scripts

## security modes

`--mode full`: unrestricted shell access
`--mode limited`: read-only safe commands
`--mode sandbox`: no shell, a11y + file ops only

## development notes

- zig 0.14 api only. 0.15+ has breaking std changes.
- no package manager. all deps inline.
- macos target primary (a11y + osascript).
- linux fallback possible via xdotool/etc.

## roadmap

- http sse transport for remote
- webrtc p2p with e2e encryption
- swiftui menu bar companion
- homebrew formula