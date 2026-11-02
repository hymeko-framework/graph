//! Verification protocol shared by weighted search, shortest paths and bounded
//! enumeration. Hashing stays outside sampling; cheap sentinels stay inside it.
use super::*;
use hymeko_graph::direct_ir::{DirectIrError, EnumOutcome};
use workloads::search::SearchRunner;

impl EvidenceCell<'_> {
    pub fn weighted<R: SearchRunner>(
        &self,
        index: &IncidenceIndex<'_>,
        start: NodeId,
        goal: NodeId,
        runner: R,
    ) {
        let traversal = HyperTraversal::new(index);
        self.phase("prepared");
        let run = {
            let mut workspace = InformedSearchWorkspace::new();
            let mut operation = WeightedQuery {
                case: SearchCase::new(&traversal, start, goal, &mut workspace),
                runner,
            };
            VerifiedRun::collect(&mut operation, &Calibrated)
        }; // Keep the original scratch retirement boundary, before report formatting.
        self.measurement(index.ir(), &run.measurement);
        self.work(run.stats.expect("weighted search records work"));
        self.result(&ResultSummary::weighted(&run.result));
    }

    pub fn enumeration<R: EnumerationRunner>(
        &self,
        index: &IncidenceIndex<'_>,
        start: NodeId,
        budget: EnumBudget,
        runner: R,
    ) {
        let mut operation = EnumerationQuery {
            index,
            start,
            budget,
            runner,
        };
        let run = VerifiedRun::collect(&mut operation, &Calibrated);
        self.measurement(index.ir(), &run.measurement);
        self.result(&ResultSummary::Items {
            count: run.result.items.len(),
            bound: run.result.bound,
            budget: Some(budget),
            hash: run.hash,
        });
    }
}

pub(super) trait EvidenceSampler {
    fn sample(&self, query: impl FnMut(bool) -> u64) -> Measurement;
}

pub(super) struct Calibrated;
impl EvidenceSampler for Calibrated {
    fn sample(&self, query: impl FnMut(bool) -> u64) -> Measurement {
        measure_calibrated_verified(query)
    }
}

pub(super) trait EvidenceOperation {
    type Output;
    fn execute(&mut self) -> Self::Output;
    fn strong(&self, result: &Self::Output) -> u64;
    fn cheap(&self, result: &Self::Output) -> u64;
    fn stats(&self) -> Option<SearchStats> {
        None
    }
}

pub(super) struct VerifiedRun<T> {
    pub measurement: Measurement,
    pub stats: Option<SearchStats>,
    pub result: T,
    pub hash: u64,
}

impl<T> VerifiedRun<T> {
    pub fn collect<O: EvidenceOperation<Output = T>>(
        operation: &mut O,
        sampler: &impl EvidenceSampler,
    ) -> Self {
        let mut query = |strong| {
            let result = operation.execute();
            if strong {
                operation.strong(&result)
            } else {
                operation.cheap(&result)
            }
        };
        let hash = query(true);
        let measurement = sampler.sample(&mut query);
        assert_eq!(measurement.checksum, hash, "measurement checksum drift");
        let stats = operation.stats();
        let result = operation.execute();
        assert_eq!(operation.stats(), stats, "search work drift");
        assert_eq!(operation.strong(&result), hash, "result hash drift");
        Self {
            measurement,
            stats,
            result,
            hash,
        }
    }
}

pub(super) struct WeightedQuery<'a, 'index, 'ir, 'scratch, R> {
    pub case: SearchCase<'a, 'index, 'ir, 'scratch>,
    pub runner: R,
}
impl<R: SearchRunner> EvidenceOperation for WeightedQuery<'_, '_, '_, '_, R> {
    type Output = Option<WeightedBergePath>;
    fn execute(&mut self) -> Self::Output {
        self.case
            .run(&self.runner)
            .expect("weighted evidence query")
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
        Some(self.case.stats())
    }
}

pub(super) trait EnumerationRunner {
    type Item;
    fn run(
        &self,
        index: &IncidenceIndex<'_>,
        start: NodeId,
        budget: EnumBudget,
    ) -> Result<EnumOutcome<Self::Item>, DirectIrError>;
    fn hash(&self, result: &EnumOutcome<Self::Item>) -> u64;
}

