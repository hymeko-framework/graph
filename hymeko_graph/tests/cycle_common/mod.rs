//! Shared cycle test execution and explicit comparison policies.
//! Runners call public APIs; strategies inspect outputs independently of the engine.
#![allow(dead_code)] // each integration target selects its own runners/strategies

use hymeko_graph::{
    SignedGraph,
    pruner::CyclePruner,
    topk_cycles::{TopKCycle, scorers},
};
use std::collections::BTreeSet;

pub struct CycleCase<'a, P> {
    pub graph: &'a SignedGraph,
    pub pruner: &'a P,
    pub length: usize,
    pub keep: usize,
}

pub trait CycleRunner<P: CyclePruner + Sync> {
    fn run(&self, case: &CycleCase<'_, P>) -> Vec<TopKCycle>;
}

pub struct SerialPerVertex;
pub struct ParallelPerVertex;
pub struct AdaptivePerVertex<'a>(pub &'a [u32]);

impl<P: CyclePruner + Sync> CycleRunner<P> for SerialPerVertex {
    fn run(&self, c: &CycleCase<'_, P>) -> Vec<TopKCycle> {
        hymeko_graph::enumerate_top_k_per_vertex_cycles(
            c.graph,
            c.length,
            c.pruner,
            c.keep,
            scorers::fraction_negative,
        )
    }
}

impl<P: CyclePruner + Sync> CycleRunner<P> for ParallelPerVertex {
    fn run(&self, c: &CycleCase<'_, P>) -> Vec<TopKCycle> {
        hymeko_graph::enumerate_top_k_per_vertex_cycles_par(
            c.graph,
            c.length,
            c.pruner,
            c.keep,
            scorers::fraction_negative,
        )
    }
}

impl<P: CyclePruner + Sync> CycleRunner<P> for AdaptivePerVertex<'_> {
    fn run(&self, c: &CycleCase<'_, P>) -> Vec<TopKCycle> {
        hymeko_graph::enumerate_top_k_per_vertex_cycles_par_adaptive(
            c.graph,
            c.length,
            c.pruner,
            self.0,
            scorers::fraction_negative,
        )
    }
}

pub trait TestStrategy {
    fn verify(&self, length: usize, left: &[TopKCycle], right: &[TopKCycle]);
}

impl<P: CyclePruner + Sync> CycleCase<'_, P> {
    pub fn compare(
        &self,
        left: &impl CycleRunner<P>,
        right: &impl CycleRunner<P>,
        strategy: &impl TestStrategy,
    ) {
        strategy.verify(self.length, &left.run(self), &right.run(self));
    }
}

pub struct CardinalityParity;
impl TestStrategy for CardinalityParity {
    fn verify(&self, _: usize, left: &[TopKCycle], right: &[TopKCycle]) {
        assert_eq!(left.len(), right.len(), "cycle cardinality differs");
    }
}

/// Existing CSR tests permit boundary-score ties but require simple balanced
/// cycles, bounded cardinality drift and (when requested) overlapping memberships.
pub struct StructuralParity {
    pub balanced: bool,
    pub max_count_drift: f64,
    pub min_overlap: Option<f64>,
}

impl StructuralParity {
    fn check_cycles(&self, length: usize, cycles: &[TopKCycle]) {
        assert!(!cycles.is_empty(), "fixture should produce cycles");
        for (_, vertices, signs) in cycles {
            assert_eq!(vertices.len(), length, "cycle length");
            assert_eq!(signs.len(), length, "sign count");
            let distinct: BTreeSet<_> = vertices.iter().copied().collect();
            assert_eq!(distinct.len(), length, "cycle must be simple");
            if self.balanced {
                assert_eq!(
                    signs.iter().map(|&s| i32::from(s)).product::<i32>(),
                    1,
                    "cycle must be balanced"
                );
            }
        }
    }

    fn canonical(cycles: &[TopKCycle]) -> BTreeSet<Vec<u32>> {
        cycles
            .iter()
            .map(|(_, vertices, _)| {
                let mut vertices = vertices.clone();
                vertices.sort_unstable();
                vertices
            })
            .collect()
    }
}

impl TestStrategy for StructuralParity {
    fn verify(&self, length: usize, left: &[TopKCycle], right: &[TopKCycle]) {
        self.check_cycles(length, left);
        self.check_cycles(length, right);
        let drift = left.len().abs_diff(right.len()) as f64 / left.len().max(right.len()) as f64;
        assert!(
            drift < self.max_count_drift,
            "cycle-count drift {drift} exceeds {}",
            self.max_count_drift
        );
        if let Some(minimum) = self.min_overlap {
            let a = Self::canonical(left);
            let b = Self::canonical(right);
            let overlap = a.intersection(&b).count() as f64 / a.len() as f64;
            assert!(
                overlap > minimum,
                "cycle-set overlap {overlap} is below {minimum}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategies_reject_corrupted_outputs() {
        let good = vec![(0.0, vec![0, 1, 2], vec![1, 1, 1])];
        let strategy = StructuralParity {
            balanced: true,
            max_count_drift: 0.03,
            min_overlap: Some(0.85),
        };
        strategy.verify(3, &good, &good);
        let bad = [
            Vec::new(),
            vec![(0., vec![0, 0, 2], vec![1, 1, 1])],
            vec![(0., vec![0, 1, 2], vec![-1, 1, 1])],
            vec![(0., vec![3, 4, 5], vec![1, 1, 1])],
            vec![(0., vec![0, 1], vec![1, 1])],
            vec![(0., vec![0, 1, 2], vec![1, 1])],
            vec![good[0].clone(), good[0].clone()],
        ];
        for result in bad {
            assert!(std::panic::catch_unwind(|| strategy.verify(3, &good, &result)).is_err());
        }
        assert!(std::panic::catch_unwind(|| CardinalityParity.verify(3, &good, &[])).is_err());
        StructuralParity {
            balanced: false,
            min_overlap: None,
            ..strategy
        }
        .verify(3, &good, &good);
    }
}
