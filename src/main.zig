const std = @import("std");
const builtin = @import("builtin");

const mcp = @import("mcp.zig");
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

    var i: usize = 1;
    while (i < args.len) : (i += 1) {
        if (std.mem.eql(u8, args[i], "--verbose") or std.mem.eql(u8, args[i], "-v")) verbose = true
        else if (std.mem.eql(u8, args[i], "--mode")) { i += 1; if (i < args.len) mode_name = args[i]; }
        else if (std.mem.eql(u8, args[i], "--help") or std.mem.eql(u8, args[i], "-h")) {
            return printHelp();
        }
    }

    const mode = tools.AccessMode.fromName(mode_name orelse "full") orelse {
        std.debug.print("invalid mode. use: full, limited, sandbox\n", .{});
        std.process.exit(1);
    };

    if (verbose) {
        std.debug.print("[folk] starting (mode={s})\n", .{@tagName(mode)});
    }

    var tool_table = tools.ToolTable.init(allocator, mode);
    defer tool_table.deinit();

    try mcp.run(allocator, verbose, &tool_table);
}

fn printHelp() !void {
    const stdout = std.io.getStdOut().writer();
    try stdout.print(
        \\folk-around - MCP computer use daemon
        \\Usage: folk-around [options]
        \\  --verbose      Show tool calls
        \\  --mode <mode>  full, limited, sandbox
        \\  --help         This help
        \\
    , .{});
}