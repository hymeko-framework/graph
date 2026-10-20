//! P3.4 §7 — memory closure. A thread-local counting allocator measures the whole-workload allocation
//! profile (count + bytes), and the per-component retained footprint is reported from exact accessors
//! where they exist (`IncidenceIndex::retained_bytes`, `GraphicMatroidWorkspace::retained_bytes`) and
//! from an **allocation delta** otherwise (`TraversalWorkspace` / search scratch — labeled). The process
//! peak RSS per workload lives in the fresh-subprocess CPU TSV (`…-p34-cpu-scaling.tsv`); this file does
//! not conflate the ~12 B/incidence `IncidenceIndex` figure with total model memory — it reports every
//! component and the whole-process allocated bytes separately, then normalizes per entity.
//!
//! Run: cargo test -p hymeko_graph --release --test p34_memory measure_memory -- --ignored --nocapture

use hymeko::util::measurement::{AllocationSnapshot, CountingAllocator};

use hymeko::common::ids::NodeId;
use hymeko::ir::incidence::build_incidence_index;
use hymeko_graph::direct_ir::HyperTraversal;
use hymeko_graph::{
    GraphicMatroidWorkspace, LeviCoord, MatroidRef, TraversalWorkspace, connected_components,
    levi_spanning_basis, rank2_spanning_forest,
};

#[path = "informed_common/mod.rs"]
mod ic;

#[global_allocator]
static GA: CountingAllocator = CountingAllocator;

fn snap() -> (u64, u64) {
    let snapshot = AllocationSnapshot::current_thread();
    (snapshot.allocations, snapshot.bytes)
}
fn since(base: (u64, u64)) -> (u64, u64) {
    let n = snap();
    (n.0 - base.0, n.1 - base.1)
}

#[test]
#[ignore = "memory bench; run with --release --ignored --nocapture"]
fn measure_memory() {
    let workloads = [
        "sts511",
        "sts1023",
        "mycielski",
        "random1e6",
        "map512",
        "transport",
    ];
    let mut rows: Vec<String> = vec![
        [
            "workload",
            "decls",
            "nodes",
            "arcs",
            "incidences",
            "ir_bytes",
            "index_retained_b",
            "search_ws_b",
            "rank2_ws_b",
            "levi_ws_b",
            "basis_out_b",
            "total_allocs",
            "total_bytes",
            "b_per_decl",
            "b_per_node",
            "b_per_arc",
            "b_per_inc",
            "b_per_levi_ground",
        ]
        .join("\t"),
    ];

    println!(
        "\n# P3.4 memory closure (retained bytes per component; whole-workload alloc profile)"
    );
    println!(
        "{:<11} {:>10} {:>7} {:>8} {:>9} {:>12} {:>10} {:>10} {:>12} {:>7}",
        "workload",
        "inc",
        "idx_retB",
        "rank2_wsB",
        "levi_wsB",
        "total_bytes",
        "allocs",
        "b/inc",
        "b/levi_gr",
        "comps"
    );

    for w in workloads {
        let start = snap();
        let ir = ic::named_workload(w);
        let ir_delta = since(start);

        let idx = build_incidence_index(&ir).unwrap();
        let index_retained = idx.retained_bytes();

        // search scratch: one reachable_nodes, measure its allocation delta.
        let trav = HyperTraversal::new(&idx);
        let before_search = snap();
        let mut tw = TraversalWorkspace::new();
        let _ = trav.reachable_nodes(&mut tw, NodeId::new(0)).unwrap();
        let comps = connected_components(&idx, &mut tw).unwrap().count();
        let search_ws_delta = since(before_search);

        // rank-2 workspace (only meaningful on rank-2 graphs; else 0 / NA).
        let mut r2ws = GraphicMatroidWorkspace::new();
        let mut r2out = Vec::new();
        let rank2_ws_b = match rank2_spanning_forest(MatroidRef::new(&idx), &mut r2ws, &mut r2out) {
            Ok(_) => r2ws.retained_bytes(),
            Err(_) => 0,
        };

        // Levi workspace + basis output.
        let mut lws = GraphicMatroidWorkspace::new();
        let mut lout: Vec<LeviCoord> = Vec::new();
        levi_spanning_basis(MatroidRef::new(&idx), &mut lws, &mut lout).unwrap();
        let levi_ws_b = lws.retained_bytes();
        let basis_out_b = lout.capacity() * std::mem::size_of::<LeviCoord>();

        let total = since(start);
        assert!(
            total.1 > index_retained as u64,
            "whole-workload allocated bytes must exceed the IncidenceIndex alone (guards index-as-total-memory)"
        );
        let decls = ir.decl_nodes.len();
        let nodes = ir.nodes.len();
        let arcs = ir.arcs.len();
        let inc: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
        let bpe = |d: u64| {
            if d > 0 {
                total.1 as f64 / d as f64
            } else {
                0.0
            }
        };

        println!(
            "{w:<11} {inc:>10} {index_retained:>7} {rank2_ws_b:>8} {levi_ws_b:>9} {:>12} {:>10} {:>7.2} {:>7.2} {comps:>6}",
            total.1,
            total.0,
            bpe(inc as u64),
            bpe(inc as u64)
        );
        rows.push(
            [
                w.to_string(),
                decls.to_string(),
                nodes.to_string(),
                arcs.to_string(),
                inc.to_string(),
                ir_delta.1.to_string(),
                index_retained.to_string(),
                search_ws_delta.1.to_string(),
                rank2_ws_b.to_string(),
                levi_ws_b.to_string(),
                basis_out_b.to_string(),
                total.0.to_string(),
                total.1.to_string(),
                format!("{:.2}", bpe(decls as u64)),
                format!("{:.2}", bpe(nodes as u64)),
                format!("{:.2}", bpe(arcs as u64)),
                format!("{:.2}", bpe(inc as u64)),
                format!("{:.2}", bpe(inc as u64)), // levi ground == incidences
            ]
            .join("\t"),
        );
    }

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../reports/data/2026-09-03-p34-memory.tsv"
    );
    std::fs::write(path, rows.join("\n") + "\n").unwrap();
    println!("\nwrote {path}");
}
