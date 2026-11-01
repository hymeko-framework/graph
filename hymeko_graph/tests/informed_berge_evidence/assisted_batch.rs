//! Whole-batch preparation amortization over existing production search runners.
use super::*;
use hymeko_graph::rand_lcg::Lcg;

const QUERY_SEED: u64 = 20261027;

struct QueryPlan {
    starts: Vec<NodeId>,
    hash: u64,
}
impl QueryPlan {
    fn build(nodes: usize, first: NodeId, goal: NodeId, count: usize) -> Self {
        assert!([1, 8, 64].contains(&count) && count < nodes);
        assert!(nodes <= u32::MAX as usize && first.raw() < nodes && goal.raw() < nodes);
        assert_ne!(first, goal);
        let mut starts = vec![first];
        let mut seen = HashSet::from([first.raw(), goal.raw()]);
        let mut rng = Lcg::new(QUERY_SEED);
        // A shared deterministic prefix for every batch size. Bounded rejection
        // followed by a deterministic completion also covers small domains.
        for _ in 0..nodes.saturating_mul(4) {
            if starts.len() == count {
                break;
            }
            let candidate = rng.next_in_range(nodes as u32) as usize;
            if seen.insert(candidate) {
                starts.push(NodeId::new(candidate));
            }
        }
        for candidate in 0..nodes {
            if starts.len() == count {
                break;
            }
            if seen.insert(candidate) {
                starts.push(NodeId::new(candidate));
            }
        }
        assert_eq!(starts.len(), count);
        let mut hash = fnv_mix(0xcbf2_9ce4_8422_2325, b"HYMEKO-GOAL-QUERY-PLAN");
        hash = fnv_mix(hash, &(goal.raw() as u64).to_le_bytes());
        for start in &starts {
            hash = fnv_mix(hash, &(start.raw() as u64).to_le_bytes());
        }
        Self { starts, hash }
    }
}

struct QueryOutcome {
    path: Option<WeightedBergePath>,
    work: Option<SearchStats>,
}

#[derive(Clone, Copy)]
enum BatchMethod {
    Bfs,
    Informed(Method),
}

struct QueryBatch<'a> {
    scenario: &'a Scenario,
    plan: &'a QueryPlan,
    method: BatchMethod,
}
impl QueryBatch<'_> {
    fn collect<R: SearchRunner>(&self, index: &IncidenceIndex<'_>, runner: R) -> Vec<QueryOutcome> {
        let traversal = HyperTraversal::new(index);
        let mut workspace = InformedSearchWorkspace::new();
        self.plan
            .starts
            .iter()
            .map(|&start| {
                let path = runner
                    .run(&traversal, start, self.scenario.goal, &mut workspace)
                    .unwrap();
                QueryOutcome {
                    path,
                    work: Some(workspace.stats()),
                }
            })
            .collect()
    }
    fn verify(
        &self,
        index: &IncidenceIndex<'_>,
        cost: &FixtureCost,
        result: &[QueryOutcome],
        expected: &[Option<f64>],
    ) {
        assert_eq!(result.len(), self.plan.starts.len());
        assert_eq!(expected.len(), result.len());
        for ((&start, answer), &value) in self.plan.starts.iter().zip(result).zip(expected) {
            self.scenario
                .verify_from(index, cost, start, &answer.path, value);
        }
    }
}
impl EvidenceOperation for QueryBatch<'_> {
    type Output = Vec<QueryOutcome>;
    fn execute(&mut self) -> Self::Output {
        // One timed preparation and fresh workspace per complete batch. Only
        // immutable IR and the deterministic query list live across batches.
        let index = build_incidence_index(&self.scenario.ir).unwrap();
        let method = match self.method {
            BatchMethod::Informed(method) => method,
            BatchMethod::Bfs => {
                assert!(
                    matches!(self.scenario.geometry, QueryGeometry::Grid(_)),
                    "Berge BFS is admitted only for unit-cost grid fixtures"
                );
                let traversal = HyperTraversal::new(&index);
                let mut workspace = TraversalWorkspace::new();
                return self
                    .plan
                    .starts
                    .iter()
                    .map(|&start| {
                        let path = traversal
                            .berge_shortest_path(&mut workspace, start, self.scenario.goal)
                            .unwrap()
                            .map(|path| WeightedBergePath {
                                total_cost: path.steps.len() as f64,
                                start: path.start,
                                steps: path.steps,
                            });
                        QueryOutcome { path, work: None }
                    })
                    .collect();
            }
        };
        let model = Model::build(self.scenario, &index, method);
        let heuristic = model.bound(self.scenario);
        match method {
            Method::Dijkstra => self.collect(&index, Dijkstra::new(&model.cost, &Undirected)),
            Method::AStar => self.collect(&index, AStar::new(&model.cost, &heuristic, &Undirected)),
            Method::Assisted => {
                let prepared = PreparedBergeSearch::build(
                    &index,
                    self.scenario.goal,
                    &mut TraversalWorkspace::new(),
                )
                .unwrap();
                self.collect(
                    &index,
                    Assisted {
                        prepared: &prepared,
                        cost: &model.cost,
                        heuristic: &heuristic,
                    },
                )
            }
        }
    }
    fn strong(&self, result: &Self::Output) -> u64 {
        result.iter().fold(self.plan.hash, |hash, answer| {
            fnv_mix(
                hash,
                &answer
                    .path
                    .as_ref()
                    .map_or(0, |p| p.oriented_materialisation_digest().raw())
                    .to_le_bytes(),
            )
        })
    }
    fn cheap(&self, result: &Self::Output) -> u64 {
        result.iter().fold(result.len() as u64, |hash, answer| {
            hash.wrapping_add(answer.path.as_ref().map_or(0, |p| 1 + p.steps.len() as u64))
        })
    }
}

