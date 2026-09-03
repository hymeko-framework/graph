//! P3.3 §7 — rank-2 graphic-matroid benchmark. Measures, per (family, size): index build (context),
//! **spanning-forest computation** (reused workspace), the **weighted basis** (Kruskal sort-dominated),
//! the **incremental stream** cost, the resulting **rank / basis size**, and the DSU workspace's retained
//! bytes — with the arc and incidence counts for bytes-per-entity context. The facade itself is O(1)
//! (proven in `matroid_facade`); this file measures the Θ(ground-set) *computations* over it.
//!
//! Run for the numbers:
//!   cargo test -p hymeko_graph --release --test matroid_bench measure_matroid -- --ignored --nocapture
//! A TSV is written to `reports/data/2026-09-03-p33-matroid-latency.tsv`.

use std::fs;
use std::path::PathBuf;

use hymeko::common::ids::HyperArcId;
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::Ir;
use hymeko_graph::{
    GraphicMatroidWorkspace, IncrementalRank2, MatroidRef, rank2_spanning_forest,
    rank2_weighted_basis,
};

#[path = "informed_common/mod.rs"]
mod ic;
use ic::SignMode;

fn nnz(ir: &Ir) -> usize {
    ir.arcs.iter().map(|a| a.refs.len()).sum()
}

#[test]
#[ignore = "benchmark; run with --release --ignored --nocapture"]
fn measure_matroid_latency() {
    let cases: Vec<(String, Ir, bool)> = vec![
        (
            "path-1k".into(),
            ic::window_ir(1024, 2, true, SignMode::AllPlus),
            true,
        ),
        (
            "path-8k".into(),
            ic::window_ir(8192, 2, true, SignMode::AllPlus),
            true,
        ),
        (
            "path-65k".into(),
            ic::window_ir(65536, 2, true, SignMode::AllPlus),
            true,
        ),
        ("grid-32".into(), ic::grid_ir(32, 32), false),
        ("grid-128".into(), ic::grid_ir(128, 128), false),
        ("grid-256".into(), ic::grid_ir(256, 256), false),
        (
            "comps-8x1k".into(),
            ic::multi_component_ir(8, 1024, 2),
            false,
        ),
        ("dup-8k".into(), ic::dup_heavy_ir(8192, 2), false),
    ];

    let mut rows: Vec<String> = vec![
        "family\tnodes\tarcs\tincidences\trank\tidx_build_us\tforest_us\tweighted_us\tincr_us\tdsu_bytes"
            .into(),
    ];
    println!(
        "\n# P3.3 rank-2 matroid latency (release; median us over 15 iters, reused workspace)"
    );
    println!(
        "{:<12} {:>8} {:>8} {:>8} {:>8} {:>11} {:>10} {:>11} {:>9} {:>9}",
        "family",
        "nodes",
        "arcs",
        "inc",
        "rank",
        "idx_build",
        "forest",
        "weighted",
        "incr",
        "dsu_B"
    );

    let mut ws = GraphicMatroidWorkspace::new();
    let mut out = Vec::new();
    for (name, ir, weighted) in &cases {
        let nodes = ir.nodes.len();
        let arcs = ir.arcs.len();
        let inc = nnz(ir);

        let (_, idx_us, _) = ic::median_us(15, || {
            let _ = build_incidence_index(ir).unwrap();
        });
        let idx = build_incidence_index(ir).unwrap();
        let m = MatroidRef::new(ir, &idx);

        // warm the workspace, then measure the spanning-forest computation.
        let rank = rank2_spanning_forest(m, &mut ws, &mut out).unwrap();
        let (_, forest_us, _) = ic::median_us(15, || {
            let _ = rank2_spanning_forest(m, &mut ws, &mut out).unwrap();
        });

        // weighted basis (Kruskal). Unweighted graphs default all weights to 1.0 (still sorts).
        let (_, weighted_us, _) = ic::median_us(15, || {
            let _ = rank2_weighted_basis(m, true, &mut ws, &mut out).unwrap();
        });
        let _ = weighted;

        // incremental stream of every arc.
        let (_, incr_us, _) = ic::median_us(15, || {
            let mut stream = IncrementalRank2::begin(&mut ws, nodes).unwrap();
            for a in 0..arcs {
                let _ = stream.offer(ir, HyperArcId::new(a)).unwrap();
            }
        });

        let dsu_bytes = ws.retained_bytes();
        println!(
            "{name:<12} {nodes:>8} {arcs:>8} {inc:>8} {rank:>8} {idx_us:>11.2} {forest_us:>10.2} {weighted_us:>11.2} {incr_us:>9.2} {dsu_bytes:>9}"
        );
        rows.push(format!(
            "{name}\t{nodes}\t{arcs}\t{inc}\t{rank}\t{idx_us:.3}\t{forest_us:.3}\t{weighted_us:.3}\t{incr_us:.3}\t{dsu_bytes}"
        ));
    }

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../reports/data/2026-09-03-p33-matroid-latency.tsv");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, rows.join("\n") + "\n").unwrap();
    println!("\nwrote {}", path.display());
}
