//! Thin timing clients for the production prepared Berge search. Sampling,
//! process orchestration, fixtures and search engines retain their existing owners.
use super::*;
use hymeko::ir::incidence::IncidenceRef;
use hymeko_graph::direct_ir::{
    BergeCost, PreparedBergeSearch, ZeroHeuristic, astar_berge_path, dijkstra_berge_path,
};
use workloads::search::{SearchResult, SearchRunner};

#[path = "assisted_batch.rs"]
mod batch;

#[derive(Clone, Copy, PartialEq)]
enum Method {
    Dijkstra,
    AStar,
    Assisted,
}

/// Cost configuration for these fixed fixtures. The positive minimum is checked
/// against every incidence before sampling; step evaluation stays in Graph.
enum FixtureCost {
    Unit,
    Weighted(f64),
}
impl BergeCost for FixtureCost {
    fn minimum_step_cost(&self) -> f64 {
        match self {
            Self::Unit => 1.0,
            Self::Weighted(minimum) => *minimum,
        }
    }
    fn step_cost(&self, from: DeclId, to: &IncidenceRef<'_>) -> f64 {
        match self {
            Self::Unit => UnitCost.step_cost(from, to),
            Self::Weighted(_) => IncidenceWeightCost { base: 0.0 }.step_cost(from, to),
        }
    }
}

