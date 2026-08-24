//! The worked example from README.md, kept compiling.
//!
//! A README snippet that no longer builds is worse than none, and this crate's
//! README is not a doctest target.

use hymeko_graph::{CyclePruner, PrunerDecision, SignedGraph, enumerate_simple_cycles};

struct TwoColour {
    colour: Vec<bool>,
}

impl CyclePruner for TwoColour {
    fn extend_ok(&self, path: &[u32], next: u32) -> PrunerDecision {
        match path.last() {
            Some(&tail) if self.colour[tail as usize] == self.colour[next as usize] => {
                PrunerDecision::Reject
            }
            _ => PrunerDecision::Accept,
        }
    }
    fn emit_ok(&self, cycle: &[u32], _signs: &[i8]) -> PrunerDecision {
        PrunerDecision::from_bool(cycle.len().is_multiple_of(2))
    }
}

#[test]
fn the_readme_worked_example_holds() {
    // 4-cycle 0-1-2-3 with a chord 0—2 joining two same-coloured vertices.
    let g = SignedGraph::from_parts(4, &[0, 1, 2, 3, 0], &[1, 2, 3, 0, 2], &[1; 5]);
    let p = TwoColour {
        colour: vec![false, true, false, true],
    };

    // The chord is refused mid-DFS, so no triangle survives.
    assert_eq!(enumerate_simple_cycles(&g, 3, &p).len(), 0);
    // The alternating 4-cycle does.
    assert_eq!(enumerate_simple_cycles(&g, 4, &p), vec![vec![0, 1, 2, 3]]);
}