pub(super) struct Paths(pub Option<NodeId>);
pub(super) struct Cycles;

impl EnumerationRunner for Paths {
    type Item = BergePath;
    fn run(
        &self,
        index: &IncidenceIndex<'_>,
        start: NodeId,
        budget: EnumBudget,
    ) -> Result<EnumOutcome<Self::Item>, DirectIrError> {
        enumerate_berge_paths(index, start, self.0, budget)
    }
    fn hash(&self, result: &EnumOutcome<Self::Item>) -> u64 {
        hash_paths(&result.items, result.bound)
    }
}
impl EnumerationRunner for Cycles {
    type Item = BergeCycle;
    fn run(
        &self,
        index: &IncidenceIndex<'_>,
        start: NodeId,
        budget: EnumBudget,
    ) -> Result<EnumOutcome<Self::Item>, DirectIrError> {
        enumerate_berge_cycles(index, start, budget)
    }
    fn hash(&self, result: &EnumOutcome<Self::Item>) -> u64 {
        hash_cycles(&result.items, result.bound)
    }
}

pub(super) struct EnumerationQuery<'a, 'ir, R> {
    pub index: &'a IncidenceIndex<'ir>,
    pub start: NodeId,
    pub budget: EnumBudget,
    pub runner: R,
}
impl<R: EnumerationRunner> EvidenceOperation for EnumerationQuery<'_, '_, R> {
    type Output = EnumOutcome<R::Item>;
    fn execute(&mut self) -> Self::Output {
        self.runner
            .run(self.index, self.start, self.budget)
            .expect("bounded evidence query")
    }
    fn strong(&self, result: &Self::Output) -> u64 {
        self.runner.hash(result)
    }
    fn cheap(&self, result: &Self::Output) -> u64 {
        ((result.items.len() as u64) << 2)
            ^ match result.bound {
                None => 0,
                Some(BudgetKind::Depth) => 1,
                Some(BudgetKind::Results) => 2,
                Some(BudgetKind::Expansions) => 3,
            }
    }
}

pub(super) struct ShortestQuery<'a, 'index, 'ir> {
    pub traversal: &'a HyperTraversal<'index, 'ir>,
    pub start: NodeId,
    pub goal: NodeId,
    pub workspace: TraversalWorkspace,
}
impl EvidenceOperation for ShortestQuery<'_, '_, '_> {
    type Output = Option<BergePath>;
    fn execute(&mut self) -> Self::Output {
        self.traversal
            .berge_shortest_path(&mut self.workspace, self.start, self.goal)
            .expect("BFS evidence query")
    }
    fn strong(&self, result: &Self::Output) -> u64 {
        result.as_ref().map_or(0, hash_berge_path)
    }
    fn cheap(&self, result: &Self::Output) -> u64 {
        result
            .as_ref()
            .map_or(0, |p| (1_u64 << 63) ^ p.hyperedge_len() as u64)
    }
}

pub(super) struct HopPreparation<'a, 'ir> {
    pub index: &'a IncidenceIndex<'ir>,
    pub goal: NodeId,
}
impl EvidenceOperation for HopPreparation<'_, '_> {
    type Output = Vec<u32>;
    fn execute(&mut self) -> Self::Output {
        workloads::hop_distances_from(self.index.ir(), self.index, self.goal)
    }
    fn strong(&self, result: &Self::Output) -> u64 {
        hash_hops(result)
    }
    fn cheap(&self, result: &Self::Output) -> u64 {
        ((result.len() as u64) << 32) ^ result.last().copied().map_or(u64::MAX, u64::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Default)]
    struct DeterministicSampler {
        trace: Option<Rc<RefCell<Vec<&'static str>>>>,
        corrupt_checksum: bool,
    }
    impl EvidenceSampler for DeterministicSampler {
        fn sample(&self, mut query: impl FnMut(bool) -> u64) -> Measurement {
            if let Some(trace) = &self.trace {
                trace.borrow_mut().push("sample");
            }
            let mut samples = Vec::new();
            let mut cheap = None;
            let mut hash = None;
            for ordinal in 0..5 {
                let sentinel = query(false);
                assert_eq!(*cheap.get_or_insert(sentinel), sentinel);
                let checksum = query(true);
                assert_eq!(*hash.get_or_insert(checksum), checksum);
                samples.push(statistics::Sample {
                    ordinal,
                    wall_us: 1.0,
                    cpu_us: None,
                    checksum,
                });
            }
            Measurement {
                warmups: 0,
                repetitions_per_sample: 1,
                samples,
                wall: statistics::Distribution::summarize(&mut [1.0; 5]),
                cpu: None,
                checksum: hash.unwrap() ^ u64::from(self.corrupt_checksum),
            }
        }
    }

