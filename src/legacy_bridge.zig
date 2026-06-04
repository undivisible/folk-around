const std = @import("std");
const tools = @import("tools.zig");

export fn folk_zig_legacy_init() c_int {
    return 0;
}

export fn folk_zig_legacy_call(name_ptr: [*]const u8, name_len: usize, args_ptr: [*]const u8, args_len: usize, mode_raw: u8) ?[*:0]u8 {
    const allocator = std.heap.smp_allocator;
    const name = name_ptr[0..name_len];
    const args_json = args_ptr[0..args_len];
    const mode: tools.AccessMode = switch (mode_raw) {
        0 => .full,
        1 => .limited,
        2 => .sandbox,
        else => .full,
    };
    var parsed = std.json.parseFromSlice(std.json.Value, allocator, args_json, .{}) catch return null;
    defer parsed.deinit();
    var table = tools.ToolTable.init(allocator, mode);
    defer table.deinit();
    const result = table.call(name, parsed.value) catch return null;
    var buf: std.ArrayList(u8) = .empty;
    var writer: std.Io.Writer.Allocating = .fromArrayList(allocator, &buf);
    std.json.Stringify.value(result, .{}, &writer.writer) catch return null;
    buf = writer.toArrayList();
    buf.append(allocator, 0) catch return null;
    return @ptrCast(buf.items.ptr);
}

export fn folk_zig_legacy_free(ptr: [*:0]u8) void {
    const allocator = std.heap.smp_allocator;
    const bytes = std.mem.span(ptr);
    allocator.free(ptr[0 .. bytes.len + 1]);
}
