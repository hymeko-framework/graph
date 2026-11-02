//! Fresh-process evidence client for informed Berge search and bounded Berge enumeration.
//!
//! Production remains the only algorithm owner. This client builds deterministic fixtures, invokes
//! the public APIs, records full-result hashes outside timed boundaries and emits machine-readable
//! rows for `scripts/informed_berge_evidence/run.ps1`.

#[path = "informed_berge_evidence/execution.rs"]
mod execution;
#[path = "informed_berge_evidence/reporting.rs"]
mod reporting;
use execution::*;
#[path = "informed_berge_evidence/assisted.rs"]
mod assisted;
use reporting::{EvidenceCell, ResultSummary};

use hymeko_graph::direct_ir::{HopHeuristic, MaxHeuristic};
use std::collections::HashSet;
use std::env;
use workloads::MapPositions as Positions;
use workloads::search::{AStar, Dijkstra, SearchCase};

use hymeko::common::ids::{DeclId, NodeId};
use hymeko::ir::incidence::{IncidenceIndex, build_incidence_index};
use hymeko::ir::ir::Ir;
use hymeko_graph::direct_ir::{
    BergeCycle, BergeHeuristic, BergePath, BudgetKind, EnumBudget, HyperTraversal,
    IncidenceWeightCost, InformedSearchWorkspace, MetricHeuristic, SearchStats, TraversalWorkspace,
    Undirected, UnitCost, WeightedBergePath, enumerate_berge_cycles, enumerate_berge_paths,
};

#[path = "informed_common/parsing.rs"]
mod parsing;
#[allow(dead_code)]
#[path = "performance_evidence/statistics.rs"]
mod statistics;
#[path = "performance_evidence/telemetry.rs"]
mod telemetry;
#[path = "informed_common/mod.rs"]
mod workloads;

use statistics::{Measurement, measure_calibrated_verified};

fn fnv_mix(mut hash: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn hash_berge_path(path: &BergePath) -> u64 {
    let mut hash = fnv_mix(0xcbf2_9ce4_8422_2325, b"HYMEKO-BERGE-PATH-EVIDENCE-v1");
    hash = fnv_mix(hash, &(path.start.raw() as u64).to_le_bytes());
    for step in &path.steps {
        hash = fnv_mix(hash, &(step.arc.raw() as u64).to_le_bytes());
        // The same ARRIVAL quantities this evidence hash has always mixed; only the accessor path
        // changed when the step gained its entry coordinate.
        hash = fnv_mix(hash, &step.to.slot.to_le_bytes());
        hash = fnv_mix(hash, &i64::from(step.to.sign).to_le_bytes());
        hash = fnv_mix(hash, &(step.to.node.raw() as u64).to_le_bytes());
    }
    hash
}

fn bound_key(bound: Option<BudgetKind>) -> &'static str {
    match bound {
        None => "none",
        Some(BudgetKind::Depth) => "depth",
        Some(BudgetKind::Results) => "results",
        Some(BudgetKind::Expansions) => "expansions",
    }
}

fn hash_paths(items: &[BergePath], bound: Option<BudgetKind>) -> u64 {
    let mut hash = fnv_mix(0xcbf2_9ce4_8422_2325, b"HYMEKO-BERGE-PATH-SET-v1");
    hash = fnv_mix(hash, bound_key(bound).as_bytes());
    hash = fnv_mix(hash, &(items.len() as u64).to_le_bytes());
    for path in items {
        hash = fnv_mix(hash, &hash_berge_path(path).to_le_bytes());
    }
    hash
}

fn hash_cycles(items: &[BergeCycle], bound: Option<BudgetKind>) -> u64 {
    let mut hash = fnv_mix(0xcbf2_9ce4_8422_2325, b"HYMEKO-BERGE-CYCLE-SET-v1");
    hash = fnv_mix(hash, bound_key(bound).as_bytes());
    hash = fnv_mix(hash, &(items.len() as u64).to_le_bytes());
    for cycle in items {
        hash = fnv_mix(hash, &(cycle.nodes.len() as u64).to_le_bytes());
        for node in &cycle.nodes {
            hash = fnv_mix(hash, &(node.raw() as u64).to_le_bytes());
        }
        for arc in &cycle.arcs {
            hash = fnv_mix(hash, &(arc.raw() as u64).to_le_bytes());
        }
    }
    hash
}

