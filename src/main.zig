const std = @import("std");
const builtin = @import("builtin");

const mcp = @import("mcp.zig");
const http_transport = @import("http.zig");
const p2p = @import("p2p.zig");
const shell = @import("shell.zig");
const tools = @import("tools.zig");

pub fn main(init: std.process.Init.Minimal) !void {
    const allocator = std.heap.smp_allocator;
    const args = init.args.vector;

    var verbose = false;
    var mode_name: ?[]const u8 = null;
    var http_port: ?u16 = null;
    var signal_url: ?[]const u8 = null;
    var room: ?[]const u8 = null;

    var i: usize = 1;
    while (i < args.len) : (i += 1) {
        const arg = std.mem.span(args[i]);

        if (std.mem.eql(u8, arg, "--verbose") or std.mem.eql(u8, arg, "-v")) {
            verbose = true;
        } else if (std.mem.eql(u8, arg, "--mode")) {
            i += 1;
            if (i < args.len) mode_name = std.mem.span(args[i]);
        } else if (std.mem.eql(u8, arg, "--http")) {
            i += 1;
            if (i < args.len) http_port = std.fmt.parseUnsigned(u16, std.mem.span(args[i]), 10) catch null;
        } else if (std.mem.eql(u8, arg, "--signal-server")) {
            i += 1;
            if (i < args.len) signal_url = std.mem.span(args[i]);
        } else if (std.mem.eql(u8, arg, "--room")) {
            i += 1;
            if (i < args.len) room = std.mem.span(args[i]);
        } else if (std.mem.eql(u8, arg, "--p2p")) {
            signal_url = signal_url orelse "https://folk-around-signal.undivisible.workers.dev";
        } else if (std.mem.eql(u8, arg, "--help") or std.mem.eql(u8, arg, "-h")) {
            return printHelp();
        }
    }

    const mode = tools.AccessMode.fromName(mode_name orelse "full") orelse {
        std.debug.print("invalid mode. use: full, limited, sandbox\n", .{});
        std.process.exit(1);
    };

    var tool_table = tools.ToolTable.init(allocator, mode);
    defer tool_table.deinit();

    if (signal_url) |url| {
        if (verbose) std.debug.print("[folk] P2P mode, signaling: {s}\n", .{url});
        var pm = try p2p.P2PManager.init(allocator, .{
            .enabled = true,
            .signal_url = url,
            .room = room orelse "default",
        }, verbose);
        try pm.start();
        defer pm.stop();
        // Also start HTTP for local MCP client access
        const port: u16 = @intCast(http_port orelse 8080);
        if (!verbose) {
            std.debug.print("[folk] signaling: {s} room={s}\n", .{ url, room orelse "default" });
            std.debug.print("[folk] HTTP listening on http://127.0.0.1:{d}/\n", .{port});
        }
        try http_transport.run(allocator, verbose, &tool_table, port);
    } else if (http_port) |port| {
        if (verbose) {
            std.debug.print("[folk] HTTP SSE mode on port {d}\n", .{port});
        } else {
            std.debug.print("[folk] HTTP listening on http://127.0.0.1:{d}/\n", .{port});
        }
        try http_transport.run(allocator, verbose, &tool_table, port);
    } else {
        if (verbose) std.debug.print("[folk] stdio mode (mode={s})\n", .{@tagName(mode)});
        try mcp.run(allocator, verbose, &tool_table);
    }
}

fn printHelp() !void {
    std.debug.print(
        \\folk-around - MCP computer use daemon
        \\Usage: folk-around [options]
        \\
        \\  --verbose           Show tool calls
        \\  --mode <mode>       full, limited, sandbox (default: full)
        \\  --http <port>       HTTP SSE transport (e.g. --http 8080)
        \\  --p2p               Join CF signaling server and expose local HTTP
        \\  --signal-server <url>  Custom signaling server URL
        \\  --room <name>       P2P room name (default: "default")
        \\  --help              This help
        \\
        \\Transports:
        \\  stdio     default, pipe to any MCP client
        \\  --http    HTTP SSE for remote over Tailscale/SSH
        \\  --p2p     Signaling server registration plus local HTTP MCP
        \\
    , .{});
}