enum CallerBound<'a> {
    Zero,
    Manhattan(ManhattanHeuristic<'a>),
    Metric(MetricHeuristic<'a, Positions>),
    Hops(HopHeuristic<'a>),
}
impl BergeHeuristic for CallerBound<'_> {
    const ADMISSIBLE: bool = true;
    const CONSISTENT: bool = true;
    fn h(&self, ir: &Ir, node: DeclId) -> f64 {
        match self {
            Self::Zero => ZeroHeuristic.h(ir, node),
            Self::Manhattan(h) => h.h(ir, node),
            Self::Metric(h) => h.h(ir, node),
            Self::Hops(h) => h.h(ir, node),
        }
    }
}

struct Model {
    cost: FixtureCost,
    positions: Positions,
    hops: Vec<u32>,
    method: Method,
}
impl Model {
    fn build(scenario: &Scenario, index: &IncidenceIndex<'_>, method: Method) -> Self {
        let grid = matches!(scenario.geometry, QueryGeometry::Grid(_));
        let cost = if grid {
            FixtureCost::Unit
        } else {
            FixtureCost::Weighted(workloads::map_min_edge_cost(&scenario.ir))
        };
        let positions =
            if method == Method::Dijkstra || scenario.geometry == QueryGeometry::Transport {
                Positions(Vec::new())
            } else {
                scenario.positions()
            };
        // Assisted transport uses its own production hop certificate. Do not
        // compute another reverse BFS for the same bound on that path.
        let hops = if scenario.geometry == QueryGeometry::Transport && method == Method::AStar {
            workloads::hop_distances_from(&scenario.ir, index, scenario.goal)
        } else {
            Vec::new()
        };
        Self {
            cost,
            positions,
            hops,
            method,
        }
    }
    fn bound<'a>(&'a self, scenario: &Scenario) -> CallerBound<'a> {
        let goal = scenario.ir.nodes[scenario.goal.raw()].decl;
        if self.method == Method::Dijkstra {
            CallerBound::Zero
        } else if matches!(scenario.geometry, QueryGeometry::Grid(_)) {
            CallerBound::Manhattan(ManhattanHeuristic {
                positions: &self.positions,
                goal: self.positions.0[goal.raw()].unwrap(),
            })
        } else if scenario.geometry == QueryGeometry::Transport {
            if self.method == Method::Assisted {
                CallerBound::Zero
            } else {
                CallerBound::Hops(HopHeuristic::new(&self.hops, self.cost.minimum_step_cost()))
            }
        } else {
            CallerBound::Metric(MetricHeuristic::new(&self.positions, goal))
        }
    }
    fn validate(&self, index: &IncidenceIndex<'_>) {
        for (a, arc) in index.ir().arcs.iter().enumerate() {
            for slot in 0..arc.refs.len() {
                let to = index.resolve(hymeko::common::ids::HyperArcId::new(a), slot as u32);
                let value = self.cost.step_cost(to.target, &to);
                assert!(value.is_finite() && value >= self.cost.minimum_step_cost());
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum QueryGeometry {
    Grid(usize),
    Roads,
    Transport,
}

struct Scenario {
    geometry: QueryGeometry,
    ir: Ir,
    start: NodeId,
    goal: NodeId,
}
impl Scenario {
    fn build(name: &str) -> Self {
        let (ir, start, goal) = search_fixture(name);
        let geometry = match name {
            "grid200_d20" | "grid200_d40" | "grid200_d80" => QueryGeometry::Grid(200),
            "grid512_d511" => QueryGeometry::Grid(512),
            name if name.starts_with("transport") || name.starts_with("tour") => {
                QueryGeometry::Transport
            }
            _ => QueryGeometry::Roads,
        };
        Self {
            geometry,
            ir,
            start,
            goal,
        }
    }
    fn positions(&self) -> Positions {
        match self.geometry {
            QueryGeometry::Grid(side) => Positions::from_grid(side, side),
            _ => Positions::from_ir(&self.ir),
        }
    }
    fn heuristic_name(&self) -> &'static str {
        match self.geometry {
            QueryGeometry::Grid(_) => "manhattan",
            QueryGeometry::Roads => "euclidean",
            QueryGeometry::Transport => "zero",
        }
    }
    fn verify(
        &self,
        index: &IncidenceIndex<'_>,
        cost: &FixtureCost,
        path: &Option<WeightedBergePath>,
        expected: Option<f64>,
    ) {
        self.verify_from(index, cost, self.start, path, expected);
    }
    fn verify_from(
        &self,
        index: &IncidenceIndex<'_>,
        cost: &FixtureCost,
        start: NodeId,
        path: &Option<WeightedBergePath>,
        expected: Option<f64>,
    ) {
        assert_eq!(
            path.as_ref().map(|p| p.total_cost),
            expected,
            "optimal cost mismatch"
        );
        if let Some(path) = path {
            assert_eq!(path.start, start);
            let mut node = start;
            let mut sum = 0.0;
            for step in &path.steps {
                assert_eq!(step.from.node, node);
                let from = index.resolve(step.arc, step.from.slot);
                let to = index.resolve(step.arc, step.to.slot);
                assert_eq!(from.target, self.ir.nodes[node.raw()].decl);
                assert_eq!(to.target, self.ir.nodes[step.to.node.raw()].decl);
                sum += cost.step_cost(from.target, &to);
                node = step.to.node;
            }
            assert_eq!(node, self.goal);
            assert_eq!(sum, path.total_cost);
        }
    }
    fn warm<R: SearchRunner>(
        &self,
        cell: &EvidenceCell<'_>,
        index: &IncidenceIndex<'_>,
        model: &Model,
        runner: R,
        expected: Option<f64>,
    ) {
        let check = runner
            .run(
                &HyperTraversal::new(index),
                self.start,
                self.goal,
                &mut InformedSearchWorkspace::new(),
            )
            .unwrap();
        self.verify(index, &model.cost, &check, expected);
        cell.weighted(index, self.start, self.goal, runner);
    }
    fn measure(&self, cell: &EvidenceCell<'_>) {
        let index = build_incidence_index(&self.ir).unwrap();
        let control = Model::build(self, &index, Method::Dijkstra);
        control.validate(&index);
        let reference = dijkstra_berge_path(
            &HyperTraversal::new(&index),
            self.start,
            self.goal,
            &control.cost,
            &Undirected,
            &mut InformedSearchWorkspace::new(),
        )
        .unwrap();
        let expected = reference.as_ref().map(|p| p.total_cost);
        let subject = cell.subject;
        let first = subject.starts_with("first_");
        let prep = subject == "prepare";
        let scope = if first {
            "resident_ir_to_index_caller_setup_search_and_drop"
        } else if prep {
            "prepared_index_to_goal_certificate_and_drop_reused_traversal_workspace"
        } else {
            "prepared_index_bounds_reused_search_workspace_query_result_drop"
        };
        println!(
            "EVIDENCE_INPUT\tpipeline=berge_bound_assist\tstart={}\tgoal={}\tconstruction_timed=false\tindex_timed={first}\tgoal_preparation_timed={}\tcaller_setup_timed={first}\tmetric_scope={scope}\tverified=true\texpected_cost_micros={}\tflow=undirected",
            self.start.raw(),
            self.goal.raw(),
            prep || subject == "first_assisted",
            expected.map_or("NA".to_owned(), |v| format!("{:.0}", v * 1e6))
        );
        if prep {
            let mut op = GoalPreparation {
                index: &index,
                goal: self.goal,
                workspace: TraversalWorkspace::new(),
            };
            let run = VerifiedRun::collect(&mut op, &Calibrated);
            cell.measurement(&self.ir, &run.measurement);
            cell.result(&ResultSummary::Items {
                count: self.ir.nodes.len(),
                bound: None,
                budget: None,
                hash: run.hash,
            });
            return;
        }
        let method = match subject {
            "dijkstra" | "first_dijkstra" => Method::Dijkstra,
            "astar" | "first_astar" => Method::AStar,
            "assisted" | "first_assisted" => Method::Assisted,
            other => panic!("unknown assisted phase {other}"),
        };
        if first {
            let mut op = FirstQuery {
                scenario: self,
                method,
                stats: SearchStats::default(),
            };
            let check = op.execute();
            self.verify(&index, &control.cost, &check, expected);
            let run = VerifiedRun::collect(&mut op, &Calibrated);
            self.verify(&index, &control.cost, &run.result, expected);
            if let Some(path) = &run.result {
                let vertices: Vec<_> = std::iter::once(path.start.raw())
                    .chain(path.steps.iter().map(|s| s.to.node.raw()))
                    .collect();
                let arcs: Vec<_> = path.steps.iter().map(|s| s.arc.raw()).collect();
                println!(
                    "EVIDENCE_ROUTE\tkind=path\tverified=true\thops={}\tcost={}\tvertices={vertices:?}\tarc_ids={arcs:?}",
                    arcs.len(),
                    path.total_cost
                );
            }
            cell.measurement(&self.ir, &run.measurement);
            cell.work(run.stats.unwrap());
            cell.result(&ResultSummary::weighted(&run.result));
        } else {
            let model = Model::build(self, &index, method);
            let heuristic = model.bound(self);
            match method {
                Method::Dijkstra => self.warm(
                    cell,
                    &index,
                    &model,
                    Dijkstra::new(&model.cost, &Undirected),
                    expected,
                ),
                Method::AStar => self.warm(
                    cell,
                    &index,
                    &model,
                    AStar::new(&model.cost, &heuristic, &Undirected),
                    expected,
                ),
                Method::Assisted => {
                    let prepared = PreparedBergeSearch::build(
                        &index,
                        self.goal,
                        &mut TraversalWorkspace::new(),
                    )
                    .unwrap();
                    self.warm(
                        cell,
                        &index,
                        &model,
                        Assisted {
                            prepared: &prepared,
                            cost: &model.cost,
                            heuristic: &heuristic,
                        },
                        expected,
                    );
                }
            }
        }
    }
}

struct Assisted<'a, 'index, 'ir> {
    prepared: &'a PreparedBergeSearch<'index, 'ir>,
    cost: &'a FixtureCost,
    heuristic: &'a CallerBound<'a>,
}
impl SearchRunner for Assisted<'_, '_, '_> {
    fn run(
        &self,
        _: &HyperTraversal<'_, '_>,
        start: NodeId,
        _: NodeId,
        workspace: &mut InformedSearchWorkspace,
    ) -> SearchResult {
        self.prepared.search_with_heuristic(
            start,
            self.cost,
            self.heuristic,
            &Undirected,
            workspace,
        )
    }
}