    struct ProbeOutput {
        trace: Rc<RefCell<Vec<&'static str>>>,
        value: u64,
    }
    impl Drop for ProbeOutput {
        fn drop(&mut self) {
            self.trace.borrow_mut().push("drop");
        }
    }
    #[derive(Default)]
    struct Probe {
        trace: Rc<RefCell<Vec<&'static str>>>,
        calls: usize,
        corrupt_result: bool,
        corrupt_stats: bool,
    }
    impl EvidenceOperation for Probe {
        type Output = ProbeOutput;
        fn execute(&mut self) -> ProbeOutput {
            self.calls += 1;
            self.trace.borrow_mut().push("execute");
            ProbeOutput {
                trace: self.trace.clone(),
                value: if self.corrupt_result && self.calls == 12 {
                    42
                } else {
                    41
                },
            }
        }
        fn strong(&self, result: &ProbeOutput) -> u64 {
            self.trace.borrow_mut().push("strong");
            result.value
        }
        fn cheap(&self, _: &ProbeOutput) -> u64 {
            self.trace.borrow_mut().push("cheap");
            7
        }
        fn stats(&self) -> Option<SearchStats> {
            self.corrupt_stats.then(|| SearchStats {
                expanded_nodes: if self.calls == 12 { 1 } else { 0 },
                ..SearchStats::default()
            })
        }
    }

    #[test]
    fn protocol_preserves_sampling_hash_and_destruction_boundaries() {
        let mut probe = Probe::default();
        let sampler = DeterministicSampler {
            trace: Some(probe.trace.clone()),
            ..Default::default()
        };
        let run = VerifiedRun::collect(&mut probe, &sampler);
        assert_eq!(run.hash, 41);
        assert_eq!(probe.calls, 12);
        let mut expected = vec!["execute", "strong", "drop", "sample"];
        for _ in 0..5 {
            expected.extend(["execute", "cheap", "drop", "execute", "strong", "drop"]);
        }
        expected.extend(["execute", "strong"]);
        assert_eq!(*probe.trace.borrow(), expected);
        drop(run);
        expected.push("drop");
        assert_eq!(*probe.trace.borrow(), expected);
    }

    #[test]
    #[should_panic(expected = "result hash drift")]
    fn changed_final_result_is_rejected() {
        VerifiedRun::collect(
            &mut Probe {
                corrupt_result: true,
                ..Default::default()
            },
            &DeterministicSampler::default(),
        );
    }

    #[test]
    #[should_panic(expected = "search work drift")]
    fn changed_work_statistics_are_rejected() {
        VerifiedRun::collect(
            &mut Probe {
                corrupt_stats: true,
                ..Default::default()
            },
            &DeterministicSampler::default(),
        );
    }

    #[test]
    #[should_panic(expected = "measurement checksum drift")]
    fn changed_measurement_checksum_is_rejected() {
        VerifiedRun::collect(
            &mut Probe::default(),
            &DeterministicSampler {
                corrupt_checksum: true,
                ..Default::default()
            },
        );
    }

