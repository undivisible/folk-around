const std = @import("std");
const shell = @import("shell.zig");

const Allocator = std.mem.Allocator;
const Value = std.json.Value;

pub const AccessMode = enum { full, limited, sandbox,
    pub fn fromName(n: []const u8) ?AccessMode {
        if (std.mem.eql(u8, n, "full")) return .full;
        if (std.mem.eql(u8, n, "limited")) return .limited;
        if (std.mem.eql(u8, n, "sandbox")) return .sandbox;
        return null;
    }
};

const safe_cmds = std.StaticStringMap(void).initComptime(.{
    .{ "ls", {} }, .{ "cat", {} }, .{ "grep", {} }, .{ "find", {} },
    .{ "head", {} }, .{ "tail", {} }, .{ "wc", {} }, .{ "curl", {} },
    .{ "echo", {} }, .{ "date", {} }, .{ "whoami", {} }, .{ "hostname", {} },
    .{ "uname", {} }, .{ "which", {} }, .{ "pwd", {} }, .{ "ps", {} },
    .{ "uptime", {} }, .{ "df", {} }, .{ "du", {} },
});

fn strArg(args: Value, key: []const u8) ?[]const u8 {
    if (args != .object) return null;
    const v = args.object.get(key) orelse return null;
    if (v != .string) return null;
    return v.string;
}

fn boolArg(args: Value, key: []const u8) bool {
    if (args != .object) return false;
    const v = args.object.get(key) orelse return false;
    return (v == .bool and v.bool);
}

fn textResult(allocator: Allocator, text: []const u8) !Value {
    var arr = std.json.Array.init(allocator);
    var obj = std.json.ObjectMap.init(allocator);
    try obj.put("type", Value{ .string = "text" });
    try obj.put("text", Value{ .string = text });
    try arr.append(Value{ .object = obj });
    var res = std.json.ObjectMap.init(allocator);
    try res.put("content", Value{ .array = arr });
    return Value{ .object = res };
}

fn errResult(allocator: Allocator, text: []const u8) !Value {
    var r = try textResult(allocator, text);
    try r.object.put("isError", Value{ .bool = true });
    return r;
}

// ── Handlers ──

fn hShell(allocator: Allocator, args: Value, mode: AccessMode) !Value {
    const cmd = strArg(args, "command") orelse return errResult(allocator, "missing command");
    const cwd = strArg(args, "cwd");

    if (mode != .full) {
        const sp = std.mem.indexOfScalar(u8, cmd, ' ') orelse cmd.len;
        if (safe_cmds.get(cmd[0..sp]) == null) {
            return errResult(allocator, "command blocked in this mode");
        }
    }

    const r = shell.exec(allocator, cmd, cwd) catch |e|
        return errResult(allocator, @errorName(e));

    const text = try std.fmt.allocPrint(allocator, "stdout:\n{s}\n\nstderr:\n{s}\n\nexit: {d}", .{r.stdout, r.stderr, r.exit_code});
    return textResult(allocator, text);
}

fn hSysInfo(allocator: Allocator, _: Value, _: AccessMode) !Value {
    const text = try std.fmt.allocPrint(allocator, "os: {s}\narch: {s}", .{
        @tagName(@import("builtin").os.tag), @tagName(@import("builtin").cpu.arch)
    });
    return textResult(allocator, text);
}

fn hListApps(allocator: Allocator, _: Value, mode: AccessMode) !Value {
    const limit = if (mode == .full) "50" else "30";
    const r = shell.exec(allocator, try std.fmt.allocPrint(allocator, "ps aux --no-headers | head -{s}", .{limit}), null) catch |e|
        return errResult(allocator, @errorName(e));
    return textResult(allocator, r.stdout);
}

fn hSpawn(allocator: Allocator, args: Value, mode: AccessMode) !Value {
    if (mode != .full) return errResult(allocator, "full mode only");
    const cmd = strArg(args, "command") orelse return errResult(allocator, "missing command");
    _ = shell.spawn(allocator, cmd) catch |e| return errResult(allocator, @errorName(e));
    return textResult(allocator, "spawned");
}