struct FirstQuery<'a> {
    scenario: &'a Scenario,
    method: Method,
    stats: SearchStats,
}
impl EvidenceOperation for FirstQuery<'_> {
    type Output = Option<WeightedBergePath>;
    fn execute(&mut self) -> Self::Output {
        let index = build_incidence_index(&self.scenario.ir).unwrap();
        let model = Model::build(self.scenario, &index, self.method);
        let h = model.bound(self.scenario);
        let mut ws = InformedSearchWorkspace::new();
        let path = if self.method == Method::Assisted {
            PreparedBergeSearch::build(&index, self.scenario.goal, &mut TraversalWorkspace::new())
                .unwrap()
                .search_with_heuristic(self.scenario.start, &model.cost, &h, &Undirected, &mut ws)
                .unwrap()
        } else {
            astar_berge_path(
                &HyperTraversal::new(&index),
                self.scenario.start,
                self.scenario.goal,
                &model.cost,
                &h,
                &Undirected,
                &mut ws,
            )
            .unwrap()
        };
        self.stats = ws.stats();
        path
    }
    fn strong(&self, result: &Self::Output) -> u64 {
        result.as_ref().map_or(0, |p| p.path_hash().raw())
    }
    fn cheap(&self, result: &Self::Output) -> u64 {
        result
            .as_ref()
            .map_or(0, |p| (1_u64 << 63) ^ p.hyperedge_len() as u64)
    }
    fn stats(&self) -> Option<SearchStats> {
        Some(self.stats)
    }
}

