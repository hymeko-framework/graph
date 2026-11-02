//! Independent P3.4 audit measurements.
//!
//! This is deliberately separate from the P3.4 claim-generating harnesses. It uses fresh raw output,
//! independent (non-generation-stamped) DSU oracles, at least seven measured repetitions after warm-up,
//! quartiles, worst values, alternating A/B order, and A/A null controls.

use hymeko::util::measurement::{AllocationSnapshot, CountingAllocator};
use hymeko::util::measurement::{MeasurementRunner, Runner};
use std::hint::black_box;

use hymeko::common::ids::NodeId;
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::Ir;
use hymeko_graph::{
    GraphicMatroidWorkspace, LeviCoord, MatroidRef, TraversalWorkspace, connected_components,
    levi_components, levi_rank, levi_spanning_basis, par_batch_reachable_counts,
    rank2_spanning_forest, rank2_weighted_basis,
};

#[path = "informed_common/mod.rs"]
mod ic;

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

#[test]
fn shared_runner_reuses_storage_without_allocating() {
    struct DeterministicClock;
    impl hymeko::util::measurement::Clock for DeterministicClock {
        type Start = ();
        fn start(&mut self) {}
        fn elapsed_us(&mut self, (): ()) -> f64 {
            1.0
        }
    }
    let mut runner = MeasurementRunner::with_clock(2, 9, DeterministicClock);
    let before = AllocationSnapshot::current_thread();
    let single = runner.run(|| 7u64);
    let (left, right) = runner.compare(|| 11u64, || 13u64);
    let after = AllocationSnapshot::current_thread().since(before);
    assert_eq!(after.allocations, 0);
    assert_eq!(after.bytes, 0);
    assert_eq!((single.median, left.median, right.median), (1.0, 1.0, 1.0));
}

fn allocation_snapshot() -> (u64, u64) {
    let snapshot = AllocationSnapshot::current_thread();
    (snapshot.allocations, snapshot.bytes)
}

fn allocation_delta(before: (u64, u64)) -> (u64, u64) {
    let after = allocation_snapshot();
    (after.0 - before.0, after.1 - before.1)
}

type Stats = hymeko::util::measurement::Distribution;

fn measure(f: impl FnMut()) -> Stats {
    MeasurementRunner::new(2, 9).run(f)
}

fn measure_alternating(a: impl FnMut(), b: impl FnMut()) -> (Stats, Stats) {
    MeasurementRunner::new(2, 9).compare(a, b)
}

fn dims(ir: &Ir) -> (usize, usize, usize) {
    (
        ir.nodes.len(),
        ir.arcs.len(),
        ir.arcs.iter().map(|a| a.refs.len()).sum(),
    )
}

#[allow(clippy::too_many_arguments)]
fn timed_row(
    rows: &mut Vec<String>,
    section: &str,
    workload: &str,
    variant: &str,
    ir: &Ir,
    stats: Stats,
    checksum: u64,
    value: f64,
    unit: &str,
    notes: &str,
) {
    let (nodes, arcs, incidences) = dims(ir);
    rows.push(
        [
            section.to_string(),
            workload.to_string(),
            variant.to_string(),
            nodes.to_string(),
            arcs.to_string(),
            incidences.to_string(),
            "2".to_string(),
            "9".to_string(),
            format!("{:.3}", stats.median),
            format!("{:.3}", stats.q1),
            format!("{:.3}", stats.q3),
            format!("{:.3}", stats.worst),
            checksum.to_string(),
            format!("{value:.6}"),
            unit.to_string(),
            notes.to_string(),
        ]
        .join("\t"),
    );
}

#[allow(clippy::too_many_arguments)]
fn value_row(
    rows: &mut Vec<String>,
    section: &str,
    workload: &str,
    variant: &str,
    ir: &Ir,
    checksum: u64,
    value: f64,
    unit: &str,
    notes: &str,
) {
    let (nodes, arcs, incidences) = dims(ir);
    rows.push(
        [
            section.to_string(),
            workload.to_string(),
            variant.to_string(),
            nodes.to_string(),
            arcs.to_string(),
            incidences.to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            checksum.to_string(),
            format!("{value:.6}"),
            unit.to_string(),
            notes.to_string(),
        ]
        .join("\t"),
    );
}

