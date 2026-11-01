//! Certified mass-coordinate execution view; the canonical IR is unchanged.
use super::{PageRank, PreparedRates};
use hymeko_numerics::Operator;
use std::cell::RefCell;

/// Optional exact-arithmetic reduction of the Krylov coordinate space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageRankQuotient {
    /// Keep the full operator domain and existing selection policy.
    #[default]
    Disabled,
    /// Consider isolates and same-hyperarc degree-one vertices under unit,
    /// undirected routing. Personalization must be constant inside each class.
    /// Admission includes setup/lifting work; final validation uses the full IR.
    Pendant,
}

pub(super) struct QuotientPlan {
    class_of: Vec<usize>,
    representatives: Vec<usize>,
    sizes: Vec<usize>,
    setup_work: u128,
}

impl QuotientPlan {
    pub(super) fn build(rank: &PageRank<'_>) -> Option<Self> {
        if !matches!(rank.rates, PreparedRates::Unit) || rank.node_count() < 2 {
            return None;
        }
        let nodes = rank.node_count();
        let isolates = rank.outgoing.iter().filter(|&&d| d == 0.0).count();
        let leaves = rank.outgoing.iter().filter(|&&d| d == 1.0).count();
        // At most one pendant class per arc. This is a guaranteed dimension
        // saving, not a sampled signature or an assumed equality of degrees.
        let group_leaves = leaves > rank.destinations.len();
        let saved = isolates.saturating_sub(1)
            + if group_leaves {
                leaves - rank.destinations.len()
            } else {
                0
            };
        if 4 * (saved as u128) < nodes as u128 {
            return None;
        }
        let mut plan = Self {
            class_of: vec![usize::MAX; nodes],
            representatives: Vec::with_capacity(nodes - saved),
            sizes: Vec::with_capacity(nodes - saved),
            setup_work: 3 * nodes as u128
                + if group_leaves {
                    rank.incidence_count() as u128
                } else {
                    0
                },
        };
        let mut isolated_class = None;
        for (node, &degree) in rank.outgoing.iter().enumerate() {
            if degree == 0.0 {
                let class = *isolated_class.get_or_insert_with(|| plan.add_class(node));
                plan.assign(node, class);
            } else if degree != 1.0 || !group_leaves {
                let class = plan.add_class(node);
                plan.assign(node, class);
            }
        }
        if group_leaves {
            for row in rank.projection.rows() {
                let mut pendant_class = None;
                for node in row.iter_targets() {
                    if rank.outgoing[node] == 1.0 {
                        let class = *pendant_class.get_or_insert_with(|| plan.add_class(node));
                        plan.assign(node, class);
                    }
                }
            }
        }
        debug_assert!(plan.class_of.iter().all(|&c| c < plan.nodes()));
        Some(plan)
    }

    fn add_class(&mut self, representative: usize) -> usize {
        let class = self.representatives.len();
        self.representatives.push(representative);
        self.sizes.push(0);
        class
    }

    fn assign(&mut self, node: usize, class: usize) {
        debug_assert_eq!(self.class_of[node], usize::MAX);
        self.class_of[node] = class;
        self.sizes[class] += 1;
    }

    pub(super) fn nodes(&self) -> usize {
        self.sizes.len()
    }

    pub(super) fn compatible(&self, personalization: &[f64]) -> bool {
        self.class_of.iter().enumerate().all(|(node, &class)| {
            personalization[node].to_bits()
                == personalization[self.representatives[class]].to_bits()
        })
    }

    pub(super) fn retained_bytes(&self) -> usize {
        (self.class_of.capacity() + self.representatives.capacity() + self.sizes.capacity())
            * size_of::<usize>()
    }

    pub(super) fn extra_work(&self, directions: usize) -> u128 {
        let n = self.class_of.len() as u128;
        let q = self.nodes() as u128;
        // Per operator application: scale class masses, lift and restrict.
        // Also charge partitioning and initial/final coordinate transforms.
        self.setup_work + (directions as u128 + 2) * (2 * n + 3 * q) + 2 * n + 2 * q
    }

    pub(super) fn restrict(&self, full: &[f64], reduced: &mut [f64]) {
        for ((value, &node), &size) in reduced
            .iter_mut()
            .zip(&self.representatives)
            .zip(&self.sizes)
        {
            *value = full[node] * size as f64;
        }
    }

    pub(super) fn lift(&self, reduced: &[f64], full: &mut [f64], scaled: &mut [f64]) {
        for ((value, &mass), &size) in scaled.iter_mut().zip(reduced).zip(&self.sizes) {
            *value = mass / size as f64;
        }
        for (value, &class) in full.iter_mut().zip(&self.class_of) {
            *value = scaled[class];
        }
    }
}

#[derive(Default)]
pub(super) struct QuotientWorkspace {
    pub(super) solution: Vec<f64>,
    pub(super) rhs: Vec<f64>,
    pub(super) lifted: Vec<f64>,
    pub(super) image: Vec<f64>,
}
impl QuotientWorkspace {
    pub(super) fn prepare(&mut self, nodes: usize, groups: usize) {
        self.solution.resize(groups, 0.0);
        self.rhs.resize(groups, 0.0);
        self.lifted.resize(nodes, 0.0);
        self.image.resize(nodes, 0.0);
    }
    pub(super) fn retained_bytes(&self) -> usize {
        (self.solution.capacity()
            + self.rhs.capacity()
            + self.lifted.capacity()
            + self.image.capacity())
            * size_of::<f64>()
    }
}

/// Q = R T L. T remains the existing incidence/range operator, including dangling
/// mass. The proof applies to signed Krylov vectors, not just probabilities.
pub(super) struct QuotientTransition<'a, O> {
    pub(super) plan: &'a QuotientPlan,
    pub(super) original: &'a O,
    pub(super) buffers: RefCell<(&'a mut [f64], &'a mut [f64])>,
}
impl<O: Operator> Operator for QuotientTransition<'_, O> {
    fn dim(&self) -> usize {
        self.plan.nodes()
    }
    fn apply(&self, input: &[f64], output: &mut [f64]) {
        let mut buffers = self.buffers.borrow_mut();
        let (lifted, image) = &mut *buffers;
        self.plan.lift(input, lifted, output);
        self.original.apply(lifted, image);
        self.plan.restrict(image, output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PageRankModel, ir_test_support::Case};
    use hymeko::ir::ir::IrBuilder;

    #[test]
    fn dimension_admission_has_an_inclusive_quarter_saving_boundary() {
        for active in [5, 6] {
            let rows: Vec<_> = (0..active - 1).map(|i| vec![i, i + 1]).collect();
            let borrowed: Vec<_> = rows.iter().map(Vec::as_slice).collect();
            let ir = Case::unsigned(8, &borrowed).build().unwrap();
            let rank = PageRank::build(&ir, PageRankModel::default()).unwrap();
            let plan = QuotientPlan::build(&rank);
            // Three isolates save two coordinates (admit); two save one (skip).
            assert_eq!(plan.is_some(), active == 5);
            if let Some(plan) = plan {
                assert_eq!(plan.nodes(), 6);
            }
        }
    }
}