struct GoalPreparation<'a, 'ir> {
    index: &'a IncidenceIndex<'ir>,
    goal: NodeId,
    workspace: TraversalWorkspace,
}
impl<'a, 'ir> EvidenceOperation for GoalPreparation<'a, 'ir> {
    type Output = PreparedBergeSearch<'a, 'ir>;
    fn execute(&mut self) -> Self::Output {
        PreparedBergeSearch::build(self.index, self.goal, &mut self.workspace).unwrap()
    }
    fn strong(&self, plan: &Self::Output) -> u64 {
        let mut hash = fnv_mix(0xcbf2_9ce4_8422_2325, b"HYMEKO-PREPARED-BERGE-PLAN");
        for n in 0..self.index.ir().nodes.len() {
            hash = fnv_mix(
                hash,
                &plan
                    .relaxed_hops(NodeId::new(n))
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            );
            hash = fnv_mix(
                hash,
                &plan.entropy_priority(NodeId::new(n)).unwrap().to_le_bytes(),
            );
        }
        hash
    }
    fn cheap(&self, plan: &Self::Output) -> u64 {
        u64::from(plan.relaxed_hops(self.goal).unwrap()) ^ self.index.ir().nodes.len() as u64
    }
}

#[test]
#[ignore = "explicit HYMEKO_ASSISTED_BENCH=1; existing Campaign/ProcessRunner"]
fn assisted_cell() {
    assert_eq!(env::var("HYMEKO_ASSISTED_BENCH").as_deref(), Ok("1"));
    let run = env::var("INFORMED_BERGE_RUN").unwrap();
    let workload = env::var("INFORMED_BERGE_WORKLOAD").unwrap();
    let subject = env::var("INFORMED_BERGE_SUBJECT").unwrap();
    let cell = EvidenceCell {
        run: &run,
        section: if subject == "prepare" {
            "heuristic_preparation"
        } else {
            "informed_search"
        },
        workload: &workload,
        subject: &subject,
    };
    Scenario::build(&workload).measure(&cell);
}