/// A deliberately plain oracle: eager initialization, recursive path compression, union by rank, and
/// no production workspace or generation-reset code.
struct OracleDsu {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl OracleDsu {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }

    fn union(&mut self, a: usize, b: usize) -> bool {
        let (mut a, mut b) = (self.find(a), self.find(b));
        if a == b {
            return false;
        }
        if self.rank[a] < self.rank[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b] = a;
        if self.rank[a] == self.rank[b] {
            self.rank[a] += 1;
        }
        true
    }
}

fn oracle_levi(ir: &Ir) -> (usize, usize) {
    let n = ir.nodes.len();
    let vtx = n + ir.arcs.len();
    let mut dsu = OracleDsu::new(vtx);
    let mut rank = 0usize;
    for (ai, arc) in ir.arcs.iter().enumerate() {
        for r in &arc.refs {
            let node = ir
                .as_node(r.target())
                .expect("audit workloads contain node targets");
            rank += usize::from(dsu.union(node.raw(), n + ai));
        }
    }
    (rank, vtx - rank)
}

fn audit_levi_oracle(
    rows: &mut Vec<String>,
    name: &str,
    ir: Ir,
    expected_components: Option<usize>,
) {
    let idx = build_incidence_index(&ir).expect("index");
    let (oracle_rank, oracle_components) = oracle_levi(&ir);
    if let Some(expected) = expected_components {
        assert_eq!(oracle_components, expected, "{name}: expected components");
    }
    let mut ws = GraphicMatroidWorkspace::new();
    let actual = levi_rank(MatroidRef::new(&idx), &mut ws).expect("production Levi rank");
    assert_eq!(actual, oracle_rank, "{name}: independent Levi rank");
    value_row(
        rows,
        "levi_oracle",
        name,
        "independent_eager_dsu",
        &ir,
        oracle_components as u64,
        oracle_rank as f64,
        "rank",
        "checksum=independent component count; production rank asserted equal",
    );
}

fn audit_rank2_timing(rows: &mut Vec<String>, name: &str, ir: Ir) {
    let idx = build_incidence_index(&ir).expect("index");
    let matroid = MatroidRef::new(&idx);
    let mut forest_ws = GraphicMatroidWorkspace::new();
    let mut weighted_ws = GraphicMatroidWorkspace::new();
    let mut null_ws = GraphicMatroidWorkspace::new();
    let mut forest_out = Vec::new();
    let mut weighted_out = Vec::new();
    let mut null_out = Vec::new();
    let (forest, weighted) = measure_alternating(
        || {
            black_box(rank2_spanning_forest(matroid, &mut forest_ws, &mut forest_out).unwrap());
        },
        || {
            black_box(
                rank2_weighted_basis(matroid, true, &mut weighted_ws, &mut weighted_out).unwrap(),
            );
        },
    );
    assert_eq!(forest_out.len(), weighted_out.len());
    let (null_a, null_b) = measure_alternating(
        || {
            black_box(rank2_spanning_forest(matroid, &mut forest_ws, &mut forest_out).unwrap());
        },
        || {
            black_box(rank2_spanning_forest(matroid, &mut null_ws, &mut null_out).unwrap());
        },
    );
    assert_eq!(forest_out, null_out);
    let arcs = ir.arcs.len() as f64;
    timed_row(
        rows,
        "rank2_timing",
        name,
        "forest",
        &ir,
        forest,
        forest_out.len() as u64,
        forest.median * 1000.0 / arcs,
        "ns_per_arc",
        "alternating with weighted basis",
    );
    timed_row(
        rows,
        "rank2_timing",
        name,
        "weighted_min",
        &ir,
        weighted,
        weighted_out.len() as u64,
        weighted.median / forest.median,
        "ratio_vs_forest",
        "alternating with forest; includes edge collection, allocation, sort, output",
    );
    timed_row(
        rows,
        "aa_null",
        name,
        "forest_a",
        &ir,
        null_a,
        forest_out.len() as u64,
        null_a.median / null_b.median,
        "ratio_a_over_b",
        "same operation, alternating independent workspaces",
    );
    timed_row(
        rows,
        "aa_null",
        name,
        "forest_b",
        &ir,
        null_b,
        null_out.len() as u64,
        null_b.median / null_a.median,
        "ratio_b_over_a",
        "same operation, alternating independent workspaces",
    );
}

