//! Maximal Structure Generation (MSG) — Friedler et al. 1992.
//!
//! Given a P-graph $(M, O, E)$ with a raw set $R \subseteq M$ and a
//! product set $P \subseteq M$, the **maximal structure** is the
//! sub-P-graph that contains exactly those operating units that are
//! *both* forward-feasible (every input is raw or produced by some
//! surviving unit) *and* backward-feasible (every output reaches a
//! product through some surviving unit), iterated to a fixpoint.
//!
//! Every combinatorially feasible solution structure is a subgraph of
//! the maximal structure, so MSG is the obligatory pre-pass for SSG /
//! ABB and prunes operating units that cannot participate in any
//! feasible solution.
//!
//! # Correspondence with the canonical Friedler 1992 axioms
//!
//! (Established in the 2026-05-19 phase-2 audit; see
//! `reports/2026-05-19-pgraph-axiom-semantics-fix-phase2.md`.)
//!
//! - The **forward pass** ("every input must be raw or producible")
//!   enforces **A2/S2 forward direction** restricted to materials
//!   that are inputs of surviving units: any consumed material that
//!   is neither raw nor produced is rejected (so the surviving
//!   M-nodes that are *inputs* satisfy `no-ancestor ⟹ raw`).
//! - The **backward pass** enforces **A4/S4**: every surviving
//!   O-node has a path to some product material. The fixpoint
//!   iteration is the standard backward-reachability closure.
//! - The **`strict_no_excess` strengthener** is an *orthogonal*
//!   condition (Friedler 1992 calls this the "strict P-graph
//!   rule"): every output of every surviving unit must be product
//!   or consumed. It is **not** part of S1..S5 — it is an
//!   additional waste-free constraint.
//!
//! What MSG does **not** check (these live at the schema layer in
//! [`crate::axioms`]):
//!
//! - **A1/S1** (products are M-nodes): preserved by construction of
//!   `LoweredPGraph`.
//! - **A2/S2 reverse direction** (raws must not be produced inside):
//!   relies on the `LoweredPGraph` having no `u → r` edges for
//!   `r ∈ raws`. Verified at schema construction via
//!   [`crate::axioms::AxiomBundle`].
//! - **A3/S3** (O-nodes in catalogue): schema-layer concern.
//! - **A5/S5** (every M-node has ≥1 incident edge): schema-layer
//!   concern; MSG never reasons about stray M-nodes.

use std::collections::BTreeSet;

use hymeko::common::ids::DeclId;

use crate::lowering::LoweredPGraph;

/// Output of MSG: the surviving units and the materials they produce
/// or consume.
#[derive(Debug, Clone)]
pub struct MaximalStructure {
    /// Operating units in the maximal structure.
    pub units: BTreeSet<DeclId>,
    /// Materials touched by the maximal structure (raw, produced, or
    /// consumed by surviving units, plus the required products).
    pub materials: BTreeSet<DeclId>,
}

/// MSG knobs.
///
/// **Correction 2026-05-27 (Pimentel report).** The default is now the
/// **canonical Friedler maximal structure** (reduction + composition,
/// book Ch.4): a unit survives iff it is forward-feasible (every input
/// is raw or produced by some surviving unit — *cycles are admitted*)
/// **and** backward-reachable from a product. No "no-excess" rule is
/// applied — it is not among axioms S1–S5. The pre-2026-05-27 default
/// (`strict_no_excess = true`) was non-canonical: it imposed a
/// no-excess rule in the backward pass and used a raw-reachability
/// forward pass that wrongly dropped structurally-valid cycles,
/// yielding too-small / empty maximal structures (e.g. book Example 4.1
/// gave 3 units instead of 7; Example 3.3 gave 0 instead of 29).
///
/// `strict_no_excess = true` is retained only as an explicit,
/// **non-canonical** "no-waste" post-filter on top of the canonical
/// structure: it additionally drops units that produce any material
/// that is neither a product nor consumed by another surviving unit.
/// Use it only when modelling a strict no-waste regime; it does **not**
/// reproduce the textbook maximal structures.
#[derive(Debug, Clone, Copy, Default)]
pub struct MaximalStructureOptions {
    /// When `false` (default) the canonical maximal structure is
    /// returned. When `true`, an extra non-canonical no-waste filter is
    /// applied. See type-level docs.
    pub strict_no_excess: bool,
}

/// Back-compat shim: the canonical maximal structure
/// ([`maximal_structure_with_options`] with the default options).
pub fn maximal_structure(p: &LoweredPGraph) -> MaximalStructure {
    maximal_structure_with_options(p, MaximalStructureOptions::default())
}

/// Adapter: compute the maximal structure under the `strict_no_excess`
/// option by mapping it to a [`Regime`](crate::regime) and delegating to
/// [`maximal_structure_with_regime`]. `false → Canonical` (the textbook
/// maximal structure), `true → NoExcess` (canonical + no-waste filter).
pub fn maximal_structure_with_options(
    p: &LoweredPGraph,
    opts: MaximalStructureOptions,
) -> MaximalStructure {
    maximal_structure_with_regime(p, crate::regime::from_strict_flag(opts.strict_no_excess))
}

