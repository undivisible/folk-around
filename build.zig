const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    // CLI daemon
    const exe = b.addExecutable(.{
        .name = "folk-around",
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
    });
    b.installArtifact(exe);

    // macOS menu bar app (Zig + AppKit)
    const app = b.addExecutable(.{
        .name = "FolkAround",
        .root_source_file = b.path("src/mac_app.zig"),
        .target = target,
        .optimize = optimize,
    });
    app.linkFramework("Cocoa");
    app.linkFramework("ApplicationServices");
    b.installArtifact(app);

    // Helper: build both CLI and app
    const all_step = b.step("all", "Build daemon + menu bar app");
    all_step.dependOn(&exe.step);
    all_step.dependOn(&app.step);

    // Run step for the daemon
    const run_cmd = b.addRunArtifact(exe);
    run_cmd.step.dependOn(b.getInstallStep());
    if (b.args) |args| run_cmd.addArgs(args);
    const run_step = b.step("run", "Run folk-around daemon");
    run_step.dependOn(&run_cmd.step);
}