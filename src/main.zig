const std = @import("std");
const builtin = @import("builtin");

const mcp = @import("mcp.zig");
const http_transport = @import("http.zig");
const p2p = @import("p2p.zig");
const shell = @import("shell.zig");
const tools = @import("tools.zig");

pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    const args = try std.process.argsAlloc(allocator);
    defer std.process.argsFree(allocator, args);

    var verbose = false;
    var mode_name: ?[]const u8 = null;
    var http_port: ?u16 = null;
    var p2p_enabled = false;

    var i: usize = 1;
    while (i < args.len) : (i += 1) {
        if (std.mem.eql(u8, args[i], "--verbose") or std.mem.eql(u8, args[i], "-v")) verbose = true
        else if (std.mem.eql(u8, args[i], "--mode")) { i += 1; if (i < args.len) mode_name = args[i]; }
        else if (std.mem.eql(u8, args[i], "--http")) { i += 1; if (i < args.len) http_port = std.fmt.parseUnsigned(u16, args[i], 10) catch null; }
        else if (std.mem.eql(u8, args[i], "--p2p")) p2p_enabled = true;
        else if (std.mem.eql(u8, args[i], "--help") or std.mem.eql(u8, args[i], "-h")) {
            return printHelp();
        }
    }

    const mode = tools.AccessMode.fromName(mode_name orelse "full") orelse {
        std.debug.print("invalid mode. use: full, limited, sandbox\n", .{});
        std.process.exit(1);
    };

    var tool_table = tools.ToolTable.init(allocator, mode);
    defer tool_table.deinit();

    if (p2p_enabled) {
        if (verbose) std.debug.print("[folk] P2P mode\n", .{});
        var pm = p2p.P2PManager.init(allocator, .{ .enabled = true });
        try pm.start();
        defer pm.stop();
        // Also start HTTP for signaling
        try http_transport.run(allocator, verbose, &tool_table, 8080);
    } else if (http_port) |port| {
        if (verbose) std.debug.print("[folk] HTTP SSE mode on port {d}\n", .{port});
        try http_transport.run(allocator, verbose, &tool_table, port);
    } else {
        if (verbose) std.debug.print("[folk] stdio mode (mode={s})\n", .{@tagName(mode)});
        try mcp.run(allocator, verbose, &tool_table);
    }
}

fn printHelp() !void {
    const stdout = std.io.getStdOut().writer();
    try stdout.print(
        \\folk-around - MCP computer use daemon
        \\Usage: folk-around [options]
        \\  --verbose      Show tool calls
        \\  --mode <mode>  full, limited, sandbox
        \\  --http <port>  HTTP SSE transport (e.g. --http 8080)
        \\  --p2p          P2P mode (experimental, uses port 8080 for signaling)
        \\  --help         This help
        \\
    , .{});
}