#[test]
fn assisted_adapters_preserve_cost_paths_and_preparation_boundaries() {
    for name in ["grid200_d20", "map32", "transport128"] {
        let scenario = Scenario::build(name);
        let index = build_incidence_index(&scenario.ir).unwrap();
        let model = Model::build(&scenario, &index, Method::Dijkstra);
        model.validate(&index);
        let expected = dijkstra_berge_path(
            &HyperTraversal::new(&index),
            scenario.start,
            scenario.goal,
            &model.cost,
            &Undirected,
            &mut InformedSearchWorkspace::new(),
        )
        .unwrap()
        .map(|p| p.total_cost);
        for method in [Method::Dijkstra, Method::AStar, Method::Assisted] {
            let mut op = FirstQuery {
                scenario: &scenario,
                method,
                stats: SearchStats::default(),
            };
            let path = op.execute();
            scenario.verify(&index, &model.cost, &path, expected);
            assert_eq!(op.stats.seeded_bound, method == Method::Assisted);
        }
        let mut op = GoalPreparation {
            index: &index,
            goal: scenario.goal,
            workspace: TraversalWorkspace::new(),
        };
        let a = op.execute();
        let b = op.execute();
        assert_eq!(op.strong(&a), op.strong(&b));
        assert_eq!(op.cheap(&a), scenario.ir.nodes.len() as u64);
    }
}

#[test]
fn scaling_geometry_and_default_tour_limits_are_explicit() {
    for (name, nodes, cost) in [
        ("grid512_d511", 262144, Some(511.0)),
        ("map256", 65536, None),
        ("transport256_off", 65536, None),
    ] {
        let scenario = Scenario::build(name);
        assert_eq!(scenario.ir.nodes.len(), nodes);
        let index = build_incidence_index(&scenario.ir).unwrap();
        let model = Model::build(&scenario, &index, Method::Dijkstra);
        model.validate(&index);
        let mut reference = FirstQuery {
            scenario: &scenario,
            method: Method::Dijkstra,
            stats: Default::default(),
        };
        let path = reference.execute();
        let expected = path.as_ref().map(|p| p.total_cost);
        scenario.verify(&index, &model.cost, &path, expected);
        if let Some(cost) = cost {
            assert_eq!(expected, Some(cost));
        }
        let mut astar = FirstQuery {
            scenario: &scenario,
            method: Method::AStar,
            stats: Default::default(),
        };
        scenario.verify(&index, &model.cost, &astar.execute(), expected);
        if name == "transport256_off" {
            assert_eq!(scenario.heuristic_name(), "zero");
            assert!(path.unwrap().steps.len() > 1);
        }
    }
    let mut solved = 0;
    for name in ["tour24", "tour32", "tour64", "tour64_corridor"] {
        let scenario = Scenario::build(name);
        let costs = scenario
            .ir
            .arcs
            .iter()
            .map(|a| workloads::first_num_weight_pub(&scenario.ir, &a.refs[0]))
            .collect();
        let query = TourQuery {
            scenario: &scenario,
            costs,
        };
        match query.checked() {
            Ok(tour) => {
                solved += 1;
                assert_eq!(tour.order.len(), scenario.ir.nodes.len() + 1);
                assert_eq!(
                    tour.walk.nodes().collect::<HashSet<_>>().len(),
                    scenario.ir.nodes.len()
                );
                assert_eq!(tour.walk.nodes().last(), Some(tour.walk.start));
                println!(
                    "SCALE_CHECK {name} odd={} cost={} hops={}",
                    tour.odd_vertices,
                    tour.total_cost,
                    tour.walk.steps.len()
                );
            }
            Err(hymeko_graph::direct_ir::TourError::Budget { resource }) => {
                assert_eq!(resource, "matching states");
                println!("SCALE_CHECK {name} matching_states_limit");
            }
            Err(error) => panic!("unexpected scaling failure: {error}"),
        }
    }
    assert!(
        solved >= 2,
        "must solve genuinely larger tours with unchanged default limits"
    );
}