    #[test]
    fn search_and_preparation_operations_keep_results_and_work_counts() {
        let ir = workloads::window_ir(8, 3, true, workloads::SignMode::Alternating);
        let index = build_incidence_index(&ir).unwrap();
        let traversal = HyperTraversal::new(&index);
        let (start, goal) = (NodeId::new(0), NodeId::new(7));
        let mut workspace = InformedSearchWorkspace::new();
        let baseline = hymeko_graph::direct_ir::dijkstra_berge_path(
            &traversal,
            start,
            goal,
            &UnitCost,
            &Undirected,
            &mut workspace,
        )
        .unwrap();
        let baseline_stats = workspace.stats();
        let mut weighted = WeightedQuery {
            case: SearchCase::new(&traversal, start, goal, &mut workspace),
            runner: Dijkstra::new(&UnitCost, &Undirected),
        };
        let run = VerifiedRun::collect(&mut weighted, &DeterministicSampler::default());
        assert_eq!(run.hash, baseline.unwrap().path_hash().raw());
        assert_eq!(run.stats, Some(baseline_stats));
        let mut shortest = ShortestQuery {
            traversal: &traversal,
            start,
            goal,
            workspace: TraversalWorkspace::new(),
        };
        let bfs = VerifiedRun::collect(&mut shortest, &DeterministicSampler::default());
        assert_eq!(
            bfs.result.unwrap().hyperedge_len(),
            run.result.unwrap().hyperedge_len()
        );
        let mut preparation = HopPreparation {
            index: &index,
            goal,
        };
        let prepared = VerifiedRun::collect(&mut preparation, &DeterministicSampler::default());
        assert_eq!(
            prepared.result,
            workloads::hop_distances_from(&ir, &index, goal)
        );
        assert_eq!(prepared.result[ir.nodes[goal.raw()].decl.raw()], 0);
        let disconnected = workloads::multi_component_ir(2, 3, 2);
        let disconnected_index = build_incidence_index(&disconnected).unwrap();
        let disconnected_traversal = HyperTraversal::new(&disconnected_index);
        let mut missing = WeightedQuery {
            case: SearchCase::new(
                &disconnected_traversal,
                NodeId::new(0),
                NodeId::new(5),
                &mut workspace,
            ),
            runner: Dijkstra::new(&UnitCost, &Undirected),
        };
        let absent = VerifiedRun::collect(&mut missing, &DeterministicSampler::default());
        assert!(absent.result.is_none());
        assert_eq!(absent.hash, 0);
        assert_eq!(missing.cheap(&absent.result), 0);
    }

    #[test]
    fn bounded_operations_preserve_frozen_hashes_and_limit_codes() {
        let ir = layered_ir(4);
        let index = build_incidence_index(&ir).unwrap();
        let mut query = EnumerationQuery {
            index: &index,
            start: NodeId::new(0),
            budget: EnumBudget::new(5, 100, 10_000),
            runner: Paths(Some(NodeId::new(ir.nodes.len() - 1))),
        };
        let paths = VerifiedRun::collect(&mut query, &DeterministicSampler::default());
        assert_eq!(paths.hash, 0x5e30e2bae0258b4a);
        assert_eq!(paths.result.items.len(), 16);
        assert_eq!(query.cheap(&paths.result), (16 << 2) ^ 1);
        query.runner = Paths(None);
        let all = VerifiedRun::collect(&mut query, &DeterministicSampler::default());
        assert!(all.result.items.len() > paths.result.items.len());
        query.budget = EnumBudget::new(5, 0, 10_000);
        let empty = VerifiedRun::collect(&mut query, &DeterministicSampler::default());
        assert!(empty.result.items.is_empty());
        assert_eq!(empty.result.bound, Some(BudgetKind::Results));
        assert_eq!(query.cheap(&empty.result), 2);
        query.budget = EnumBudget::new(5, 100, 0);
        let stopped = VerifiedRun::collect(&mut query, &DeterministicSampler::default());
        assert_eq!(stopped.result.bound, Some(BudgetKind::Expansions));
        assert_eq!(query.cheap(&stopped.result), 3);
        let parallel = parallel_ir(6);
        let parallel_index = build_incidence_index(&parallel).unwrap();
        let mut cycles = EnumerationQuery {
            index: &parallel_index,
            start: NodeId::new(0),
            budget: EnumBudget::new(2, 100, 10_000),
            runner: Cycles,
        };
        let run = VerifiedRun::collect(&mut cycles, &DeterministicSampler::default());
        assert_eq!(run.hash, 0xa70363a1887c28e5);
        assert_eq!(run.result.items.len(), 15);
        assert_eq!(cycles.cheap(&run.result), 15 << 2);
    }
}
