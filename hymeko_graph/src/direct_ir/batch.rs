//! Batch reachability from many start nodes -- serial and Rayon.
//!
//! Each start's reachable-set size is independent, so the batch is embarrassingly parallel. The
//! serial form reuses one [`TraversalWorkspace`] across starts (warm, zero per-query allocation);
//! the Rayon form uses a **thread-local** workspace via `map_init`, so within a worker the queries
//! are warm too. Both preserve input order (`out[i]` is `starts[i]`'s result), so the serial and
//! parallel results are identical -- parallelism changes speed, never the answer.

use rayon::prelude::*;

use hymeko::common::ids::NodeId;
use hymeko::ir::incidence::IncidenceIndex;

use super::DirectIrError;
use super::traversal::HyperTraversal;
use super::workspace::TraversalWorkspace;

/// Reachable-set size from every start, serial, reusing one workspace.
///
/// # Errors
/// The first [`DirectIrError`] encountered (invalid start or malformed arc reference).
pub fn batch_reachable_counts(
    index: &IncidenceIndex<'_>,
    starts: &[NodeId],
) -> Result<Vec<usize>, DirectIrError> {
    let trav = HyperTraversal::new(index);
    let mut ws = TraversalWorkspace::new();
    let mut out = Vec::with_capacity(starts.len());
    for &s in starts {
        out.push(trav.reachable_nodes(&mut ws, s)?.len());
    }
    Ok(out)
}

/// Reachable-set size from every start, in parallel with per-worker thread-local workspaces. The
/// result is identical to [`batch_reachable_counts`] (same order, same values).
///
/// # Errors
/// The first [`DirectIrError`] encountered across the batch.
pub fn par_batch_reachable_counts(
    index: &IncidenceIndex<'_>,
    starts: &[NodeId],
) -> Result<Vec<usize>, DirectIrError> {
    let trav = HyperTraversal::new(index);
    starts
        .par_iter()
        .map_init(TraversalWorkspace::new, |ws, &s| {
            trav.reachable_nodes(ws, s).map(|r| r.len())
        })
        .collect()
}
