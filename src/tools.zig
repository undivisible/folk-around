const std = @import("std");
const shell = @import("shell.zig");

const Allocator = std.mem.Allocator;
const Value = std.json.Value;

pub const AccessMode = enum {
    full,
    limited,
    sandbox,
    pub fn fromName(n: []const u8) ?AccessMode {
        if (std.mem.eql(u8, n, "full")) return .full;
        if (std.mem.eql(u8, n, "limited")) return .limited;
        if (std.mem.eql(u8, n, "sandbox")) return .sandbox;
        return null;
    }
};

const safe_cmds = std.StaticStringMap(void).initComptime(.{
    .{ "ls", {} },     .{ "cat", {} },   .{ "grep", {} },   .{ "find", {} },
    .{ "head", {} },   .{ "tail", {} },  .{ "wc", {} },     .{ "curl", {} },
    .{ "echo", {} },   .{ "date", {} },  .{ "whoami", {} }, .{ "hostname", {} },
    .{ "uname", {} },  .{ "which", {} }, .{ "pwd", {} },    .{ "ps", {} },
    .{ "uptime", {} }, .{ "df", {} },    .{ "du", {} },
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

fn deinitValue(allocator: Allocator, value: Value) void {
    switch (value) {
        .array => |array| {
            for (array.items) |item| deinitValue(allocator, item);
            var owned = array;
            owned.deinit();
        },
        .object => |object| {
            var it = object.iterator();
            while (it.next()) |entry| deinitValue(allocator, entry.value_ptr.*);
            var owned = object;
            owned.deinit(allocator);
        },
        else => {},
    }
}

fn map(allocator: Allocator) !Value {
    return Value{ .object = try std.json.ObjectMap.init(allocator, &.{}, &.{}) };
}

fn makeArray(allocator: Allocator) Value {
    return Value{ .array = std.json.Array.init(allocator) };
}

fn put(map_value: *Value, allocator: Allocator, key: []const u8, val: Value) !void {
    try map_value.object.put(allocator, key, val);
}

fn stringProperty(allocator: Allocator, description: []const u8) !Value {
    var prop = try map(allocator);
    try put(&prop, allocator, "type", Value{ .string = "string" });
    try put(&prop, allocator, "description", Value{ .string = description });
    return prop;
}

fn objectSchema(allocator: Allocator) !Value {
    var schema = try map(allocator);
    try put(&schema, allocator, "type", Value{ .string = "object" });
    try put(&schema, allocator, "properties", try map(allocator));
    return schema;
}

fn addProperty(schema: *Value, allocator: Allocator, name: []const u8, property: Value) !void {
    const properties = schema.object.getPtr("properties").?;
    try put(properties, allocator, name, property);
}

fn addRequired(schema: *Value, allocator: Allocator, fields: []const []const u8) !void {
    var required = makeArray(allocator);
    for (fields) |field| try required.array.append(Value{ .string = field });
    try put(schema, allocator, "required", required);
}

fn schemaFor(allocator: Allocator, name: []const u8) !Value {
    var schema = try objectSchema(allocator);
    if (std.mem.eql(u8, name, "folk_shell")) {
        try addProperty(&schema, allocator, "command", try stringProperty(allocator, "Command to execute"));
        try addProperty(&schema, allocator, "cwd", try stringProperty(allocator, "Working directory"));
        try addRequired(&schema, allocator, &.{"command"});
    } else if (std.mem.eql(u8, name, "folk_spawn")) {
        try addProperty(&schema, allocator, "command", try stringProperty(allocator, "Command to spawn"));
        try addRequired(&schema, allocator, &.{"command"});
    } else if (std.mem.eql(u8, name, "folk_clipboard_write")) {
        try addProperty(&schema, allocator, "text", try stringProperty(allocator, "Text to copy"));
        try addRequired(&schema, allocator, &.{"text"});
    } else if (std.mem.eql(u8, name, "folk_osascript")) {
        try addProperty(&schema, allocator, "script", try stringProperty(allocator, "AppleScript source"));
        try addRequired(&schema, allocator, &.{"script"});
    } else if (std.mem.eql(u8, name, "folk_tell")) {
        try addProperty(&schema, allocator, "app", try stringProperty(allocator, "Application name"));
        try addProperty(&schema, allocator, "command", try stringProperty(allocator, "AppleScript command body"));
        try addRequired(&schema, allocator, &.{ "app", "command" });
    }
    return schema;
}

fn textResult(allocator: Allocator, text: []const u8) !Value {
    var arr = std.json.Array.init(allocator);
    var obj = std.json.ObjectMap.init(allocator, &.{}, &.{}) catch unreachable;
    try obj.put(allocator, "type", Value{ .string = "text" });
    try obj.put(allocator, "text", Value{ .string = text });
    try arr.append(Value{ .object = obj });
    var res = std.json.ObjectMap.init(allocator, &.{}, &.{}) catch unreachable;
    try res.put(allocator, "content", Value{ .array = arr });
    return Value{ .object = res };
}

fn errResult(allocator: Allocator, text: []const u8) !Value {
    var r = try textResult(allocator, text);
    try r.object.put(allocator, "isError", Value{ .bool = true });
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

    const text = try std.fmt.allocPrint(allocator, "stdout:\n{s}\n\nstderr:\n{s}\n\nexit: {d}", .{ r.stdout, r.stderr, r.exit_code });
    return textResult(allocator, text);
}

fn hSysInfo(allocator: Allocator, _: Value, _: AccessMode) !Value {
    const text = try std.fmt.allocPrint(allocator, "os: {s}\narch: {s}", .{ @tagName(@import("builtin").os.tag), @tagName(@import("builtin").cpu.arch) });
    return textResult(allocator, text);
}

fn hListApps(allocator: Allocator, _: Value, mode: AccessMode) !Value {
    const limit: usize = if (mode == .full) 50 else 30;
    const r = shell.execArgv(allocator, &.{ "ps", "ax", "-o", "pid=,comm=" }, null) catch |e|
        return errResult(allocator, @errorName(e));
    var lines = std.mem.splitScalar(u8, r.stdout, '\n');
    var text: std.ArrayList(u8) = .empty;
    defer text.deinit(allocator);
    var count: usize = 0;
    while (count < limit) : (count += 1) {
        const line = lines.next() orelse break;
        try text.appendSlice(allocator, line);
        try text.append(allocator, '\n');
    }
    return textResult(allocator, text.items);
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
    _ = shell.execArgvInput(allocator, &.{"/usr/bin/pbcopy"}, text, null) catch |e| return errResult(allocator, @errorName(e));
    return textResult(allocator, "copied to clipboard");
}

fn hOSA(allocator: Allocator, args: Value, _: AccessMode) !Value {
    const script = strArg(args, "script") orelse return errResult(allocator, "missing script");
    const r = shell.execArgv(allocator, &.{ "osascript", "-e", script }, null) catch |e| return errResult(allocator, @errorName(e));
    return textResult(allocator, r.stdout);
}

fn hTell(allocator: Allocator, args: Value, _: AccessMode) !Value {
    const app = strArg(args, "app") orelse return errResult(allocator, "missing app");
    const cmd_body = strArg(args, "command") orelse return errResult(allocator, "missing command");
    const script = try std.fmt.allocPrint(allocator, "tell application \"{s}\" to {s}", .{ app, cmd_body });
    defer allocator.free(script);
    const r = shell.execArgv(allocator, &.{ "osascript", "-e", script }, null) catch |e| return errResult(allocator, @errorName(e));
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
        var tt = ToolTable{ .allocator = allocator, .tools = .empty, .mode = mode };
        registerAll(&tt);
        return tt;
    }

    pub fn deinit(self: *ToolTable) void {
        for (self.tools.items) |tool| deinitValue(self.allocator, tool.input_schema);
        self.tools.deinit(self.allocator);
    }

    pub fn call(self: *ToolTable, name: []const u8, args: Value) !Value {
        for (self.tools.items) |t| {
            if (std.mem.eql(u8, t.name, name)) return t.handler(self.allocator, args, self.mode);
        }
        return errResult(self.allocator, "not found");
    }
};

fn reg(tt: *ToolTable, name: []const u8, desc: []const u8, handler: anytype) !void {
    try tt.tools.append(tt.allocator, ToolEntry{
        .name = name,
        .description = desc,
        .input_schema = try schemaFor(tt.allocator, name),
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

test "all registered tools advertise object input schemas" {
    const allocator = std.testing.allocator;
    var table = ToolTable.init(allocator, .full);
    defer table.deinit();

    try std.testing.expectEqual(@as(usize, 9), table.tools.items.len);
    for (table.tools.items) |tool| {
        try std.testing.expect(tool.input_schema == .object);
        const schema_type = tool.input_schema.object.get("type") orelse return error.MissingSchemaType;
        try std.testing.expectEqualStrings("object", schema_type.string);
        _ = tool.input_schema.object.get("properties") orelse return error.MissingSchemaProperties;
    }
}

test "shell tool schema declares command as required" {
    const allocator = std.testing.allocator;
    var table = ToolTable.init(allocator, .full);
    defer table.deinit();

    const tool = table.tools.items[0];
    try std.testing.expectEqualStrings("folk_shell", tool.name);
    const required = tool.input_schema.object.get("required") orelse return error.MissingRequired;
    try std.testing.expect(required == .array);
    try std.testing.expectEqual(@as(usize, 1), required.array.items.len);
    try std.testing.expectEqualStrings("command", required.array.items[0].string);
}