fn audit_levi_timing(rows: &mut Vec<String>, name: &str, ir: Ir) {
    let idx = build_incidence_index(&ir).expect("index");
    let matroid = MatroidRef::new(&idx);
    let mut a_ws = GraphicMatroidWorkspace::new();
    let mut b_ws = GraphicMatroidWorkspace::new();
    let mut a_out: Vec<LeviCoord> = Vec::new();
    let mut b_out: Vec<LeviCoord> = Vec::new();
    let (a, b) = measure_alternating(
        || {
            black_box(levi_spanning_basis(matroid, &mut a_ws, &mut a_out).unwrap());
        },
        || {
            black_box(levi_spanning_basis(matroid, &mut b_ws, &mut b_out).unwrap());
        },
    );
    assert_eq!(a_out, b_out);
    let inc = dims(&ir).2 as f64;
    timed_row(
        rows,
        "levi_timing",
        name,
        "basis_a",
        &ir,
        a,
        a_out.len() as u64,
        a.median * 1000.0 / inc,
        "ns_per_incidence",
        "alternating A/A; warm workspace and output capacity",
    );
    timed_row(
        rows,
        "aa_null",
        name,
        "levi_basis_b",
        &ir,
        b,
        b_out.len() as u64,
        b.median / a.median,
        "ratio_b_over_a",
        "same operation, alternating independent workspaces",
    );
}

fn audit_batch_scaling(rows: &mut Vec<String>, name: &str, ir: Ir) {
    let idx = build_incidence_index(&ir).expect("index");
    let n = ir.nodes.len();
    let step = (n / 64).max(1);
    let starts: Vec<NodeId> = (0..n).step_by(step).take(64).map(NodeId::new).collect();
    assert_eq!(starts.len(), 64);
    let mut baseline = None;
    let mut checksum = None;
    for threads in [1usize, 2, 4, 8, 16, 24] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .expect("local Rayon pool");
        let mut current = 0u64;
        let stats = measure(|| {
            let counts = pool
                .install(|| par_batch_reachable_counts(&idx, &starts))
                .expect("batch reachable");
            current = counts.iter().map(|&x| x as u64).sum();
            black_box(&counts);
        });
        match checksum {
            None => checksum = Some(current),
            Some(expected) => {
                assert_eq!(current, expected, "{name}: checksum at {threads} threads")
            }
        }
        let base = *baseline.get_or_insert(stats.median);
        timed_row(
            rows,
            "batch_scaling",
            name,
            &format!("{threads}_threads"),
            &ir,
            stats,
            current,
            base / stats.median,
            "speedup_vs_1_thread",
            "fixed batch of 64 independent serial reachability searches; local Rayon pool",
        );
    }
}

#[derive(Clone)]
struct ControlDsu {
    parent: Vec<u32>,
}

impl ControlDsu {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n as u32).collect(),
        }
    }

    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let gp = self.parent[self.parent[x as usize] as usize];
            self.parent[x as usize] = gp;
            x = gp;
        }
        x
    }

    fn union(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.parent[a as usize] = b;
        }
    }

    fn node_components(&mut self, n_nodes: usize) -> usize {
        let mut seen = vec![false; self.parent.len()];
        let mut count = 0;
        for node in 0..n_nodes as u32 {
            let root = self.find(node) as usize;
            if !seen[root] {
                seen[root] = true;
                count += 1;
            }
        }
        count
    }
}

struct StarCsr {
    row: Vec<u32>,
    col: Vec<u32>,
    n_nodes: usize,
}