struct ManhattanHeuristic<'a> {
    positions: &'a Positions,
    goal: [f64; 2],
}

impl BergeHeuristic for ManhattanHeuristic<'_> {
    const ADMISSIBLE: bool = true;
    const CONSISTENT: bool = true;

    fn h(&self, _ir: &Ir, node: DeclId) -> f64 {
        self.positions
            .0
            .get(node.raw())
            .and_then(|p| *p)
            .map_or(0.0, |p| {
                (p[0] - self.goal[0]).abs() + (p[1] - self.goal[1]).abs()
            })
    }
}

fn search_fixture(name: &str) -> (Ir, NodeId, NodeId) {
    match name {
        "grid200_d20" | "grid200_d40" | "grid200_d80" => {
            let distance = name
                .strip_prefix("grid200_d")
                .and_then(|value| value.parse::<usize>().ok())
                .expect("grid distance");
            let side = 200;
            let row = side / 2;
            let column = side / 4;
            (
                workloads::grid_ir(side, side),
                NodeId::new(row * side + column),
                NodeId::new(row * side + column + distance),
            )
        }
        "grid512_d511" => (
            workloads::grid_ir(512, 512),
            NodeId::new(256 * 512),
            NodeId::new(256 * 512 + 511),
        ),
        "map32" | "map128" | "map256" | "map512" => {
            let side = name
                .strip_prefix("map")
                .and_then(|value| value.parse::<usize>().ok())
                .expect("map side");
            let ir = workloads::map_grid_ir(side, side, &workloads::map_obstacles(side, side));
            let goal = NodeId::new(ir.nodes.len() - 1);
            (ir, NodeId::new(0), goal)
        }
        "corridor128" => {
            let ir = workloads::corridor_map_ir(128, 128);
            let goal = NodeId::new(ir.nodes.len() - 1);
            (ir, NodeId::new(0), goal)
        }
        "transport128" => {
            let ir = workloads::map_grid_with_transport_ir(128, 128, &HashSet::new(), 16, 0.5);
            let goal = NodeId::new(ir.nodes.len() - 1);
            (ir, NodeId::new(0), goal)
        }
        "transport256_off" => {
            let ir = workloads::map_grid_with_transport_ir(256, 256, &HashSet::new(), 32, 0.5);
            let goal = NodeId::new(ir.nodes.len() - 2);
            (ir, NodeId::new(1), goal)
        }
        "tour8" | "tour16" | "tour24" | "tour32" | "tour64" => {
            let (width, height, stations) = match name {
                "tour8" => (4, 2, 4),
                "tour16" => (4, 4, 4),
                "tour24" => (4, 6, 6),
                "tour32" => (4, 8, 8),
                "tour64" => (8, 8, 8),
                _ => unreachable!(),
            };
            let ir = workloads::map_grid_with_transport_ir(
                width,
                height,
                &HashSet::new(),
                stations,
                0.5,
            );
            let goal = NodeId::new(ir.nodes.len() - 1);
            (ir, NodeId::new(0), goal)
        }
        "tour64_corridor" => {
            let mut ir = workloads::corridor_map_ir(8, 8);
            let refs = [0, 21, 42, 63]
                .into_iter()
                .map(|v| workloads::plus_ref(ir.nodes[v].decl, Some(0.5)))
                .collect();
            workloads::push_arc(&mut ir, refs);
            (ir, NodeId::new(0), NodeId::new(63))
        }
        other => panic!("unknown informed-search workload {other}"),
    }
}