fn hClipRead(allocator: Allocator, _: Value, _: AccessMode) !Value {
    const r = shell.exec(allocator, "pbpaste", null) catch |e|
        return errResult(allocator, @errorName(e));
    return textResult(allocator, if (r.stdout.len > 0) r.stdout else "(empty)");
}

fn hClipWrite(allocator: Allocator, args: Value, _: AccessMode) !Value {
    const text = strArg(args, "text") orelse return errResult(allocator, "missing text");
    var child = std.process.Child.init(&[_][]const u8{ "pbcopy" }, allocator);
    child.stdin_behavior = .Pipe;
    try child.spawn();
    if (child.stdin) |s| { try s.writer().writeAll(text); s.close(); }
    _ = try child.wait();
    return textResult(allocator, "copied to clipboard");
}

fn hOSA(allocator: Allocator, args: Value, _: AccessMode) !Value {
    const script = strArg(args, "script") orelse return errResult(allocator, "missing script");
    const cmd = try std.fmt.allocPrint(allocator, "osascript -e {s}", .{script});
    const r = shell.exec(allocator, cmd, null) catch |e| return errResult(allocator, @errorName(e));
    return textResult(allocator, r.stdout);
}

fn hTell(allocator: Allocator, args: Value, _: AccessMode) !Value {
    const app = strArg(args, "app") orelse return errResult(allocator, "missing app");
    const cmd_body = strArg(args, "command") orelse return errResult(allocator, "missing command");
    const script = try std.fmt.allocPrint(allocator, "tell application \"{s}\" to {s}", .{app, cmd_body});
    const cmd = try std.fmt.allocPrint(allocator, "osascript -e {s}", .{script});
    const r = shell.exec(allocator, cmd, null) catch |e| return errResult(allocator, @errorName(e));
    return textResult(allocator, r.stdout);
}

fn hScreenshot(allocator: Allocator, args: Value, _: AccessMode) !Value {
    _ = args;
    const r = shell.exec(allocator, "screencapture -x /tmp/folk_screenshot.png", null) catch |e|
        return errResult(allocator, @errorName(e));
    _ = r;
    return textResult(allocator, "screenshot -> /tmp/folk_screenshot.png");
}

// ── Tool table ──

pub const ToolEntry = struct {
    name: []const u8,
    description: []const u8,
    input_schema: Value,
    handler: *const fn (Allocator, Value, AccessMode) anyerror!Value,
};

pub const ToolTable = struct {
    allocator: Allocator,
    tools: std.ArrayList(ToolEntry),
    mode: AccessMode,

    pub fn init(allocator: Allocator, mode: AccessMode) ToolTable {
        var tt = ToolTable{ .allocator = allocator, .tools = std.ArrayList(ToolEntry).init(allocator), .mode = mode };
        registerAll(&tt);
        return tt;
    }

    pub fn deinit(self: *ToolTable) void { self.tools.deinit(); }

    pub fn call(self: *ToolTable, name: []const u8, args: Value) !Value {
        for (self.tools.items) |t| {
            if (std.mem.eql(u8, t.name, name)) return t.handler(self.allocator, args, self.mode);
        }
        return errResult(self.allocator, "not found");
    }
};

fn reg(tt: *ToolTable, name: []const u8, desc: []const u8, handler: anytype) !void {
    try tt.tools.append(ToolEntry{
        .name = name, .description = desc,
        .input_schema = Value{ .null = {} },
        .handler = handler,
    });
}

fn registerAll(tt: *ToolTable) void {
    reg(tt, "folk_shell", "Execute a shell command", hShell) catch {};
    reg(tt, "folk_system_info", "System info", hSysInfo) catch {};
    reg(tt, "folk_list_apps", "List running processes", hListApps) catch {};
    reg(tt, "folk_spawn", "Spawn background process (full mode)", hSpawn) catch {};
    reg(tt, "folk_clipboard_read", "Read clipboard contents", hClipRead) catch {};
    reg(tt, "folk_clipboard_write", "Write to clipboard", hClipWrite) catch {};
    reg(tt, "folk_osascript", "Execute AppleScript (macOS)", hOSA) catch {};
    reg(tt, "folk_tell", "Tell an app (macOS)", hTell) catch {};
    reg(tt, "folk_screenshot", "Take screenshot (macOS)", hScreenshot) catch {};
}