impl StarCsr {
    fn build(ir: &Ir) -> Self {
        let n = ir.nodes.len();
        let vtx = n + ir.arcs.len();
        let mut deg = vec![0u32; vtx + 1];
        for (ai, arc) in ir.arcs.iter().enumerate() {
            for r in &arc.refs {
                let node = ir.as_node(r.target()).expect("node incidence").raw();
                deg[node] += 1;
                deg[n + ai] += 1;
            }
        }
        let mut row = vec![0u32; vtx + 1];
        for i in 0..vtx {
            row[i + 1] = row[i] + deg[i];
        }
        let mut cursor = row.clone();
        let mut col = vec![0u32; row[vtx] as usize];
        for (ai, arc) in ir.arcs.iter().enumerate() {
            for r in &arc.refs {
                let u = ir.as_node(r.target()).expect("node incidence").raw();
                let v = n + ai;
                col[cursor[u] as usize] = v as u32;
                cursor[u] += 1;
                col[cursor[v] as usize] = u as u32;
                cursor[v] += 1;
            }
        }
        Self {
            row,
            col,
            n_nodes: n,
        }
    }

    fn components(&self) -> usize {
        let mut dsu = ControlDsu::new(self.row.len() - 1);
        for u in 0..self.row.len() - 1 {
            for k in self.row[u]..self.row[u + 1] {
                dsu.union(u as u32, self.col[k as usize]);
            }
        }
        dsu.node_components(self.n_nodes)
    }

    fn retained_bytes(&self) -> usize {
        (self.row.capacity() + self.col.capacity()) * size_of::<u32>()
    }
}

struct Clique2Section {
    edges: Vec<(u32, u32)>,
    n_nodes: usize,
}

impl Clique2Section {
    fn build(ir: &Ir) -> Self {
        let mut edges = Vec::new();
        for arc in &ir.arcs {
            let members: Vec<u32> = arc
                .refs
                .iter()
                .map(|r| ir.as_node(r.target()).expect("node incidence").raw() as u32)
                .collect();
            for i in 0..members.len() {
                for j in i + 1..members.len() {
                    edges.push((members[i], members[j]));
                }
            }
        }
        Self {
            edges,
            n_nodes: ir.nodes.len(),
        }
    }

    fn components(&self) -> usize {
        let mut dsu = ControlDsu::new(self.n_nodes);
        for &(u, v) in &self.edges {
            dsu.union(u, v);
        }
        dsu.node_components(self.n_nodes)
    }

    fn retained_bytes(&self) -> usize {
        self.edges.capacity() * size_of::<(u32, u32)>()
    }
}

fn cached_read(c: usize) {
    let mut sum = 0usize;
    for _ in 0..100_000 {
        sum = sum.wrapping_add(black_box(c));
    }
    black_box(sum);
}

