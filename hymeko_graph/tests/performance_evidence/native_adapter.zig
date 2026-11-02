//! Benchmark-only C ABI; algorithms belong to the unmodified pinned hypergraphz.
const std = @import("std");
const hgz = @import("hypergraphz");
const Vertex = struct {};
const Hyperedge = struct { weight: usize = 1 };
const Graph = hgz.HypergraphZ(Hyperedge, Vertex, .{});
const allocator = std.heap.smp_allocator;

const Handle = struct {
    graph: Graph,
    vertices: []hgz.HypergraphZId,
    components_supported: bool,

    fn init(n: u32, offsets: []const u32, members: []const u32) !*Handle {
        const self = try allocator.create(Handle);
        errdefer allocator.destroy(self);
        self.graph = try Graph.init(allocator, .{
            .vertices_capacity = n,
            .hyperedges_capacity = offsets.len - 1,
        });
        errdefer self.graph.deinit();
        // Upstream getConnectedComponents indexes pair[1] even for the one-item
        // window returned on a singleton edge. Refuse that unsupported input,
        // rather than changing upstream semantics or executing out-of-bounds code.
        self.components_supported = true;
        for (0..offsets.len - 1) |i| {
            if (offsets[i + 1] - offsets[i] == 1) self.components_supported = false;
        }
        const payloads = try allocator.alloc(Vertex, n);
        defer allocator.free(payloads);
        @memset(payloads, .{});
        self.vertices = try self.graph.createVertices(payloads);
        errdefer allocator.free(self.vertices);
        const edges_payload = try allocator.alloc(Hyperedge, offsets.len - 1);
        defer allocator.free(edges_payload);
        @memset(edges_payload, .{});
        const edges = try self.graph.createHyperedges(edges_payload);
        defer allocator.free(edges);
        var row: std.ArrayList(hgz.HypergraphZId) = .empty;
        defer row.deinit(allocator);
        for (edges, 0..) |edge, i| {
            row.clearRetainingCapacity();
            for (members[offsets[i]..offsets[i + 1]]) |member| {
                try row.append(allocator, self.vertices[member]);
            }
            try self.graph.appendVerticesToHyperedge(edge, row.items);
            const stored = try self.graph.getHyperedgeVertices(edge);
            if (!std.mem.eql(hgz.HypergraphZId, stored, row.items)) return error.BadMembership;
        }
        // Explicit ready-to-query boundary, outside operation timings.
        try self.graph.build();
        return self;
    }

    fn deinit(self: *Handle) void {
        self.graph.deinit();
        allocator.free(self.vertices);
        allocator.destroy(self);
    }
};

export fn hgz_native_create(n: u32, e: u32, offsets: [*]const u32, members: [*]const u32, length: u32) ?*Handle {
    // The explicit large-scale comparison has four million incidences.
    // Keep a bounded admission guard; the process runner also enforces 16 GiB.
    if (n > 1000000 or e > 500000 or length > 4000000 or offsets[0] != 0 or offsets[e] != length) return null;
    for (0..e) |i| {
        if (offsets[i] > offsets[i + 1] or offsets[i + 1] > length) return null;
    }
    for (members[0..length]) |member| {
        if (member >= n) return null;
    }
    return Handle.init(n, offsets[0 .. @as(usize, e) + 1], members[0..length]) catch null;
}

export fn hgz_native_destroy(self: *Handle) void {
    self.deinit();
}

export fn hgz_native_rank(self: *Handle, scores: [*]f64, n: u32, stats: [*]u64) u32 {
    if (n != self.vertices.len) return 0;
    var result = self.graph.computePageRank(.{
        .damping = 0.85,
        .tolerance = 1e-10 * (1.0 - 0.85) / 0.85,
        .max_iterations = 200,
    }) catch return 0;
    defer result.deinit(allocator);
    for (self.vertices, 0..) |id, i| {
        scores[i] = result.data.get(id) orelse return 0;
    }
    stats[0] = result.iterations;
    stats[1] = @intFromBool(result.converged);
    return 1;
}

export fn hgz_native_components(self: *Handle, labels: [*]u32, n: u32, count: *u32) u32 {
    if (n != self.vertices.len) return 0;
    if (!self.components_supported) return 2;
    var result = self.graph.getConnectedComponents() catch return 0;
    defer result.deinit(allocator);
    // Vertices are created first in a fresh graph, with IDs 1..n. Normalize each
    // component to its minimum vertex ordinal; label numbers are not semantics.
    for (result.data.items) |component| {
        var minimum: u32 = std.math.maxInt(u32);
        for (component) |id| {
            if (id == 0 or id > n) return 0;
            minimum = @min(minimum, id - 1);
        }
        for (component) |id| labels[id - 1] = minimum;
    }
    count.* = @intCast(result.data.items.len);
    return 1;
}