#[test]
#[ignore = "fresh-process informed-search evidence; driven by scripts/informed_berge_evidence/run.ps1"]
fn search_cell() {
    let run = env::var("INFORMED_BERGE_RUN").unwrap_or_else(|_| "manual".to_string());
    let workload = env::var("INFORMED_BERGE_WORKLOAD").expect("INFORMED_BERGE_WORKLOAD");
    let subject = env::var("INFORMED_BERGE_SUBJECT").expect("INFORMED_BERGE_SUBJECT");
    let cell = EvidenceCell {
        run: &run,
        section: "informed_search",
        workload: &workload,
        subject: &subject,
    };
    cell.phase("process_baseline");
    let (ir, start, goal) = search_fixture(&workload);
    cell.phase("canonical_ir");
    let index = build_incidence_index(&ir).expect("incidence index");
    cell.phase("incidence_index");
    if workload.starts_with("grid200_d") {
        let positions = Positions::from_grid(200, 200);
        let goal_decl = ir.nodes[goal.raw()].decl;
        match subject.as_str() {
            "dijkstra_unit" => {
                cell.weighted(&index, start, goal, Dijkstra::new(&UnitCost, &Undirected))
            }
            "astar_l1" => cell.weighted(
                &index,
                start,
                goal,
                AStar::new(
                    &UnitCost,
                    &ManhattanHeuristic {
                        positions: &positions,
                        goal: positions.0[goal_decl.raw()].unwrap(),
                    },
                    &Undirected,
                ),
            ),
            "astar_l2" => cell.weighted(
                &index,
                start,
                goal,
                AStar::new(
                    &UnitCost,
                    &MetricHeuristic::new(&positions, goal_decl),
                    &Undirected,
                ),
            ),
            other => panic!("invalid grid search subject {other}"),
        }
    } else {
        let cost = IncidenceWeightCost { base: 0.0 };
        let positions = Positions::from_ir(&ir);
        let goal_decl = ir.nodes[goal.raw()].decl;
        let hops = workloads::hop_distances_from(&ir, &index, goal);
        let discrete = HopHeuristic::new(&hops, workloads::map_min_edge_cost(&ir));
        let metric = MetricHeuristic::new(&positions, goal_decl);
        let combined = MaxHeuristic::new(&metric, &discrete);
        match subject.as_str() {
            "dijkstra_weight" => {
                cell.weighted(&index, start, goal, Dijkstra::new(&cost, &Undirected))
            }
            "astar_metric" if workload != "transport128" => {
                cell.weighted(&index, start, goal, AStar::new(&cost, &metric, &Undirected))
            }
            "astar_discrete" => cell.weighted(
                &index,
                start,
                goal,
                AStar::new(&cost, &discrete, &Undirected),
            ),
            "astar_combined" if workload != "transport128" => cell.weighted(
                &index,
                start,
                goal,
                AStar::new(&cost, &combined, &Undirected),
            ),
            other => panic!("invalid weighted-map subject {other}"),
        }
    }
    cell.phase("measured");
}

fn hash_hops(hops: &[u32]) -> u64 {
    let mut hash = fnv_mix(0xcbf2_9ce4_8422_2325, b"HYMEKO-DISCRETE-HOPS-v1");
    for hop in hops {
        hash = fnv_mix(hash, &hop.to_le_bytes());
    }
    hash
}

#[test]
#[ignore = "fresh-process heuristic-preparation evidence; driven by scripts/informed_berge_evidence/run.ps1"]
fn preparation_cell() {
    let run = env::var("INFORMED_BERGE_RUN").unwrap_or_else(|_| "manual".to_string());
    let workload = env::var("INFORMED_BERGE_WORKLOAD").expect("INFORMED_BERGE_WORKLOAD");
    let cell = EvidenceCell {
        run: &run,
        section: "heuristic_preparation",
        workload: &workload,
        subject: "discrete_hops",
    };
    cell.phase("process_baseline");
    let (ir, _, goal) = search_fixture(&workload);
    cell.phase("canonical_ir");
    let index = build_incidence_index(&ir).expect("incidence index");
    cell.phase("incidence_index");
    let mut operation = HopPreparation {
        index: &index,
        goal,
    };
    let run = VerifiedRun::collect(&mut operation, &Calibrated);
    cell.measurement(&ir, &run.measurement);
    cell.result(&ResultSummary::Items {
        count: run.result.len(),
        bound: None,
        budget: None,
        hash: run.hash,
    });
    cell.phase("measured");
}

fn shortest_fixture(name: &str) -> Ir {
    match name {
        "path65k" => workloads::window_ir(65_536, 2, true, workloads::SignMode::AllPlus),
        "grid256" => workloads::grid_ir(256, 256),
        "sts1023" => workloads::steiner_sts_ir(10),
        "random1e6" => workloads::uniform_rank_k_ir(0x51EED, 200_000, 333_333, 3),
        "map512" => workloads::map_grid_ir(512, 512, &workloads::map_obstacles(512, 512)),
        other => panic!("unknown shortest-path workload {other}"),
    }
}