#[test]
#[ignore = "explicit HYMEKO_ASSISTED_BENCH=1; existing Campaign and sampler"]
fn batch_cell() {
    assert_eq!(env::var("HYMEKO_ASSISTED_BENCH").as_deref(), Ok("1"));
    let run = env::var("INFORMED_BERGE_RUN").unwrap();
    let workload = env::var("INFORMED_BERGE_WORKLOAD").unwrap();
    let subject = env::var("INFORMED_BERGE_SUBJECT").unwrap();
    let (method, count) = subject
        .strip_prefix("batch_")
        .unwrap()
        .rsplit_once('_')
        .unwrap();
    let method = match method {
        "bfs" => BatchMethod::Bfs,
        "dijkstra" => BatchMethod::Informed(Method::Dijkstra),
        "astar" => BatchMethod::Informed(Method::AStar),
        "assisted" => BatchMethod::Informed(Method::Assisted),
        _ => panic!("unknown batch search method"),
    };
    let scenario = Scenario::build(&workload);
    let plan = QueryPlan::build(
        scenario.ir.nodes.len(),
        scenario.start,
        scenario.goal,
        count.parse().unwrap(),
    );
    let index = build_incidence_index(&scenario.ir).unwrap();
    let control = Model::build(&scenario, &index, Method::Dijkstra);
    control.validate(&index);
    let mut reference_workspace = InformedSearchWorkspace::new();
    let expected: Vec<_> = plan
        .starts
        .iter()
        .map(|&start| {
            dijkstra_berge_path(
                &HyperTraversal::new(&index),
                start,
                scenario.goal,
                &control.cost,
                &Undirected,
                &mut reference_workspace,
            )
            .unwrap()
            .map(|p| p.total_cost)
        })
        .collect();
    let mut operation = QueryBatch {
        scenario: &scenario,
        plan: &plan,
        method,
    };
    let checked = operation.execute();
    operation.verify(&index, &control.cost, &checked, &expected);
    drop(checked);
    let result = VerifiedRun::collect(&mut operation, &Calibrated);
    operation.verify(&index, &control.cost, &result.result, &expected);
    let cell = EvidenceCell {
        run: &run,
        workload: &workload,
        subject: &subject,
        section: "informed_query_batch",
    };
    let caller_setups = usize::from(!matches!(method, BatchMethod::Bfs));
    println!(
        "EVIDENCE_BATCH\tqueries={}\tgoal={}\tseed={QUERY_SEED}\tquery_hash={}\tverified=true\tunit=us_per_batch\tconstruction_timed=false\tquery_plan_timed=false\tindex_builds=1\tcaller_setups={caller_setups}\tworkspace=fresh_per_batch_reused_within\tmetric_scope=resident_ir_to_preparation_queries_full_paths_and_drop",
        plan.starts.len(),
        scenario.goal.raw(),
        plan.hash
    );
    for (ordinal, ((&start, answer), expected)) in plan
        .starts
        .iter()
        .zip(&result.result)
        .zip(&expected)
        .enumerate()
    {
        let cost = answer
            .path
            .as_ref()
            .map_or("NA".to_owned(), |p| format!("{:.0}", p.total_cost * 1e6));
        let expected = expected.map_or("NA".to_owned(), |v| format!("{:.0}", v * 1e6));
        let hops = answer
            .path
            .as_ref()
            .map_or("NA".to_owned(), |p| p.steps.len().to_string());
        let vertices: Vec<_> = answer
            .path
            .as_ref()
            .map_or_else(Vec::new, |p| p.nodes().map(|v| v.raw()).collect());
        let arcs: Vec<_> = answer
            .path
            .as_ref()
            .map_or_else(Vec::new, |p| p.steps.iter().map(|s| s.arc.raw()).collect());
        let expanded = answer
            .work
            .map_or("NA".to_owned(), |w| w.expanded_nodes.to_string());
        let resolved = answer
            .work
            .map_or("NA".to_owned(), |w| w.resolved_incidences.to_string());
        println!(
            "EVIDENCE_BATCH_QUERY\tordinal={ordinal}\tstart={}\tgoal={}\treachable={}\tcost_micros={cost}\texpected_cost_micros={expected}\thops={hops}\texpanded_nodes={expanded}\tresolved_incidences={resolved}\tvertices={vertices:?}\tarc_ids={arcs:?}",
            start.raw(),
            scenario.goal.raw(),
            answer.path.is_some()
        );
    }
    cell.measurement(&scenario.ir, &result.measurement);
    cell.result(&ResultSummary::Items {
        count: result.result.len(),
        bound: None,
        budget: None,
        hash: result.hash,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_plan_is_nested_unique_and_excludes_the_goal() {
        let a = QueryPlan::build(100, NodeId::new(0), NodeId::new(99), 8);
        let b = QueryPlan::build(100, NodeId::new(0), NodeId::new(99), 64);
        assert_eq!(a.starts, b.starts[..8]);
        assert_eq!(b.starts.iter().collect::<HashSet<_>>().len(), 64);
        assert!(!b.starts.contains(&NodeId::new(99)));
        assert_eq!(
            b.hash,
            QueryPlan::build(100, NodeId::new(0), NodeId::new(99), 64).hash
        );
        assert!(
            std::panic::catch_unwind(|| QueryPlan::build(8, NodeId::new(0), NodeId::new(7), 8))
                .is_err()
        );
    }

    #[test]
    fn batch_adapters_match_independent_costs_and_complete_witnesses() {
        let scenario = Scenario {
            geometry: QueryGeometry::Grid(10),
            ir: workloads::grid_ir(10, 10),
            start: NodeId::new(0),
            goal: NodeId::new(99),
        };
        let index = build_incidence_index(&scenario.ir).unwrap();
        let cost = FixtureCost::Unit;
        for count in [1, 8, 64] {
            let plan = QueryPlan::build(100, scenario.start, scenario.goal, count);
            let expected: Vec<_> = plan
                .starts
                .iter()
                .map(|start| Some((18 - start.raw() / 10 - start.raw() % 10) as f64))
                .collect();
            for method in [
                BatchMethod::Bfs,
                BatchMethod::Informed(Method::Dijkstra),
                BatchMethod::Informed(Method::AStar),
                BatchMethod::Informed(Method::Assisted),
            ] {
                let mut batch = QueryBatch {
                    scenario: &scenario,
                    plan: &plan,
                    method,
                };
                let result = batch.execute();
                batch.verify(&index, &cost, &result, &expected);
                assert_ne!(batch.strong(&result), 0);
                assert!(batch.cheap(&result) >= count as u64);
                assert!(
                    result
                        .iter()
                        .all(|r| r.work.is_none_or(|w| w.reopened == 0))
                );
            }
        }
    }

    #[test]
    fn weighted_batch_includes_unreachable_sources_and_matches_the_independent_oracle() {
        let blocked: HashSet<_> = (30..40).collect();
        let scenario = Scenario {
            geometry: QueryGeometry::Roads,
            ir: workloads::map_grid_ir(10, 10, &blocked),
            start: NodeId::new(0),
            goal: NodeId::new(99),
        };
        let index = build_incidence_index(&scenario.ir).unwrap();
        let model = Model::build(&scenario, &index, Method::Dijkstra);
        model.validate(&index);
        let plan = QueryPlan::build(100, scenario.start, scenario.goal, 64);
        let expected: Vec<_> = plan
            .starts
            .iter()
            .map(|&start| {
                workloads::oracle_dijkstra_cost(
                    &scenario.ir,
                    &index,
                    start,
                    scenario.goal,
                    workloads::OracleCost::Weight { base: 0.0 },
                )
            })
            .collect();
        assert!(expected.iter().any(Option::is_none));
        assert!(expected.iter().any(Option::is_some));
        for method in [Method::Dijkstra, Method::AStar, Method::Assisted] {
            let mut batch = QueryBatch {
                scenario: &scenario,
                plan: &plan,
                method: BatchMethod::Informed(method),
            };
            let first = batch.execute();
            batch.verify(&index, &model.cost, &first, &expected);
            let again = batch.execute();
            assert_eq!(batch.strong(&first), batch.strong(&again));
        }
    }
}