/// Compute the maximal structure by the canonical Friedler **reduction
/// + composition** procedure (book Ch.4), then apply `regime`'s refinement.
///
/// # Preconditions
/// `p` is a lowered P-graph; `p.raws`, `p.products`, and the incidence
/// queries [`LoweredPGraph::inputs`]/[`outputs`](LoweredPGraph::outputs)
/// are consistent with the schema.
///
/// # Postconditions
/// Returns the smallest superstructure containing every solution-structure
/// (axioms S1–S5): a unit is included iff it is forward-feasible (reduction)
/// **and** backward-reachable from a product (composition); structurally
/// valid cycles are retained. The canonical result is then passed through
/// [`Regime::refine_maximal`](crate::regime::Regime::refine_maximal)
/// ([`Canonical`](crate::regime::Canonical) is the identity). If no unit
/// produces a product the result is empty (degenerate maximal structure).
pub fn maximal_structure_with_regime(
    p: &LoweredPGraph,
    regime: &dyn crate::regime::Regime,
) -> MaximalStructure {
    // ── Reduction phase (forward feasibility; admits cycles) ──
    //
    // Iterate to a fixpoint, removing (1) units that produce a raw
    // material (axiom S2: raws have no producer), and (2) units with an
    // input that is neither raw nor produced by *any* surviving unit.
    // Availability uses "produced by some survivor" — NOT
    // reachability-from-raws — so a structurally valid cycle (whose
    // members mutually produce each other's inputs) survives.
    let mut units: BTreeSet<DeclId> = p.units.clone();
    loop {
        let before = units.len();
        units.retain(|u| !p.outputs(*u).iter().any(|m| p.raws.contains(m)));
        let mut available: BTreeSet<DeclId> = p.raws.clone();
        for u in &units {
            available.extend(p.outputs(*u).iter().copied());
        }
        units.retain(|u| p.inputs(*u).iter().all(|m| available.contains(m)));
        if units.len() == before {
            break;
        }
    }

    // ── Composition phase (backward reachability from products) ──
    //
    // Least fixpoint: a unit is kept iff it produces a required material;
    // including it makes its non-raw inputs required in turn. Forward-
    // feasible units that reach no product (dead-end producers) are
    // dropped here.
    let mut kept: BTreeSet<DeclId> = BTreeSet::new();
    let mut needed: BTreeSet<DeclId> = p.products.clone();
    loop {
        let new: Vec<DeclId> = units
            .iter()
            .copied()
            .filter(|u| !kept.contains(u) && p.outputs(*u).iter().any(|m| needed.contains(m)))
            .collect();
        if new.is_empty() {
            break;
        }
        for u in new {
            kept.insert(u);
            needed.extend(p.inputs(u).iter().copied().filter(|m| !p.raws.contains(m)));
        }
    }
    // ── Regime refinement (Canonical = identity; NoExcess = no-waste filter) ──
    let units = regime.refine_maximal(p, kept);

    // Materials touched by the surviving units, plus the required products.
    let mut materials: BTreeSet<DeclId> = BTreeSet::new();
    for u in &units {
        materials.extend(p.inputs(*u).iter().copied());
        materials.extend(p.outputs(*u).iter().copied());
    }
    materials.extend(p.products.iter().copied());

    MaximalStructure { units, materials }
}

/// Forward closure: smallest set $C \supseteq R$ such that whenever a
/// unit in `units` has all its inputs in $C$, its outputs are added.
pub fn close_producible(
    p: &LoweredPGraph,
    units: &BTreeSet<DeclId>,
    raws: &BTreeSet<DeclId>,
) -> BTreeSet<DeclId> {
    let mut c: BTreeSet<DeclId> = raws.clone();
    loop {
        let before = c.len();
        for u in units {
            if p.inputs(*u).iter().all(|m| c.contains(m)) {
                c.extend(p.outputs(*u).iter().copied());
            }
        }
        if c.len() == before {
            break;
        }
    }
    c
}

/// Backward closure: smallest set $C \supseteq \mathit{products}$
/// such that whenever a unit in `units` has *any* output in $C$, its
/// inputs are added (they are needed to produce something useful).
pub fn close_consumable(
    p: &LoweredPGraph,
    units: &BTreeSet<DeclId>,
    products: &BTreeSet<DeclId>,
) -> BTreeSet<DeclId> {
    let mut c: BTreeSet<DeclId> = products.clone();
    loop {
        let before = c.len();
        for u in units {
            if p.outputs(*u).iter().any(|m| c.contains(m)) {
                c.extend(p.inputs(*u).iter().copied());
            }
        }
        if c.len() == before {
            break;
        }
    }
    c
}
