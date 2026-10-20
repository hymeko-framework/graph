//! Component certificates and bounded selection over the existing IR projection.
use super::RankOrdering;
use crate::GraphicMatroidWorkspace;
use crate::direct_ir::ComponentLabeling;
use hymeko::ir::incidence::{IncidenceLayout, IncidenceOrder, IncidenceProjection};
use hymeko::tensor::contraction::{BoundedProblem, BranchAndBound};
use hymeko::tensor::operator::RowVisitor;
use hymeko::tensor::operator::{RangePartition, SparseRow};

/// Structural planning evidence, not an elapsed-time estimate or speed guarantee.
#[derive(Clone, Copy, Debug)]
pub struct PageRankPlanStats {
    /// Coordinate order selected for speculative range execution.
    pub ordering: IncidenceOrder,
    /// Ranges in the original canonical projection, before any execution layout.
    pub input_ranges: usize,
    /// Node components, including isolated singleton nodes.
    pub components: usize,
    /// Nodes with no outgoing incidence mass.
    pub isolated_nodes: usize,
    /// Rows selected for prefix/difference execution.
    pub range_rows: usize,
    /// Total lossless contiguous ranges in the projection.
    pub ranges: usize,
    /// Ordered gather/scatter incidence work under the declared model.
    pub direct_work: u128,
    /// Selected nominal work including range setup; not elapsed time.
    pub selected_work: u128,
    /// Examined successors in the shared bounded search.
    pub search_states: usize,
    /// Whether selection proved optimality under this nominal cost model.
    pub search_complete: bool,
}
pub(super) struct ExecutionPlan<'ir> {
    pub(super) layout: IncidenceLayout<'ir>,
    pub(super) partition: RangePartition,
    pub(super) range_rows: Vec<bool>,
    pub(super) stats: PageRankPlanStats,
}
pub(super) struct SelectedRows<'a, V> {
    pub(super) visitor: V,
    pub(super) rows: &'a [bool],
    pub(super) selected: bool,
}
impl<V: RowVisitor> RowVisitor for SelectedRows<'_, V> {
    #[inline]
    fn visit<R: SparseRow>(&mut self, row: usize, coordinates: R) {
        if self.rows[row] == self.selected {
            self.visitor.visit(row, coordinates);
        }
    }
}
#[derive(Default)]
struct ComponentCost {
    nodes: usize,
    slots: usize,
    ranges: usize,
}
impl ComponentCost {
    fn direct(&self) -> u128 {
        2 * self.slots as u128
    }
    fn ranges(&self) -> u128 {
        4 * self.ranges as u128 + 3 * self.nodes as u128
    }
}
#[derive(Clone)]
struct Choice {
    ranges: Vec<bool>,
    work: u128,
    uses_ranges: bool,
}
struct StructuralSearch<'a> {
    components: &'a [ComponentCost],
    setup: u128,
}
impl BoundedProblem for StructuralSearch<'_> {
    type State = Choice;
    type Error = std::convert::Infallible;
    fn lower_bound(&self, state: &Choice) -> u128 {
        state.work
            + self.components[state.ranges.len()..]
                .iter()
                .map(|c| c.direct().min(c.ranges()))
                .sum::<u128>()
    }
    fn complete(&self, state: &Choice) -> bool {
        state.ranges.len() == self.components.len()
    }
    fn branches(&self, _: &Choice) -> usize {
        2
    }
    fn branch(&self, state: &Choice, index: usize) -> Result<Option<Choice>, Self::Error> {
        let range = index == 1;
        let component = &self.components[state.ranges.len()];
        let mut next = state.clone();
        next.ranges.push(range);
        next.work += if range {
            component.ranges()
        } else {
            component.direct()
        };
        if range && !next.uses_ranges {
            next.work += self.setup;
        }
        next.uses_ranges |= range;
        Ok(Some(next))
    }
    fn order(&self, a: &Choice, b: &Choice) -> std::cmp::Ordering {
        self.lower_bound(a).cmp(&self.lower_bound(b))
    }
}
impl<'ir> ExecutionPlan<'ir> {
    pub(super) fn build(
        projection: &IncidenceProjection<'ir>,
        outgoing: &[f64],
        ordering: RankOrdering,
    ) -> Option<Self> {
        if ordering != RankOrdering::FirstEncounter {
            if let Some(plan) =
                Self::from_layout(projection.layout(IncidenceOrder::Canonical), outgoing)
            {
                return Some(plan);
            }
            if ordering == RankOrdering::Canonical {
                return None;
            }
        }
        // Pair-heavy and empty inputs have no range-work dividend to pay for a
        // new ordering. Keep their existing allocation and arithmetic paths.
        if projection.incidence_count() == 0
            || projection.incidence_count() < projection.arc_count().saturating_mul(4)
        {
            return None;
        }
        Self::from_layout(projection.layout(IncidenceOrder::FirstEncounter), outgoing)
    }
    fn from_layout(layout: IncidenceLayout<'ir>, outgoing: &[f64]) -> Option<Self> {
        let projection = layout.projection();
        // Cheap metadata admission: never scan indexed incidence lists just to
        // discover that range algebra cannot reduce their work.
        let ranges = layout.range_count();
        if 4 * ranges as u128 + 3 * outgoing.len() as u128
            >= 2 * projection.incidence_count() as u128
        {
            return None;
        }
        let labels =
            ComponentLabeling::from_projection(projection, &mut GraphicMatroidWorkspace::new())
                .ok()?;
        let mut compact = vec![usize::MAX; labels.count() as usize];
        let mut components = Vec::<ComponentCost>::new();
        let mut isolated_nodes = 0;
        for (node, &degree) in outgoing.iter().enumerate() {
            if degree == 0.0 {
                isolated_nodes += 1;
                continue;
            }
            let label = labels.labels()[node] as usize;
            if compact[label] == usize::MAX {
                compact[label] = components.len();
                components.push(ComponentCost::default());
            }
            components[compact[label]].nodes += 1;
        }
        for row in layout.rows() {
            if let Some(first) = row.iter_targets().next() {
                let canonical = layout.canonical_index(first);
                let component = &mut components[compact[labels.labels()[canonical] as usize]];
                component.slots += row.len();
                row.for_each_range(|_| component.ranges += 1);
            }
        }
        let direct_work = components.iter().map(ComponentCost::direct).sum();
        let greedy: Vec<_> = components.iter().map(|c| c.ranges() < c.direct()).collect();
        let greedy_work = components
            .iter()
            .zip(&greedy)
            .map(|(c, &r)| if r { c.ranges() } else { c.direct() })
            .sum::<u128>()
            + if greedy.iter().any(|&r| r) {
                outgoing.len() as u128
            } else {
                0
            };
        let incumbent = if greedy_work < direct_work {
            Choice {
                ranges: greedy,
                work: greedy_work,
                uses_ranges: true,
            }
        } else {
            Choice {
                ranges: vec![false; components.len()],
                work: direct_work,
                uses_ranges: false,
            }
        };
        let search = StructuralSearch {
            components: &components,
            setup: outgoing.len() as u128,
        };
        let outcome = match BranchAndBound::solve(
            &search,
            Choice {
                ranges: Vec::new(),
                work: 0,
                uses_ranges: false,
            },
            Some(incumbent),
            512,
        ) {
            Ok(outcome) => outcome,
            Err(never) => match never {},
        };
        let best = outcome.best?;
        if !best.uses_ranges {
            return None;
        }
        let selected = |node: usize| {
            let c = compact[labels.labels()[node] as usize];
            c != usize::MAX && best.ranges[c]
        };
        let mut blocks = Vec::new();
        let mut node = 0;
        while node < outgoing.len() {
            if !selected(layout.canonical_index(node)) {
                node += 1;
                continue;
            }
            let start = node;
            let label = labels.labels()[layout.canonical_index(node)];
            node += 1;
            while node < outgoing.len() && labels.labels()[layout.canonical_index(node)] == label {
                node += 1;
            }
            blocks.push(start..node);
        }
        let range_rows: Vec<_> = layout
            .rows()
            .map(|r| {
                r.iter_targets()
                    .next()
                    .is_some_and(|node| selected(layout.canonical_index(node)))
            })
            .collect();
        let stats = PageRankPlanStats {
            ordering: layout.order(),
            input_ranges: projection.range_count(),
            components: labels.count() as usize,
            isolated_nodes,
            range_rows: range_rows.iter().filter(|&&v| v).count(),
            ranges,
            direct_work,
            selected_work: best.work,
            search_states: outcome.states,
            search_complete: outcome.complete,
        };
        Some(Self {
            layout,
            partition: RangePartition::new(outgoing.len(), blocks)?,
            range_rows,
            stats,
        })
    }
    pub(super) fn retained_bytes(&self) -> usize {
        self.partition.retained_bytes()
            + self.range_rows.capacity()
            + self.layout.additional_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn structural_bounds_match_exhaustive_component_choices_with_shared_setup() {
        for seed in 0..30 {
            let components: Vec<_> = (0..6)
                .map(|i| ComponentCost {
                    nodes: 1 + (seed + i) % 11,
                    slots: 1 + (seed * 7 + i * 13) % 80,
                    ranges: 1 + (seed + i * 3) % 16,
                })
                .collect();
            let search = StructuralSearch {
                components: &components,
                setup: seed as u128 * 3,
            };
            let mut optimum = u128::MAX;
            for mask in 0..64 {
                let mut s = Choice {
                    ranges: Vec::new(),
                    work: 0,
                    uses_ranges: false,
                };
                for i in 0..6 {
                    s = search.branch(&s, (mask >> i) & 1).unwrap().unwrap();
                }
                optimum = optimum.min(s.work);
            }
            let result = BranchAndBound::solve(
                &search,
                Choice {
                    ranges: Vec::new(),
                    work: 0,
                    uses_ranges: false,
                },
                None,
                1000,
            )
            .unwrap();
            assert!(result.complete);
            assert_eq!(result.best.unwrap().work, optimum);
        }
    }
}