#[test]
#[ignore = "fresh-process Berge-shortest evidence; driven by scripts/informed_berge_evidence/run.ps1"]
fn shortest_cell() {
    let run = env::var("INFORMED_BERGE_RUN").unwrap_or_else(|_| "manual".to_string());
    let workload = env::var("INFORMED_BERGE_WORKLOAD").expect("INFORMED_BERGE_WORKLOAD");
    let subject = env::var("INFORMED_BERGE_SUBJECT").expect("INFORMED_BERGE_SUBJECT");
    let cell = EvidenceCell {
        run: &run,
        section: "berge_shortest",
        workload: &workload,
        subject: &subject,
    };
    cell.phase("process_baseline");
    let ir = shortest_fixture(&workload);
    cell.phase("canonical_ir");
    let index = build_incidence_index(&ir).expect("incidence index");
    cell.phase("incidence_index");
    let start = NodeId::new(0);
    let goal = NodeId::new(ir.nodes.len() - 1);
    let traversal = HyperTraversal::new(&index);
    match subject.as_str() {
        "bfs_shortest" => {
            let mut operation = ShortestQuery {
                traversal: &traversal,
                start,
                goal,
                workspace: TraversalWorkspace::new(),
            };
            cell.phase("prepared");
            let run = VerifiedRun::collect(&mut operation, &Calibrated);
            cell.measurement(&ir, &run.measurement);
            cell.result(&ResultSummary::shortest(&run.result, run.hash));
        }
        "dijkstra_unit" => {
            cell.weighted(&index, start, goal, Dijkstra::new(&UnitCost, &Undirected))
        }
        other => panic!("invalid shortest subject {other}"),
    }
    cell.phase("measured");
}

fn layered_ir(layers: usize) -> Ir {
    let goal = 1 + 2 * layers;
    let mut edges = Vec::new();
    for node in [1, 2] {
        edges.push((0, node, 1.0));
    }
    for layer in 0..layers - 1 {
        let from = 1 + 2 * layer;
        let to = 1 + 2 * (layer + 1);
        for a in [from, from + 1] {
            for b in [to, to + 1] {
                edges.push((a, b, 1.0));
            }
        }
    }
    let last = 1 + 2 * (layers - 1);
    for node in [last, last + 1] {
        edges.push((node, goal, 1.0));
    }
    workloads::weighted_graph_ir(goal + 1, &edges)
}

fn parallel_ir(count: usize) -> Ir {
    workloads::weighted_graph_ir(2, &vec![(0, 1, 1.0); count])
}

fn enumeration_fixture(name: &str) -> (Ir, NodeId, Option<NodeId>, EnumBudget) {
    match name {
        "layered10" => {
            let ir = layered_ir(10);
            let goal = NodeId::new(ir.nodes.len() - 1);
            (
                ir,
                NodeId::new(0),
                Some(goal),
                EnumBudget::new(11, 2_048, 2_000_000),
            )
        }
        "grid6" => (
            workloads::grid_ir(6, 6),
            NodeId::new(0),
            Some(NodeId::new(35)),
            EnumBudget::new(10, 1_000, 2_000_000),
        ),
        "grid5" => (
            workloads::grid_ir(5, 5),
            NodeId::new(12),
            None,
            EnumBudget::new(8, 2_000, 500_000),
        ),
        "sts31" => (
            workloads::steiner_sts_ir(5),
            NodeId::new(0),
            None,
            EnumBudget::new(4, 4_096, 500_000),
        ),
        "parallel16" => (
            parallel_ir(16),
            NodeId::new(0),
            None,
            EnumBudget::new(2, 1_000, 100_000),
        ),
        other => panic!("unknown enumeration workload {other}"),
    }
}