/// Neutral export from the very same IR fixtures used by the Rust client.
/// This correctness operation performs no timing and is inert without a destination.
#[test]
fn export_routing_input() {
    use std::io::Write;
    let Ok(destination) = env::var("HYMEKO_ROUTING_EXPORT") else {
        return;
    };
    let mut out = std::io::BufWriter::new(std::fs::File::create(destination).unwrap());
    write!(
        out,
        "{{\"representation\":\"canonical Core Ir\",\"cases\":["
    )
    .unwrap();
    let names = env::var("HYMEKO_ROUTING_CASES")
        .unwrap_or_else(|_| "grid200_d80,map128,transport128,tour16,tour8".to_owned());
    for (ordinal, name) in names.split(',').enumerate() {
        if ordinal != 0 {
            write!(out, ",").unwrap();
        }
        let scenario = Scenario::build(name);
        let ir = &scenario.ir;
        let index = build_incidence_index(ir).unwrap();
        let model = Model::build(&scenario, &index, Method::Dijkstra);
        let path = dijkstra_berge_path(
            &HyperTraversal::new(&index),
            scenario.start,
            scenario.goal,
            &model.cost,
            &Undirected,
            &mut InformedSearchWorkspace::new(),
        )
        .unwrap()
        .unwrap();
        scenario.verify(
            &index,
            &model.cost,
            &Some(path.clone()),
            Some(path.total_cost),
        );
        let labels =
            hymeko_graph::direct_ir::connected_components(&index, &mut TraversalWorkspace::new())
                .unwrap();
        let positions = scenario.positions();
        let label_values: Vec<_> = labels.labels().to_vec();
        let reachable: Vec<_> = HyperTraversal::new(&index)
            .reachable_nodes(&mut TraversalWorkspace::new(), scenario.start)
            .unwrap()
            .nodes()
            .map(|n| n.raw())
            .collect();
        let incidences: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
        write!(out,"{{\"family\":\"{name}\",\"nodes\":{},\"arcs\":{},\"incidences\":{incidences},\"components\":{},\"labels\":{label_values:?},\"reachable\":{reachable:?},\"start\":{},\"goal\":{},\"shortest_hops\":{},\"shortest_cost\":{},\"memberships\":[",
            ir.nodes.len(),ir.arcs.len(),labels.count(),scenario.start.raw(),scenario.goal.raw(),path.steps.len(),path.total_cost).unwrap();
        let mut costs = Vec::new();
        for (a, arc) in ir.arcs.iter().enumerate() {
            if a != 0 {
                write!(out, ",").unwrap();
            }
            let members: Vec<_> = (0..arc.refs.len())
                .map(|s| {
                    ir.as_node(
                        index
                            .resolve(hymeko::common::ids::HyperArcId::new(a), s as u32)
                            .target,
                    )
                    .unwrap()
                    .raw()
                })
                .collect();
            write!(out, "{members:?}").unwrap();
            let values: Vec<_> = (0..arc.refs.len())
                .map(|s| {
                    model.cost.step_cost(
                        ir.nodes[0].decl,
                        &index.resolve(hymeko::common::ids::HyperArcId::new(a), s as u32),
                    )
                })
                .collect();
            assert!(values.iter().all(|v| *v == values[0]));
            costs.push(values[0]);
        }
        let positions: Vec<_> = ir
            .nodes
            .iter()
            .map(|n| positions.0[n.decl.raw()].unwrap())
            .collect();
        let vertices: Vec<_> = std::iter::once(scenario.start.raw())
            .chain(path.steps.iter().map(|s| s.to.node.raw()))
            .collect();
        let arcs: Vec<_> = path.steps.iter().map(|s| s.arc.raw()).collect();
        write!(out,"],\"heuristic\":\"{}\",\"costs\":{costs:?},\"positions\":{positions:?},\"reference_vertices\":{vertices:?},\"reference_arcs\":{arcs:?}}}",scenario.heuristic_name()).unwrap();
    }
    write!(out, "]}}").unwrap();
    out.flush().unwrap();
}

struct TourQuery<'a> {
    scenario: &'a Scenario,
    costs: Vec<f64>,
}
impl TourQuery<'_> {
    fn checked(
        &self,
    ) -> Result<hymeko_graph::direct_ir::BergeTour, hymeko_graph::direct_ir::TourError> {
        let index = build_incidence_index(&self.scenario.ir).unwrap();
        hymeko_graph::direct_ir::Christofides::solve(&index, &self.costs, Default::default())
    }
}
impl EvidenceOperation for TourQuery<'_> {
    type Output = hymeko_graph::direct_ir::BergeTour;
    fn execute(&mut self) -> Self::Output {
        self.checked().unwrap()
    }
    fn strong(&self, tour: &Self::Output) -> u64 {
        hash_berge_path(&tour.walk)
    }
    fn cheap(&self, tour: &Self::Output) -> u64 {
        tour.total_cost.to_bits() ^ tour.walk.steps.len() as u64
    }
}

