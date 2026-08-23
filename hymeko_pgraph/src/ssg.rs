//! Solution Structure Generation (SSG).
//!
//! Given the maximal structure produced by [`crate::msg`], SSG
//! enumerates every **combinatorially feasible** solution structure —
//! a subset $O' \subseteq O_{\max}$ of operating units such that:
//!
//! 1. every required product is producible from the raw set using
//!    only units in $O'$;
//! 2. every input of every $u \in O'$ is raw or produced by some
//!    other unit in $O'$ (consistency);
//! 3. (P-graph "no excess" property) every produced material that is
//!    not a product is consumed by some unit in $O'$.
//!
//! Condition (3) is the strict P-graph rule; relaxing it gives SSG'.
//! We expose both via [`enumerate_with_options`].
//!
//! # Correspondence with the canonical Friedler 1992 axioms
//!
//! (2026-05-19 phase-2 audit.)
//!
//! - **(a) every input raw-or-produced** is **A2/S2 forward**
//!   restricted to inputs of selected units. Equivalent statement:
//!   for every consumed material `m`, either `m ∈ raws` or some
//!   selected unit produces `m` (i.e.\ `m` has an ancestor).
//! - **(b) every required product producible** is the constructive
//!   form of **A1/S1** ∧ **A4/S4** taken together: it asserts that
//!   the selected `O'` actually realises every product through the
//!   production closure, which is the operational counterpart of
//!   "every O-node reaches a product".
//! - **(c) `strict_no_excess`** is the *orthogonal* no-excess
//!   strengthener (not part of S1..S5; called the "strict P-graph
//!   rule" by Friedler 1992).
//!
//! What [`is_feasible`] does not check (lives in the schema layer at
//! [`crate::axioms`]): **A2 reverse** (raws produced inside),
//! **A3/S3** (O-nodes in catalogue), **A5/S5** (no isolated M-nodes).
//!
//! For pedagogical clarity this implementation uses a
//! *generate-and-test* enumeration over $2^{|O_{\max}|}$ subsets. On
//! the small process-synthesis graphs that originally motivated
//! P-graphs ($|O_{\max}| \leq 32$ in the textbook examples) this is
//! ample. ABB ([`crate::abb`]) is the right tool when $|O_{\max}|$ is
//! large.

use std::collections::BTreeSet;

use hymeko::common::ids::DeclId;

use crate::lowering::LoweredPGraph;
use crate::msg::{MaximalStructure, close_producible};

/// One feasible solution structure.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SolutionStructure {
    /// Operating units selected.
    pub units: BTreeSet<DeclId>,
}

/// Knobs controlling SSG.
#[derive(Debug, Clone, Copy)]
pub struct SsgOptions {
    /// Strict P-graph: every produced non-product material must be
    /// consumed by some included unit.  When `false`, "excess"
    /// by-products are allowed (SSG').
    pub strict_no_excess: bool,
    /// Exclude the empty structure even when products are themselves
    /// raw (default `true`; an empty structure is rarely meaningful).
    pub require_at_least_one_unit: bool,
}

impl Default for SsgOptions {
    fn default() -> Self {
        Self {
            // Canonical default (2026-05-27, Pimentel report): excess
            // byproducts are allowed (axioms S1-S5 impose no no-excess
            // rule). Set `true` only for a strict no-waste regime.
            strict_no_excess: false,
            require_at_least_one_unit: true,
        }
    }
}

/// Enumerate every feasible solution structure inside the maximal
/// structure.
pub fn enumerate(p: &LoweredPGraph, msg: &MaximalStructure) -> Vec<SolutionStructure> {
    enumerate_with_options(p, msg, SsgOptions::default())
}

/// Enumerate with explicit options.
pub fn enumerate_with_options(
    p: &LoweredPGraph,
    msg: &MaximalStructure,
    opts: SsgOptions,
) -> Vec<SolutionStructure> {
    let units: Vec<DeclId> = msg.units.iter().copied().collect();
    let n = units.len();
    if n > 30 {
        // Refuse to silently chew through 2^31 subsets.
        // Caller should use ABB instead.
        return Vec::new();
    }

    let mut out: Vec<SolutionStructure> = Vec::new();
    let total: u32 = 1u32 << n;
    for mask in 0u32..total {
        let mut sel: BTreeSet<DeclId> = BTreeSet::new();
        for (i, u) in units.iter().enumerate() {
            if (mask >> i) & 1 == 1 {
                sel.insert(*u);
            }
        }
        if opts.require_at_least_one_unit && sel.is_empty() {
            continue;
        }
        if is_feasible(p, &sel, opts) {
            out.push(SolutionStructure { units: sel });
        }
    }
    out
}

/// Base forward-feasibility, independent of regime: every input of every
/// selected unit is raw or producible from raws using `sel`, and every
/// required product is producible.
pub fn is_feasible_base(p: &LoweredPGraph, sel: &BTreeSet<DeclId>) -> bool {
    // (a) Every input of every selected unit must be raw or produced
    //     by another selected unit.
    let producible = close_producible(p, sel, &p.raws);
    for u in sel {
        for m in p.inputs(*u) {
            if !producible.contains(m) {
                return false;
            }
        }
    }
    // (b) Every required product is producible.
    for prod in &p.products {
        if !producible.contains(prod) {
            return false;
        }
    }
    true
}

/// True iff `sel` is base-feasible *and* admissible under `regime`
/// (e.g. [`NoExcess`](crate::regime::NoExcess) additionally requires the
/// strict no-excess rule). [`Canonical`](crate::regime::Canonical) admits
/// every base-feasible structure.
pub fn is_feasible_with_regime(
    p: &LoweredPGraph,
    sel: &BTreeSet<DeclId>,
    regime: &dyn crate::regime::Regime,
) -> bool {
    is_feasible_base(p, sel) && regime.structure_admissible(p, sel)
}

/// True iff `sel` is a combinatorially feasible solution structure for `p`
/// under `opts`. Adapter over [`is_feasible_with_regime`]: the
/// `strict_no_excess` flag selects the regime (`true → NoExcess`).
pub fn is_feasible(p: &LoweredPGraph, sel: &BTreeSet<DeclId>, opts: SsgOptions) -> bool {
    is_feasible_with_regime(
        p,
        sel,
        crate::regime::from_strict_flag(opts.strict_no_excess),
    )
}