#[test]
#[ignore = "fresh-process bounded-enumeration evidence; driven by scripts/informed_berge_evidence/run.ps1"]
fn enumeration_cell() {
    let run = env::var("INFORMED_BERGE_RUN").unwrap_or_else(|_| "manual".to_string());
    let workload = env::var("INFORMED_BERGE_WORKLOAD").expect("INFORMED_BERGE_WORKLOAD");
    let subject = env::var("INFORMED_BERGE_SUBJECT").expect("INFORMED_BERGE_SUBJECT");
    let cell = EvidenceCell {
        run: &run,
        section: "berge_enumeration",
        workload: &workload,
        subject: &subject,
    };
    cell.phase("process_baseline");
    let (ir, start, goal, budget) = enumeration_fixture(&workload);
    cell.phase("canonical_ir");
    let index = build_incidence_index(&ir).expect("incidence index");
    cell.phase("incidence_index");
    cell.phase("prepared");
    match subject.as_str() {
        "paths_goal" => cell.enumeration(&index, start, budget, Paths(goal)),
        "paths_all" => cell.enumeration(&index, start, budget, Paths(None)),
        "cycles" => cell.enumeration(&index, start, budget, Cycles),
        other => panic!("invalid enumeration subject {other}"),
    }
    cell.phase("measured");
}

#[test]
fn informed_search_evidence_subjects_agree() {
    let ir = workloads::grid_ir(32, 32);
    let index = build_incidence_index(&ir).unwrap();
    let traversal = HyperTraversal::new(&index);
    let positions = Positions::from_grid(32, 32);
    let start = NodeId::new(16 * 32 + 4);
    let goal = NodeId::new(16 * 32 + 20);
    let goal_decl = ir.nodes[goal.raw()].decl;
    let l1 = ManhattanHeuristic {
        positions: &positions,
        goal: positions.0[goal_decl.raw()].unwrap(),
    };
    let l2 = MetricHeuristic::new(&positions, goal_decl);
    let mut workspace = InformedSearchWorkspace::new();
    let mut case = SearchCase::new(&traversal, start, goal, &mut workspace);
    let dijkstra = case
        .run(&Dijkstra::new(&UnitCost, &Undirected))
        .unwrap()
        .unwrap();
    let a1 = case
        .run(&AStar::new(&UnitCost, &l1, &Undirected))
        .unwrap()
        .unwrap();
    assert_eq!(case.stats().reopened, 0);
    let a2 = case
        .run(&AStar::new(&UnitCost, &l2, &Undirected))
        .unwrap()
        .unwrap();
    assert_eq!(case.stats().reopened, 0);
    assert_eq!(dijkstra.total_cost, a1.total_cost);
    assert_eq!(dijkstra.total_cost, a2.total_cost);

    assert_eq!(dijkstra.path_hash().raw(), 0x67d1f5421918527c);
    assert_eq!(a1.path_hash(), dijkstra.path_hash());
    assert_eq!(a2.path_hash(), dijkstra.path_hash());
}

#[test]
fn enumeration_evidence_fixtures_pin_counts_and_hashes() {
    let layered = layered_ir(4);
    let layered_index = build_incidence_index(&layered).unwrap();
    let paths = enumerate_berge_paths(
        &layered_index,
        NodeId::new(0),
        Some(NodeId::new(layered.nodes.len() - 1)),
        EnumBudget::new(5, 100, 10_000),
    )
    .unwrap();
    assert_eq!(paths.items.len(), 16);
    assert_eq!(paths.bound, Some(BudgetKind::Depth));
    let paths_again = enumerate_berge_paths(
        &layered_index,
        NodeId::new(0),
        Some(NodeId::new(layered.nodes.len() - 1)),
        EnumBudget::new(5, 100, 10_000),
    )
    .unwrap();
    assert_eq!(
        hash_paths(&paths.items, paths.bound),
        hash_paths(&paths_again.items, paths_again.bound)
    );

    let parallel = parallel_ir(6);
    let parallel_index = build_incidence_index(&parallel).unwrap();
    let cycles = enumerate_berge_cycles(
        &parallel_index,
        NodeId::new(0),
        EnumBudget::new(2, 100, 10_000),
    )
    .unwrap();
    assert_eq!(cycles.items.len(), 15);
    assert_eq!(cycles.bound, None);
    let cycles_again = enumerate_berge_cycles(
        &parallel_index,
        NodeId::new(0),
        EnumBudget::new(2, 100, 10_000),
    )
    .unwrap();
    assert_eq!(
        hash_cycles(&cycles.items, cycles.bound),
        hash_cycles(&cycles_again.items, cycles_again.bound)
    );

    assert_eq!(hash_paths(&paths.items, paths.bound), 0x5e30e2bae0258b4a);
    assert_eq!(hash_cycles(&cycles.items, cycles.bound), 0xa70363a1887c28e5);
}