fn audit_like_for_like_components(rows: &mut Vec<String>, name: &str, ir: Ir) {
    let idx = build_incidence_index(&ir).expect("index");
    let mut truth_ws = TraversalWorkspace::new();
    let truth = connected_components(&idx, &mut truth_ws)
        .expect("components")
        .count() as usize;

    let mut direct_ws = TraversalWorkspace::new();
    let mut direct_c = 0usize;
    let direct_prep = measure(|| {
        direct_c = connected_components(&idx, &mut direct_ws)
            .expect("components")
            .count() as usize;
        black_box(direct_c);
    });

    let mut levi_ws = GraphicMatroidWorkspace::new();
    let mut levi_c = 0usize;
    let levi_prep = measure(|| {
        levi_c = levi_components(MatroidRef::new(&idx), &mut levi_ws).expect("Levi components");
        black_box(levi_c);
    });

    let mut star_c = 0usize;
    let star_prep = measure(|| {
        let star = StarCsr::build(&ir);
        star_c = star.components();
        black_box(&star);
    });
    let star = StarCsr::build(&ir);

    let mut clique_c = 0usize;
    let clique_prep = measure(|| {
        let clique = Clique2Section::build(&ir);
        clique_c = clique.components();
        black_box(&clique);
    });
    let clique = Clique2Section::build(&ir);

    assert_eq!(direct_c, truth);
    assert_eq!(levi_c, truth);
    assert_eq!(star_c, truth);
    assert_eq!(clique_c, truth);

    for (variant, prep, c, retained, note) in [
        (
            "direct_cached_count",
            direct_prep,
            direct_c,
            idx.retained_bytes(),
            "prep computes canonical connected-component count; query reads cached usize",
        ),
        (
            "levi_cached_count",
            levi_prep,
            levi_c,
            levi_ws.retained_bytes(),
            "prep computes implicit-Levi component count; query reads cached usize",
        ),
        (
            "sparse_star_cached_count",
            star_prep,
            star_c,
            star.retained_bytes(),
            "prep builds sparse bipartite CSR and computes count; query reads cached usize",
        ),
        (
            "clique_cached_count",
            clique_prep,
            clique_c,
            clique.retained_bytes(),
            "prep builds 2-section and computes count; query reads cached usize",
        ),
    ] {
        timed_row(
            rows,
            "component_count_prep",
            name,
            variant,
            &ir,
            prep,
            c as u64,
            retained as f64,
            "retained_bytes",
            note,
        );
        let query = measure(|| cached_read(c));
        timed_row(
            rows,
            "component_count_query",
            name,
            variant,
            &ir,
            query,
            c as u64,
            query.median * 1000.0 / 100_000.0,
            "ns_per_cached_read",
            "identical 100000-read result materialization loop for every representation",
        );
    }
}

fn allocation_row(
    rows: &mut Vec<String>,
    workload: &str,
    stage: &str,
    delta: (u64, u64),
    retained: usize,
    notes: &str,
) {
    rows.push(
        [
            workload.to_string(),
            stage.to_string(),
            delta.0.to_string(),
            delta.1.to_string(),
            retained.to_string(),
            notes.to_string(),
        ]
        .join("\t"),
    );
}

fn audit_allocations(rows: &mut Vec<String>, name: &str, build: impl FnOnce() -> Ir) {
    let before = allocation_snapshot();
    let ir = build();
    allocation_row(
        rows,
        name,
        "ir_construction",
        allocation_delta(before),
        0,
        "direct canonical-Ir construction; no textual source stage",
    );

    let before = allocation_snapshot();
    let idx = build_incidence_index(&ir).expect("index");
    allocation_row(
        rows,
        name,
        "incidence_index",
        allocation_delta(before),
        idx.retained_bytes(),
        "index build only",
    );

    let before = allocation_snapshot();
    for _ in 0..100_000 {
        black_box(MatroidRef::new(&idx));
    }
    allocation_row(
        rows,
        name,
        "facade_100k",
        allocation_delta(before),
        size_of::<MatroidRef<'_, '_>>(),
        "100000 facade constructions; retained is facade structural size",
    );

    let matroid = MatroidRef::new(&idx);
    let mut ws = GraphicMatroidWorkspace::new();
    let before = allocation_snapshot();
    let rank = levi_rank(matroid, &mut ws).expect("reserve through rank");
    allocation_row(
        rows,
        name,
        "levi_workspace_reserve",
        allocation_delta(before),
        ws.retained_bytes(),
        "rank without output; allocation is reusable DSU capacity",
    );

    let mut out = Vec::new();
    let before = allocation_snapshot();
    assert_eq!(
        levi_spanning_basis(matroid, &mut ws, &mut out).unwrap(),
        rank
    );
    allocation_row(
        rows,
        name,
        "levi_output_materialization",
        allocation_delta(before),
        out.capacity() * size_of::<LeviCoord>(),
        "workspace already warm; fresh basis Vec",
    );

    let before = allocation_snapshot();
    assert_eq!(
        levi_spanning_basis(matroid, &mut ws, &mut out).unwrap(),
        rank
    );
    allocation_row(
        rows,
        name,
        "levi_warm_reuse",
        allocation_delta(before),
        ws.retained_bytes() + out.capacity() * size_of::<LeviCoord>(),
        "workspace and output capacity both warm",
    );
}

