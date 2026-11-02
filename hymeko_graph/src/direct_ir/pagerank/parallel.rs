//! Parallel scheduling of the same ordered gathers and transpose reductions.
use super::{PageRank, PageRankError, PageRankWorkspace, PreparedRates, RankTransfer, Sum};
use hymeko::common::ids::HyperArcId;
use hymeko::ir::incidence::{IncidenceIndex, build_incidence_index};
use hymeko::tensor::operator::{OrderedAccumulator, RowGather};
use rayon::prelude::*;

impl<'ir> PageRank<'ir> {
    pub(super) fn reverse_index(&self) -> Result<&IncidenceIndex<'ir>, PageRankError> {
        self.reverse
            .get_or_init(|| build_incidence_index(self.projection.ir()))
            .as_ref()
            .map_err(|error| PageRankError::ReverseIndex(*error))
    }

    pub(super) fn update_parallel(
        &self,
        damping: f64,
        workspace: &mut PageRankWorkspace,
        reverse: &IncidenceIndex<'ir>,
    ) -> Result<f64, PageRankError> {
        let position = |arc, slot: u32| self.projection.row(arc).offset() + slot as usize;
        match &self.rates {
            PreparedRates::Unit => {
                self.distribute_parallel(damping, workspace, reverse, |_| 1.0, |_, _| 1.0, false)
            }
            PreparedRates::Directed(roles) => self.distribute_parallel(
                damping,
                workspace,
                reverse,
                |slot| f64::from(roles[slot] & 1),
                |arc, slot| f64::from((roles[position(arc, slot)] >> 1) & 1),
                false,
            ),
            PreparedRates::Weighted(rates) => self.distribute_parallel(
                damping,
                workspace,
                reverse,
                |slot| rates[slot].source,
                |arc, slot| rates[position(arc, slot)].destination,
                true,
            ),
        }
        self.finish_transition(damping, workspace, self.rates.weighted())
    }

    fn distribute_parallel(
        &self,
        damping: f64,
        workspace: &mut PageRankWorkspace,
        reverse: &IncidenceIndex<'ir>,
        source: impl Fn(usize) -> f64 + Sync,
        sink: impl Fn(HyperArcId, u32) -> f64 + Sync,
        normalized: bool,
    ) {
        let rows = self.projection.arc_count();
        let grain = rows.div_ceil(workspace.worker_count()).max(1);
        workspace.arc_values.resize(rows, 0.0);
        let input = &workspace.scaled;
        workspace
            .arc_values
            .par_chunks_mut(grain)
            .enumerate()
            .for_each(|(chunk, output)| {
                let first = chunk * grain;
                let end = first + output.len();
                let mut gather = RowGather::new(
                    input,
                    output,
                    &source,
                    RankTransfer {
                        damping,
                        normalized,
                        destinations: &self.destinations,
                    },
                )
                .with_gather::<Sum>()
                .starting_at(first);
                self.projection.for_each_row_range(first..end, &mut gather);
            });
        let arc_values = &workspace.arc_values;
        workspace
            .incoming
            .par_iter_mut()
            .enumerate()
            .for_each(|(node, output)| {
                let declaration = self.projection.ir().nodes[node].decl;
                let slots = reverse.incident_slots(declaration);
                let value =
                    reverse.transpose_row::<Sum>(declaration, arc_values, |ordinal, arc| {
                        sink(arc, slots[ordinal])
                    });
                // Only the completed finite nonnegative total is consumed by the
                // shared vertex sweep. Preserve its bits without another reduction.
                *output = Sum::from_first(value);
            });
    }
}