#[test]
#[ignore = "explicit HYMEKO_ASSISTED_BENCH=1; same guarded Campaign"]
fn tour_cell() {
    assert_eq!(env::var("HYMEKO_ASSISTED_BENCH").as_deref(), Ok("1"));
    let name = env::var("INFORMED_BERGE_WORKLOAD").unwrap();
    let run = env::var("INFORMED_BERGE_RUN").unwrap();
    let scenario = Scenario::build(&name);
    let costs = scenario
        .ir
        .arcs
        .iter()
        .map(|a| workloads::first_num_weight_pub(&scenario.ir, &a.refs[0]))
        .collect();
    let mut query = TourQuery {
        scenario: &scenario,
        costs,
    };
    match query.checked() {
        Ok(_) => (),
        Err(hymeko_graph::direct_ir::TourError::Budget { resource }) => {
            let incidences: usize = scenario.ir.arcs.iter().map(|a| a.refs.len()).sum();
            println!(
                "EVIDENCE_INPUT\tpipeline=metric_christofides\tconstruction_timed=false\tindex_timed=true\tmetric_scope=resident_ir_to_index_metric_christofides_expanded_walk_drop\tverified=true"
            );
            println!(
                "EVIDENCE_CELL\tsection=metric_tour\tsubject=christofides\tworkload={name}\tstatus=NA\treason=budget_{}\tchecksum=budget_limit\tnodes={}\tarcs={}\tincidences={incidences}",
                resource.replace(' ', "_"),
                scenario.ir.nodes.len(),
                scenario.ir.arcs.len()
            );
            return;
        }
        Err(error) => panic!("unexpected tour failure: {error}"),
    }
    let result = VerifiedRun::collect(&mut query, &Calibrated);
    let tour = &result.result;
    let mut visited = vec![false; scenario.ir.nodes.len()];
    visited[tour.walk.start.raw()] = true;
    let mut last = tour.walk.start;
    let mut total = 0.0;
    let index = build_incidence_index(&scenario.ir).unwrap();
    for step in &tour.walk.steps {
        assert_eq!(step.from.node, last);
        assert_eq!(
            index.resolve(step.arc, step.from.slot).target,
            scenario.ir.nodes[last.raw()].decl
        );
        assert_eq!(
            index.resolve(step.arc, step.to.slot).target,
            scenario.ir.nodes[step.to.node.raw()].decl
        );
        last = step.to.node;
        visited[last.raw()] = true;
        total += query.costs[step.arc.raw()];
    }
    assert_eq!(last, tour.walk.start);
    assert!(visited.iter().all(|&x| x));
    assert_eq!(total, tour.total_cost);
    let vertices: Vec<_> = tour.walk.nodes().map(|n| n.raw()).collect();
    let arcs: Vec<_> = tour.walk.steps.iter().map(|s| s.arc.raw()).collect();
    println!(
        "EVIDENCE_ROUTE\tkind=tour\tverified=true\tclosed=true\tvisited={}\thops={}\tcost={}\todd_vertices={}\tvertices={vertices:?}\tarc_ids={arcs:?}",
        visited.len(),
        arcs.len(),
        total,
        tour.odd_vertices
    );
    println!(
        "EVIDENCE_INPUT\tpipeline=metric_christofides\tconstruction_timed=false\tindex_timed=true\tmetric_scope=resident_ir_to_index_metric_christofides_expanded_walk_drop\tverified=true"
    );
    let cell = EvidenceCell {
        run: &run,
        section: "metric_tour",
        workload: &name,
        subject: "christofides",
    };
    cell.measurement(&scenario.ir, &result.measurement);
    cell.result(&ResultSummary::Path {
        reachable: true,
        cost: Some(total),
        hops: Some(arcs.len()),
        hash: result.hash,
    });
}
