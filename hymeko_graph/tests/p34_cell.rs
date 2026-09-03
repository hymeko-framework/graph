//! P3.4 §6 — one isolated CPU-measurement **cell**, driven by environment variables so a subprocess
//! harness can spawn it once per (workload, operation, thread-count). This binary carries **no** counting
//! allocator (that would perturb the timing); the PowerShell harness measures the whole process's
//! `TotalProcessorTime` and `PeakWorkingSet64`. Thread count is set by the parent via `RAYON_NUM_THREADS`
//! (only `batch` is Rayon-parallel — `par_batch_reachable_counts` reads the global pool; the other ops
//! are serial by construction, so their wall is thread-invariant, which the report states rather than
//! manufacturing a scaling story).
//!
//! Env: `P34_WORKLOAD` ∈ {sts511,sts1023,mycielski,random1e6,map512,transport}; `P34_OP` ∈
//! {reachable,batch,components,dijkstra,rank2,levi}; `P34_ITERS` (optional). It prints one line:
//! `P34CELL<TAB>key=val...` with `op_wall_s` (median) and `checksum`.

use std::env;

use hymeko::common::ids::NodeId;
use hymeko::ir::incidence::{IncidenceIndex, build_incidence_index};
use hymeko::ir::ir::Ir;
use hymeko_graph::direct_ir::{
    HyperTraversal, IncidenceWeightCost, InformedSearchWorkspace, Undirected, dijkstra_berge_path,
};
use hymeko_graph::{
    GraphicMatroidWorkspace, MatroidRef, TraversalWorkspace, connected_components,
    levi_spanning_basis, par_batch_reachable_counts, rank2_spanning_forest,
};

#[path = "informed_common/mod.rs"]
mod ic;

/// Returns `(median_op_wall_seconds, checksum)`. A `checksum` of `u64::MAX` marks an op that does not
/// apply to the workload (e.g. rank-2 on a multiway hypergraph), so the harness records `NA`.
fn run_op(op: &str, ir: &Ir, idx: &IncidenceIndex<'_>, iters: usize) -> (f64, u64) {
    let trav = HyperTraversal::new(idx);
    let n = ir.nodes.len();
    let mut checksum = 0u64;
    let (_, med_us, _) = match op {
        "reachable" => {
            let mut ws = TraversalWorkspace::new();
            ic::median_us(iters, || {
                checksum = trav.reachable_nodes(&mut ws, NodeId::new(0)).unwrap().len() as u64;
            })
        }
        "batch" => {
            let step = (n / 64).max(1);
            let starts: Vec<NodeId> = (0..n).step_by(step).take(64).map(NodeId::new).collect();
            ic::median_us(iters, || {
                let v = par_batch_reachable_counts(idx, &starts).unwrap();
                checksum = v.iter().map(|&x| x as u64).sum();
            })
        }
        "components" => {
            let mut ws = TraversalWorkspace::new();
            ic::median_us(iters, || {
                checksum = connected_components(idx, &mut ws).unwrap().count() as u64;
            })
        }
        "dijkstra" => {
            let cost = IncidenceWeightCost { base: 1.0 };
            let mut ws = InformedSearchWorkspace::new();
            let goal = NodeId::new(n.saturating_sub(1));
            let _ = dijkstra_berge_path(&trav, NodeId::new(0), goal, &cost, &Undirected, &mut ws);
            ic::median_us(iters, || {
                let p =
                    dijkstra_berge_path(&trav, NodeId::new(0), goal, &cost, &Undirected, &mut ws)
                        .unwrap();
                checksum = p.map(|x| x.steps.len() as u64).unwrap_or(0);
            })
        }
        "rank2" => {
            let mut ws = GraphicMatroidWorkspace::new();
            let mut out = Vec::new();
            if rank2_spanning_forest(MatroidRef::new(idx), &mut ws, &mut out).is_err() {
                checksum = u64::MAX; // NA: multiway hypergraph is not a rank-2 graph
                (0.0, 0.0, 0.0)
            } else {
                ic::median_us(iters, || {
                    checksum = rank2_spanning_forest(MatroidRef::new(idx), &mut ws, &mut out)
                        .unwrap() as u64;
                })
            }
        }
        "levi" => {
            let mut ws = GraphicMatroidWorkspace::new();
            let mut out = Vec::new();
            ic::median_us(iters, || {
                checksum =
                    levi_spanning_basis(MatroidRef::new(idx), &mut ws, &mut out).unwrap() as u64;
            })
        }
        other => panic!("unknown op {other}"),
    };
    (med_us / 1e6, checksum)
}

#[test]
#[ignore = "subprocess CPU cell; driven by P34_* env vars from scripts/p34_cpu_rss.ps1"]
fn run_cell() {
    let workload = env::var("P34_WORKLOAD").expect("P34_WORKLOAD");
    let op = env::var("P34_OP").expect("P34_OP");
    let default_iters = if op == "batch" { 3 } else { 5 };
    let iters = env::var("P34_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default_iters);

    let ir = ic::named_workload(&workload);
    let idx = build_incidence_index(&ir).unwrap();
    let inc: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
    let (op_wall_s, checksum) = run_op(&op, &ir, &idx, iters);
    let cs = if checksum == u64::MAX {
        "NA".to_string()
    } else {
        checksum.to_string()
    };
    println!(
        "P34CELL\tworkload={workload}\top={op}\tnodes={}\tarcs={}\tinc={inc}\titers={iters}\top_wall_s={op_wall_s:.6}\tchecksum={cs}",
        ir.nodes.len(),
        ir.arcs.len()
    );
}
