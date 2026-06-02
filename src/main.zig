const std = @import("std");
const builtin = @import("builtin");

const mcp = @import("mcp.zig");
const app_config = @import("config.zig");
const http_transport = @import("http.zig");
const p2p = @import("p2p.zig");
const shell = @import("shell.zig");
const tools = @import("tools.zig");

var global_tool_table: ?*tools.ToolTable = null;
var global_verbose = false;

fn relayHandler(alloc: std.mem.Allocator, msg_json: []const u8) !?[]u8 {
    const table = global_tool_table orelse return null;
    var arena = std.heap.ArenaAllocator.init(alloc);
    defer arena.deinit();
    const message_allocator = arena.allocator();

    const msg = std.json.parseFromSliceLeaky(std.json.Value, message_allocator, msg_json, .{}) catch |err| {
        if (global_verbose) std.debug.print("[folk] relay json err: {s}\n", .{@errorName(err)});
        return null;
    };
    const out = try mcp.handleMessage(message_allocator, global_verbose, table, msg) orelse return null;
    return try alloc.dupe(u8, out);
}

pub fn main(init: std.process.Init.Minimal) !void {
    const allocator = std.heap.smp_allocator;
    const args = init.args.vector;

    var verbose = false;
    var mode_name: ?[]const u8 = null;
    var http_port: ?u16 = null;
    var signal_url: ?[]const u8 = null;
    var room: ?[]const u8 = null;
    var p2p_requested = false;

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
            p2p_requested = true;
        } else if (std.mem.eql(u8, arg, "--help") or std.mem.eql(u8, arg, "-h")) {
            return printHelp();
        }
    }

    var saved_config = try app_config.load(allocator, init.environ);
    defer saved_config.deinit(allocator);

    if (p2p_requested and signal_url == null) {
        signal_url = saved_config.signal_url orelse "https://folkaround.undivisible.dev";
    }
    if (http_port == null) http_port = saved_config.http_port;
    if (mode_name == null) mode_name = saved_config.mode;

    const mode = tools.AccessMode.fromName(mode_name orelse "full") orelse {
        std.debug.print("invalid mode. use: full, limited, sandbox\n", .{});
        std.process.exit(1);
    };

    var tool_table = tools.ToolTable.init(allocator, mode);
    defer tool_table.deinit();
    global_tool_table = &tool_table;
    global_verbose = verbose;

    if (signal_url) |url| {
        const room_value = room orelse saved_config.room orelse try app_config.generatePairingCode(allocator);
        const owns_room = room == null and saved_config.room == null;
        defer if (owns_room) allocator.free(room_value);

        const port: u16 = @intCast(http_port orelse 8080);
        try app_config.save(allocator, init.environ, .{
            .signal_url = url,
            .room = room_value,
            .http_port = port,
            .mode = mode_name orelse "full",
        });

        if (verbose) std.debug.print("[folk] P2P mode, signaling: {s}\n", .{url});
        var pm = try p2p.P2PManager.init(allocator, .{
            .enabled = true,
            .signal_url = url,
            .room = room_value,
        }, verbose);
        pm.mcp_handler = relayHandler;
        try pm.start();
        defer pm.stop();
        printPairingInstructions(url, room_value, port);
        try http_transport.run(allocator, verbose, &tool_table, port);
    } else if (http_port) |port| {
        try app_config.save(allocator, init.environ, .{
            .signal_url = saved_config.signal_url,
            .room = saved_config.room,
            .http_port = port,
            .mode = mode_name orelse "full",
        });
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

fn printPairingInstructions(signal_url: []const u8, room: []const u8, port: u16) void {
    std.debug.print("[folk] pairing code: {s}\n", .{room});
    std.debug.print("[folk] give this code to the client and use signaling server: {s}\n", .{signal_url});
    std.debug.print("[folk] local MCP endpoint: http://127.0.0.1:{d}/sse\n", .{port});
    std.debug.print("[folk] waiting for peer...\n", .{});
}

fn printHelp() !void {
    std.debug.print(
        \\folk-around - MCP computer use daemon
        \\Usage: folk-around [options]
        \\
        \\  --verbose           Show tool calls
        \\  --mode <mode>       full, limited, sandbox (default: full)
        \\  --http <port>       HTTP SSE transport (e.g. --http 8080)
        \\  --p2p               Join saved/default signaling server and expose local HTTP
        \\  --signal-server <url>  Custom signaling server URL
        \\  --room <name>       Pairing code / room name
        \\  --help              This help
        \\
        \\Transports:
        \\  stdio     default, pipe to any MCP client
        \\  --http    HTTP SSE for remote over Tailscale/SSH
        \\  --p2p     Prints a pairing code, registers with signaling, and starts local MCP
        \\
    , .{});
}
