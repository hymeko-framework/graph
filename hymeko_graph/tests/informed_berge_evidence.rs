//! Fresh-process evidence client for informed Berge search and bounded Berge enumeration.
//!
//! Production remains the only algorithm owner. This client builds deterministic fixtures, invokes
//! the public APIs, records full-result hashes outside timed boundaries and emits machine-readable
//! rows for `scripts/informed_berge_evidence/run.ps1`.

use std::collections::HashSet;
use std::env;

use hymeko::common::ids::{DeclId, NodeId};
use hymeko::ir::incidence::{IncidenceIndex, build_incidence_index};
use hymeko::ir::ir::Ir;
use hymeko_graph::direct_ir::{
    BergeCycle, BergeHeuristic, BergePath, BudgetKind, EnumBudget, HyperTraversal,
    IncidenceWeightCost, InformedSearchWorkspace, MetricHeuristic, PositionProvider, SearchStats,
    TraversalWorkspace, Undirected, UnitCost, WeightedBergePath, astar_berge_path,
    dijkstra_berge_path, enumerate_berge_cycles, enumerate_berge_paths,
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

fn dimensions(ir: &Ir) -> (usize, usize, usize) {
    (
        ir.nodes.len(),
        ir.arcs.len(),
        ir.arcs.iter().map(|arc| arc.refs.len()).sum(),
    )
}

fn optional(value: Option<f64>) -> String {
    value.map_or_else(|| "NA".to_string(), |value| format!("{value:.3}"))
}

fn emit_phase(run: &str, workload: &str, subject: &str, phase: &str) {
    match telemetry::memory_snapshot() {
        Some(memory) => println!(
            "EVIDENCE_PHASE\trun={run}\tworkload={workload}\tsubject={subject}\tphase={phase}\tworking_set_b={}\tself_peak_b={}",
            memory.working_set_bytes, memory.peak_working_set_bytes
        ),
        None => println!(
            "EVIDENCE_PHASE\trun={run}\tworkload={workload}\tsubject={subject}\tphase={phase}\tworking_set_b=NA\tself_peak_b=NA"
        ),
    }
}

fn emit_measurement(
    run: &str,
    section: &str,
    workload: &str,
    subject: &str,
    ir: &Ir,
    measurement: &Measurement,
) {
    let (nodes, arcs, incidences) = dimensions(ir);
    for sample in &measurement.samples {
        println!(
            "EVIDENCE_SAMPLE\trun={run}\tsection={section}\tworkload={workload}\tsubject={subject}\tordinal={}\trepetitions={}\twall_us={:.3}\tcpu_us={}\tchecksum={}",
            sample.ordinal,
            measurement.repetitions_per_sample,
            sample.wall_us,
            optional(sample.cpu_us),
            sample.checksum
        );
    }
    println!(
        "EVIDENCE_CELL\trun={run}\tsection={section}\tworkload={workload}\tsubject={subject}\tstatus=OK\tnodes={nodes}\tarcs={arcs}\tincidences={incidences}\twarmups={}\tsamples={}\trepetitions={}\twall_median_us={:.3}\twall_q1_us={:.3}\twall_q3_us={:.3}\twall_worst_us={:.3}\tcpu_median_us={}\tcpu_q1_us={}\tcpu_q3_us={}\tcpu_worst_us={}\tchecksum={}",
        measurement.warmups,
        measurement.samples.len(),
        measurement.repetitions_per_sample,
        measurement.wall.median,
        measurement.wall.q1,
        measurement.wall.q3,
        measurement.wall.worst,
        optional(measurement.cpu.map(|distribution| distribution.median)),
        optional(measurement.cpu.map(|distribution| distribution.q1)),
        optional(measurement.cpu.map(|distribution| distribution.q3)),
        optional(measurement.cpu.map(|distribution| distribution.worst)),
        measurement.checksum
    );
}

fn emit_work(run: &str, workload: &str, subject: &str, stats: SearchStats) {
    println!(
        "EVIDENCE_WORK\trun={run}\tworkload={workload}\tsubject={subject}\texpanded_nodes={}\texpanded_arcs={}\tresolved_incidences={}\tpushes={}\tpops={}\tstale_pops={}\theuristic_evals={}\treopened={}",
        stats.expanded_nodes,
        stats.expanded_arcs,
        stats.resolved_incidences,
        stats.pushes,
        stats.pops,
        stats.stale_pops,
        stats.heuristic_evals,
        stats.reopened
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_result(
    run: &str,
    section: &str,
    workload: &str,
    subject: &str,
    reachable: &str,
    cost_micros: &str,
    hops: &str,
    path_hash: u64,
    items: &str,
    bound: &str,
    budget: Option<EnumBudget>,
) {
    let (max_depth, max_results, max_expansions) = budget.map_or_else(
        || ("NA".to_string(), "NA".to_string(), "NA".to_string()),
        |b| {
            (
                b.max_depth.to_string(),
                b.max_results.to_string(),
                b.max_expansions.to_string(),
            )
        },
    );
    println!(
        "EVIDENCE_RESULT\trun={run}\tsection={section}\tworkload={workload}\tsubject={subject}\treachable={reachable}\tcost_micros={cost_micros}\thops={hops}\tpath_hash={path_hash}\titems={items}\tbound={bound}\tmax_depth={max_depth}\tmax_results={max_results}\tmax_expansions={max_expansions}"
    );
}

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
        hash = fnv_mix(hash, &step.slot.to_le_bytes());
        hash = fnv_mix(hash, &i64::from(step.sign).to_le_bytes());
        hash = fnv_mix(hash, &(step.node.raw() as u64).to_le_bytes());
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

struct Positions(Vec<Option<[f64; 2]>>);

impl Positions {
    fn from_ir(ir: &Ir) -> Self {
        Self(
            (0..ir.decl_nodes.len())
                .map(|raw| workloads::ir_position(ir, DeclId::new(raw)))
                .collect(),
        )
    }

    fn from_grid(width: usize, height: usize) -> Self {
        let positions = workloads::grid_positions(width, height)
            .into_iter()
            .map(Some)
            .collect();
        Self(positions)
    }
}

impl PositionProvider for Positions {
    fn position(&self, node: DeclId) -> Option<&[f64]> {
        self.0
            .get(node.raw())
            .and_then(Option::as_ref)
            .map(|position| position.as_slice())
    }
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

struct DiscreteHeuristic {
    hops: Vec<u32>,
    min_cost: f64,
}

impl BergeHeuristic for DiscreteHeuristic {
    const ADMISSIBLE: bool = true;
    const CONSISTENT: bool = true;

    fn h(&self, _ir: &Ir, node: DeclId) -> f64 {
        self.hops.get(node.raw()).copied().map_or(0.0, |hops| {
            if hops == u32::MAX {
                0.0
            } else {
                f64::from(hops) * self.min_cost
            }
        })
    }
}

struct CombinedHeuristic<'a> {
    metric: MetricHeuristic<'a, Positions>,
    discrete: &'a DiscreteHeuristic,
}

impl BergeHeuristic for CombinedHeuristic<'_> {
    const ADMISSIBLE: bool = true;
    const CONSISTENT: bool = true;

    fn h(&self, ir: &Ir, node: DeclId) -> f64 {
        self.metric.h(ir, node).max(self.discrete.h(ir, node))
    }
}

fn measure_weighted_query(
    mut query: impl FnMut(&mut InformedSearchWorkspace) -> Option<WeightedBergePath>,
) -> (Measurement, SearchStats, Option<WeightedBergePath>) {
    let mut workspace = InformedSearchWorkspace::new();
    let (expected, measurement) = {
        let mut measured = |strong: bool| {
            query(&mut workspace).map_or(0, |path| {
                if strong {
                    path.path_hash()
                } else {
                    (1_u64 << 63) ^ path.hyperedge_len() as u64
                }
            })
        };
        let expected = measured(true);
        let measurement = measure_calibrated_verified(&mut measured);
        (expected, measurement)
    };
    assert_eq!(measurement.checksum, expected);

    let measured_stats = workspace.stats();
    let result = query(&mut workspace);
    assert_eq!(workspace.stats(), measured_stats, "search work drift");
    let result_hash = result.as_ref().map_or(0, WeightedBergePath::path_hash);
    assert_eq!(result_hash, expected, "weighted path hash drift");
    (measurement, measured_stats, result)
}

#[allow(clippy::too_many_arguments)]
fn emit_weighted_search<C, H>(
    run: &str,
    section: &str,
    workload: &str,
    subject: &str,
    ir: &Ir,
    index: &IncidenceIndex<'_>,
    start: NodeId,
    goal: NodeId,
    cost: &C,
    heuristic: &H,
) where
    C: hymeko_graph::direct_ir::BergeCost,
    H: BergeHeuristic,
{
    let traversal = HyperTraversal::new(index);
    emit_phase(run, workload, subject, "prepared");
    let (measurement, stats, result) = measure_weighted_query(|workspace| {
        astar_berge_path(
            &traversal,
            start,
            goal,
            cost,
            heuristic,
            &Undirected,
            workspace,
        )
        .expect("A* evidence query")
    });
    emit_measurement(run, section, workload, subject, ir, &measurement);
    emit_work(run, workload, subject, stats);
    let reachable = result.is_some().to_string();
    let cost_micros = result.as_ref().map_or_else(
        || "NA".to_string(),
        |path| format!("{:.0}", path.total_cost * 1_000_000.0),
    );
    let hops = result
        .as_ref()
        .map_or_else(|| "NA".to_string(), |path| path.hyperedge_len().to_string());
    let path_hash = result.as_ref().map_or(0, WeightedBergePath::path_hash);
    emit_result(
        run,
        section,
        workload,
        subject,
        &reachable,
        &cost_micros,
        &hops,
        path_hash,
        "NA",
        "NA",
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_dijkstra<C: hymeko_graph::direct_ir::BergeCost>(
    run: &str,
    section: &str,
    workload: &str,
    subject: &str,
    ir: &Ir,
    index: &IncidenceIndex<'_>,
    start: NodeId,
    goal: NodeId,
    cost: &C,
) {
    let traversal = HyperTraversal::new(index);
    emit_phase(run, workload, subject, "prepared");
    let (measurement, stats, result) = measure_weighted_query(|workspace| {
        dijkstra_berge_path(&traversal, start, goal, cost, &Undirected, workspace)
            .expect("Dijkstra evidence query")
    });
    emit_measurement(run, section, workload, subject, ir, &measurement);
    emit_work(run, workload, subject, stats);
    let reachable = result.is_some().to_string();
    let cost_micros = result.as_ref().map_or_else(
        || "NA".to_string(),
        |path| format!("{:.0}", path.total_cost * 1_000_000.0),
    );
    let hops = result
        .as_ref()
        .map_or_else(|| "NA".to_string(), |path| path.hyperedge_len().to_string());
    let path_hash = result.as_ref().map_or(0, WeightedBergePath::path_hash);
    emit_result(
        run,
        section,
        workload,
        subject,
        &reachable,
        &cost_micros,
        &hops,
        path_hash,
        "NA",
        "NA",
        None,
    );
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
        "map32" | "map128" | "map512" => {
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
        other => panic!("unknown informed-search workload {other}"),
    }
}

#[test]
#[ignore = "fresh-process informed-search evidence; driven by scripts/informed_berge_evidence/run.ps1"]
fn search_cell() {
    let run = env::var("INFORMED_BERGE_RUN").unwrap_or_else(|_| "manual".to_string());
    let workload = env::var("INFORMED_BERGE_WORKLOAD").expect("INFORMED_BERGE_WORKLOAD");
    let subject = env::var("INFORMED_BERGE_SUBJECT").expect("INFORMED_BERGE_SUBJECT");
    emit_phase(&run, &workload, &subject, "process_baseline");
    let (ir, start, goal) = search_fixture(&workload);
    emit_phase(&run, &workload, &subject, "canonical_ir");
    let index = build_incidence_index(&ir).expect("incidence index");
    emit_phase(&run, &workload, &subject, "incidence_index");

    if workload.starts_with("grid200_") {
        let positions = Positions::from_grid(200, 200);
        let goal_decl = ir.nodes[goal.raw()].decl;
        let goal_position = positions.0[goal_decl.raw()].expect("goal position");
        match subject.as_str() {
            "dijkstra_unit" => emit_dijkstra(
                &run,
                "informed_search",
                &workload,
                &subject,
                &ir,
                &index,
                start,
                goal,
                &UnitCost,
            ),
            "astar_l1" => emit_weighted_search(
                &run,
                "informed_search",
                &workload,
                &subject,
                &ir,
                &index,
                start,
                goal,
                &UnitCost,
                &ManhattanHeuristic {
                    positions: &positions,
                    goal: goal_position,
                },
            ),
            "astar_l2" => emit_weighted_search(
                &run,
                "informed_search",
                &workload,
                &subject,
                &ir,
                &index,
                start,
                goal,
                &UnitCost,
                &MetricHeuristic::new(&positions, goal_decl),
            ),
            other => panic!("invalid grid subject {other}"),
        }
    } else {
        let cost = IncidenceWeightCost { base: 0.0 };
        let positions = Positions::from_ir(&ir);
        let goal_decl = ir.nodes[goal.raw()].decl;
        let hops = workloads::hop_distances_from(&ir, &index, goal);
        let discrete = DiscreteHeuristic {
            hops,
            min_cost: workloads::map_min_edge_cost(&ir),
        };
        let metric = MetricHeuristic::new(&positions, goal_decl);
        let combined = CombinedHeuristic {
            metric: MetricHeuristic::new(&positions, goal_decl),
            discrete: &discrete,
        };
        match subject.as_str() {
            "dijkstra_weight" => emit_dijkstra(
                &run,
                "informed_search",
                &workload,
                &subject,
                &ir,
                &index,
                start,
                goal,
                &cost,
            ),
            "astar_metric" if workload != "transport128" => emit_weighted_search(
                &run,
                "informed_search",
                &workload,
                &subject,
                &ir,
                &index,
                start,
                goal,
                &cost,
                &metric,
            ),
            "astar_discrete" => emit_weighted_search(
                &run,
                "informed_search",
                &workload,
                &subject,
                &ir,
                &index,
                start,
                goal,
                &cost,
                &discrete,
            ),
            "astar_combined" if workload != "transport128" => emit_weighted_search(
                &run,
                "informed_search",
                &workload,
                &subject,
                &ir,
                &index,
                start,
                goal,
                &cost,
                &combined,
            ),
            other => panic!("invalid weighted-map subject {other}"),
        }
    }
    emit_phase(&run, &workload, &subject, "measured");
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
    let subject = "discrete_hops";
    emit_phase(&run, &workload, subject, "process_baseline");
    let (ir, _, goal) = search_fixture(&workload);
    emit_phase(&run, &workload, subject, "canonical_ir");
    let index = build_incidence_index(&ir).expect("incidence index");
    emit_phase(&run, &workload, subject, "incidence_index");
    let mut prepare = |strong: bool| {
        let hops = workloads::hop_distances_from(&ir, &index, goal);
        if strong {
            hash_hops(&hops)
        } else {
            ((hops.len() as u64) << 32) ^ hops.last().copied().map_or(u64::MAX, u64::from)
        }
    };
    let expected = prepare(true);
    let measurement = measure_calibrated_verified(&mut prepare);
    let hops = workloads::hop_distances_from(&ir, &index, goal);
    let checksum = hash_hops(&hops);
    assert_eq!(expected, checksum);
    assert_eq!(measurement.checksum, checksum);
    emit_measurement(
        &run,
        "heuristic_preparation",
        &workload,
        subject,
        &ir,
        &measurement,
    );
    emit_result(
        &run,
        "heuristic_preparation",
        &workload,
        subject,
        "NA",
        "NA",
        "NA",
        checksum,
        &hops.len().to_string(),
        "none",
        None,
    );
    emit_phase(&run, &workload, subject, "measured");
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
    emit_phase(&run, &workload, &subject, "process_baseline");
    let ir = shortest_fixture(&workload);
    emit_phase(&run, &workload, &subject, "canonical_ir");
    let index = build_incidence_index(&ir).expect("incidence index");
    emit_phase(&run, &workload, &subject, "incidence_index");
    let start = NodeId::new(0);
    let goal = NodeId::new(ir.nodes.len() - 1);
    let traversal = HyperTraversal::new(&index);

    match subject.as_str() {
        "bfs_shortest" => {
            let mut workspace = TraversalWorkspace::new();
            emit_phase(&run, &workload, &subject, "prepared");
            let (expected, measurement) = {
                let mut query = |strong: bool| {
                    traversal
                        .berge_shortest_path(&mut workspace, start, goal)
                        .expect("BFS Berge shortest")
                        .map_or(0, |path| {
                            if strong {
                                hash_berge_path(&path)
                            } else {
                                (1_u64 << 63) ^ path.hyperedge_len() as u64
                            }
                        })
                };
                let expected = query(true);
                let measurement = measure_calibrated_verified(&mut query);
                (expected, measurement)
            };
            assert_eq!(measurement.checksum, expected);
            let result = traversal
                .berge_shortest_path(&mut workspace, start, goal)
                .expect("BFS Berge result");
            assert_eq!(result.as_ref().map_or(0, hash_berge_path), expected);
            emit_measurement(
                &run,
                "berge_shortest",
                &workload,
                &subject,
                &ir,
                &measurement,
            );
            emit_result(
                &run,
                "berge_shortest",
                &workload,
                &subject,
                &result.is_some().to_string(),
                "NA",
                &result
                    .as_ref()
                    .map_or_else(|| "NA".to_string(), |path| path.hyperedge_len().to_string()),
                expected,
                "NA",
                "NA",
                None,
            );
        }
        "dijkstra_unit" => emit_dijkstra(
            &run,
            "berge_shortest",
            &workload,
            &subject,
            &ir,
            &index,
            start,
            goal,
            &UnitCost,
        ),
        other => panic!("invalid shortest-path subject {other}"),
    }
    emit_phase(&run, &workload, &subject, "measured");
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
    emit_phase(&run, &workload, &subject, "process_baseline");
    let (ir, start, goal, budget) = enumeration_fixture(&workload);
    emit_phase(&run, &workload, &subject, "canonical_ir");
    let index = build_incidence_index(&ir).expect("incidence index");
    emit_phase(&run, &workload, &subject, "incidence_index");
    emit_phase(&run, &workload, &subject, "prepared");

    match subject.as_str() {
        "paths_goal" | "paths_all" => {
            let requested_goal = if subject == "paths_goal" { goal } else { None };
            let mut query = |strong: bool| {
                let outcome = enumerate_berge_paths(&index, start, requested_goal, budget)
                    .expect("Berge path enumeration");
                if strong {
                    hash_paths(&outcome.items, outcome.bound)
                } else {
                    ((outcome.items.len() as u64) << 2)
                        ^ match outcome.bound {
                            None => 0,
                            Some(BudgetKind::Depth) => 1,
                            Some(BudgetKind::Results) => 2,
                            Some(BudgetKind::Expansions) => 3,
                        }
                }
            };
            let expected = query(true);
            let measurement = measure_calibrated_verified(&mut query);
            assert_eq!(measurement.checksum, expected);
            let outcome = enumerate_berge_paths(&index, start, requested_goal, budget)
                .expect("Berge path result");
            assert_eq!(hash_paths(&outcome.items, outcome.bound), expected);
            emit_measurement(
                &run,
                "berge_enumeration",
                &workload,
                &subject,
                &ir,
                &measurement,
            );
            emit_result(
                &run,
                "berge_enumeration",
                &workload,
                &subject,
                "NA",
                "NA",
                "NA",
                expected,
                &outcome.items.len().to_string(),
                bound_key(outcome.bound),
                Some(budget),
            );
        }
        "cycles" => {
            let mut query = |strong: bool| {
                let outcome =
                    enumerate_berge_cycles(&index, start, budget).expect("Berge cycle enumeration");
                if strong {
                    hash_cycles(&outcome.items, outcome.bound)
                } else {
                    ((outcome.items.len() as u64) << 2)
                        ^ match outcome.bound {
                            None => 0,
                            Some(BudgetKind::Depth) => 1,
                            Some(BudgetKind::Results) => 2,
                            Some(BudgetKind::Expansions) => 3,
                        }
                }
            };
            let expected = query(true);
            let measurement = measure_calibrated_verified(&mut query);
            assert_eq!(measurement.checksum, expected);
            let outcome =
                enumerate_berge_cycles(&index, start, budget).expect("Berge cycle result");
            assert_eq!(hash_cycles(&outcome.items, outcome.bound), expected);
            emit_measurement(
                &run,
                "berge_enumeration",
                &workload,
                &subject,
                &ir,
                &measurement,
            );
            emit_result(
                &run,
                "berge_enumeration",
                &workload,
                &subject,
                "NA",
                "NA",
                "NA",
                expected,
                &outcome.items.len().to_string(),
                bound_key(outcome.bound),
                Some(budget),
            );
        }
        other => panic!("invalid enumeration subject {other}"),
    }
    emit_phase(&run, &workload, &subject, "measured");
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
    let dijkstra = dijkstra_berge_path(
        &traversal,
        start,
        goal,
        &UnitCost,
        &Undirected,
        &mut workspace,
    )
    .unwrap()
    .unwrap();
    let a1 = astar_berge_path(
        &traversal,
        start,
        goal,
        &UnitCost,
        &l1,
        &Undirected,
        &mut workspace,
    )
    .unwrap()
    .unwrap();
    assert_eq!(workspace.stats().reopened, 0);
    let a2 = astar_berge_path(
        &traversal,
        start,
        goal,
        &UnitCost,
        &l2,
        &Undirected,
        &mut workspace,
    )
    .unwrap()
    .unwrap();
    assert_eq!(workspace.stats().reopened, 0);
    assert_eq!(dijkstra.total_cost, a1.total_cost);
    assert_eq!(dijkstra.total_cost, a2.total_cost);
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
}
