//! RingMPSC - Build Configuration
const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    // Core channel module
    const channel = b.addModule("channel", .{
        .root_source_file = b.path("src/channel.zig"),
    });

    // Executables
    const bins = [_]struct { name: []const u8, src: []const u8 }{
        .{ .name = "bench", .src = "src/bench_final.zig" },
        .{ .name = "test-fifo", .src = "src/test_fifo.zig" },
        .{ .name = "test-determinism", .src = "src/test_determinism.zig" },
        .{ .name = "test-chaos", .src = "src/test_chaos.zig" },
    };

    inline for (bins) |bin| {
        const mod = b.createModule(.{
            .root_source_file = b.path(bin.src),
            .target = target,
            .optimize = optimize,
        });
        mod.addImport("channel", channel);
        const exe = b.addExecutable(.{ .name = bin.name, .root_module = mod });
        b.installArtifact(exe);
    }

    // Unit tests
    const test_mod = b.createModule(.{
        .root_source_file = b.path("src/channel.zig"),
        .target = target,
        .optimize = optimize,
    });
    const tests = b.addTest(.{ .root_module = test_mod });
    b.step("test", "Run unit tests").dependOn(&b.addRunArtifact(tests).step);
}