#[test]
#[ignore = "independent P3.4 audit measurement; --release --ignored --nocapture"]
fn independent_graph_measurements() {
    let mut rows = vec![
        [
            "section",
            "workload",
            "variant",
            "nodes",
            "arcs",
            "incidences",
            "warmups",
            "samples",
            "median_us",
            "q1_us",
            "q3_us",
            "worst_us",
            "checksum",
            "value",
            "unit",
            "notes",
        ]
        .join("\t"),
    ];
    let mut allocations = vec![
        [
            "workload",
            "stage",
            "allocations",
            "allocated_bytes",
            "retained_bytes",
            "notes",
        ]
        .join("\t"),
    ];

    audit_levi_oracle(&mut rows, "sts31", ic::steiner_sts_ir(5), Some(1));
    audit_levi_oracle(&mut rows, "sts127", ic::steiner_sts_ir(7), Some(1));
    audit_levi_oracle(&mut rows, "sts511", ic::steiner_sts_ir(9), Some(1));
    audit_levi_oracle(&mut rows, "sts1023", ic::steiner_sts_ir(10), Some(1));
    let (two_sts, _) = ic::two_disjoint_sts_ir(3);
    audit_levi_oracle(&mut rows, "two_sts7", two_sts, Some(2));
    audit_levi_oracle(
        &mut rows,
        "random1e6",
        ic::uniform_rank_k_ir(0x51EED, 200_000, 333_333, 3),
        None,
    );
    audit_levi_oracle(
        &mut rows,
        "transport",
        ic::map_grid_with_transport_ir(256, 256, &ic::map_obstacles(256, 256), 8, 5.0),
        None,
    );

    audit_rank2_timing(
        &mut rows,
        "path65k",
        ic::window_ir(65_536, 2, true, ic::SignMode::AllPlus),
    );
    audit_rank2_timing(&mut rows, "grid256", ic::grid_ir(256, 256));

    audit_levi_timing(&mut rows, "sts1023", ic::steiner_sts_ir(10));
    audit_levi_timing(
        &mut rows,
        "random1e6",
        ic::uniform_rank_k_ir(0x51EED, 200_000, 333_333, 3),
    );

    audit_batch_scaling(&mut rows, "sts1023", ic::steiner_sts_ir(10));
    audit_batch_scaling(
        &mut rows,
        "random1e6",
        ic::uniform_rank_k_ir(0x51EED, 200_000, 333_333, 3),
    );
    audit_batch_scaling(
        &mut rows,
        "map512",
        ic::map_grid_ir(512, 512, &ic::map_obstacles(512, 512)),
    );

    audit_like_for_like_components(&mut rows, "sts1023", ic::steiner_sts_ir(10));
    audit_like_for_like_components(
        &mut rows,
        "random1e6",
        ic::uniform_rank_k_ir(0x51EED, 200_000, 333_333, 3),
    );
    audit_like_for_like_components(
        &mut rows,
        "map512",
        ic::map_grid_ir(512, 512, &ic::map_obstacles(512, 512)),
    );

    audit_allocations(&mut allocations, "sts1023", || ic::steiner_sts_ir(10));
    audit_allocations(&mut allocations, "random1e6", || {
        ic::uniform_rank_k_ir(0x51EED, 200_000, 333_333, 3)
    });
    audit_allocations(&mut allocations, "path65k", || {
        ic::window_ir(65_536, 2, false, ic::SignMode::AllPlus)
    });

    let report_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../reports/data");
    let rerun = report_dir.join("2026-09-03-codex-p34-rerun.tsv");
    let alloc = report_dir.join("2026-09-03-codex-p34-allocations.tsv");
    std::fs::write(&rerun, rows.join("\n") + "\n").expect("write audit rerun TSV");
    std::fs::write(&alloc, allocations.join("\n") + "\n").expect("write audit allocation TSV");
    println!("wrote {}", rerun.display());
    println!("wrote {}", alloc.display());
}
