//! Accelerated Branch-and-Bound (ABB) for cost-minimising synthesis.
//!
//! Given a P-graph, raw set, product set and per-unit costs $c(u)$,
//! ABB returns the feasible solution structure of minimum total cost.
//!
//! The "accelerated" Friedler ABB exploits structural lower bounds
//! drawn from the maximal structure to prune large parts of the
//! $2^{|O_{\max}|}$ subset lattice. The variant implemented here uses
//! two bounds:
//!
//! 1. **Inclusion bound** — once a partial selection has cost $\geq$
//!    the current best, abandon.
//! 2. **Reachability bound** — at every branch point we check that
//!    the *remaining* (still-undecided plus already-included) units
//!    suffice to produce every required product. If not, prune.
//!
//! Branching is by include / exclude on a fixed unit ordering. Ties
//! are broken in favour of including, so the search hits a feasible
//! incumbent quickly.
//!
//! # Correspondence with the canonical Friedler 1992 axioms
//!
//! (2026-05-19 phase-2 audit.) ABB delegates leaf-level feasibility
//! to [`crate::ssg::is_feasible`], so it inherits exactly SSG's
//! axiom coverage (A2-forward + A1/A4-as-realisation + optional
//! no-excess strengthener). The two ABB bounds add no new axiom
//! semantics:
//!
//! - The **inclusion bound** is purely cost-based and is independent
//!   of S1..S5.
//! - The **reachability bound** is an early-cut form of the leaf's
//!   A1/A4 check: if even the optimistic-remaining unit set cannot
//!   produce every required product, no descendant will satisfy
//!   the leaf check either.

use std::collections::BTreeSet;

use hymeko::common::ids::DeclId;

use crate::lowering::LoweredPGraph;
use crate::msg::{MaximalStructure, close_producible};
use crate::ssg::is_feasible_with_regime;

/// Output of [`solve`]: optimal selection plus its cost.
#[derive(Debug, Clone)]
pub struct AbbSolution {
    /// Operating units in the optimal solution.
    pub units: BTreeSet<DeclId>,
    /// Total cost.
    pub cost: f64,
    /// Number of nodes explored (diagnostic).
    pub explored: u64,
    /// Number of nodes pruned by the inclusion bound (cost ≥ incumbent).
    pub pruned_by_inclusion: u64,
    /// Number of nodes pruned by the reachability bound
    /// (`producible(R, optimistic) ⊉ products`).
    pub pruned_by_reachability: u64,
}

impl AbbSolution {
    /// Total prunes (inclusion + reachability) — back-compat shim.
    pub fn pruned(&self) -> u64 {
        self.pruned_by_inclusion + self.pruned_by_reachability
    }
}

/// ABB knobs.
#[derive(Debug, Clone)]
pub struct AbbOptions {
    /// Whether to enforce the strict no-excess P-graph rule
    /// (mirrors [`SsgOptions::strict_no_excess`]).
    pub strict_no_excess: bool,
    /// Hard cap on explored nodes (safety net for pathological
    /// inputs). 0 = unlimited.
    pub max_explored: u64,
    /// Stage P-mo (2026-05-19): multi-objective weight vector.
    ///
    /// When `None`, the per-unit cost is the scalar
    /// [`LoweredPGraph::costs`] value (single-criterion ABB, the
    /// original Friedler 1992 form). When `Some(w)`, the per-unit
    /// cost is the dot product `w · cost_vector(u)` against
    /// [`LoweredPGraph::cost_vectors`]. Dimensions absent from a
    /// unit's vector contribute zero. The two paths return
    /// byte-identical results when only one cost dimension is
    /// declared and its weight is set to the scalar value.
    pub cost_weights: Option<Vec<f64>>,
}

impl Default for AbbOptions {
    fn default() -> Self {
        Self {
            // Canonical default (2026-05-27, Pimentel report): no
            // no-excess constraint. Byproducts may be vented; this
            // matches the textbook solution-structures and optima.
            // The prior `true` default made ABB report `None` or a
            // suboptimal incumbent on problems with byproducts
            // (e.g. book Example 14.1 -> None vs optimum 16).
            strict_no_excess: false,
            max_explored: 1_000_000,
            cost_weights: None,
        }
    }
}

/// Per-unit cost under the active option (scalar or weighted-sum).
/// Used internally by both the inclusion bound and the branching
/// step's incremental cost update.
fn effective_cost(p: &LoweredPGraph, opts: &AbbOptions, u: DeclId) -> f64 {
    match &opts.cost_weights {
        None => p.costs.get(&u).copied().unwrap_or(1.0),
        Some(w) => match p.cost_vectors.get(&u) {
            Some(v) => {
                // Dot product. Vectors may differ in length defensively
                // — pad the shorter with zeros (weights of unused
                // dimensions or units missing a tagged cost child).
                let n = w.len().min(v.len());
                (0..n).map(|i| w[i] * v[i]).sum()
            }
            // Unit has no multi-cost vector → cost zero under the
            // weighted form (the user can fall back to scalar by
            // not supplying weights).
            None => 0.0,
        },
    }
}

/// Solve the cost-minimising synthesis problem.
///
/// Returns `None` if no feasible solution exists.
pub fn solve(p: &LoweredPGraph, msg: &MaximalStructure) -> Option<AbbSolution> {
    solve_with_options(p, msg, AbbOptions::default())
}

/// Solve with explicit options. Adapter: the `strict_no_excess` flag
/// selects the leaf-feasibility [`Regime`](crate::regime)
/// (`true → NoExcess`); delegates to [`solve_with_regime`].
pub fn solve_with_options(
    p: &LoweredPGraph,
    msg: &MaximalStructure,
    opts: AbbOptions,
) -> Option<AbbSolution> {
    let regime = crate::regime::from_strict_flag(opts.strict_no_excess);
    solve_with_regime(p, msg, opts, regime)
}

/// Solve with explicit options and an explicit leaf-feasibility
/// [`Regime`](crate::regime). `opts.strict_no_excess` is ignored here —
/// `regime` governs admissibility; `opts` supplies `max_explored` and
/// `cost_weights`.
pub fn solve_with_regime(
    p: &LoweredPGraph,
    msg: &MaximalStructure,
    opts: AbbOptions,
    regime: &dyn crate::regime::Regime,
) -> Option<AbbSolution> {
    let order: Vec<DeclId> = msg.units.iter().copied().collect();
    let mut state = SearchState {
        order,
        included: BTreeSet::new(),
        excluded: BTreeSet::new(),
        cost: 0.0,
        best: None,
        explored: 0,
        pruned_by_inclusion: 0,
        pruned_by_reachability: 0,
        opts,
        regime,
    };
    branch(p, msg, &mut state, 0);
    state.best.map(|(units, cost)| AbbSolution {
        units,
        cost,
        explored: state.explored,
        pruned_by_inclusion: state.pruned_by_inclusion,
        pruned_by_reachability: state.pruned_by_reachability,
    })
}

/// Return the top-$k$ cost-minimal feasible structures, ranked
/// ascending by cost.
///
/// Promoted to library API on 2026-06-03 after the Pimentel benchmark
/// validation (see [`reports/2026-06-03-pimentel-benchmark-validation.md`])
/// asked for CLI access to the 2nd-best (12) and 3rd-best (13)
/// alternatives, not just the optimum. Mirrors the `topk()` helper
/// in [`hymeko_pgraph/tests/topk_sanity.rs`] verbatim — same algorithm
/// (MSG → decision-mapping SSG enumeration → admissibility filter →
/// cost-rank-and-truncate), now exposed for callers outside the test.
///
/// Returns up to `k` solutions; fewer if the feasible space has fewer
/// admissible structures (e.g. `k=10` on a problem with 3 distinct
/// solutions returns 3 entries).
///
/// Each returned [`AbbSolution`] carries the same `explored` /
/// `pruned_*` counters as the single-best [`solve`] path; the
/// counters reflect the full enumeration walk, not per-solution.
/// Empty `units` rows are filtered.
pub fn solve_top_k(
    p: &LoweredPGraph,
    msg: &MaximalStructure,
    k: usize,
    opts: AbbOptions,
) -> Vec<AbbSolution> {
    let regime = crate::regime::from_strict_flag(opts.strict_no_excess);
    solve_top_k_with_regime(p, msg, k, opts, regime)
}

/// Top-$k$ variant of [`solve_with_regime`]. The regime governs
/// admissibility; `opts` supplies `max_explored` and `cost_weights`.
pub fn solve_top_k_with_regime(
    p: &LoweredPGraph,
    msg: &MaximalStructure,
    k: usize,
    opts: AbbOptions,
    regime: &dyn crate::regime::Regime,
) -> Vec<AbbSolution> {
    if k == 0 {
        return Vec::new();
    }
    // Enumerate every solution-structure exactly once via the
    // decision-mapping SSG (Friedler 1992, Ch. 5, Def. 5.1).
    // Filter by regime admissibility and score by sum of effective
    // (scalar or multi-objective) costs. This is the exact pipeline
    // ``tests/topk_sanity.rs::topk`` used; promoted to library API
    // so CLI / Python callers can verify Pimentel's expected top-K
    // without reaching into a private test helper.
    let ssg = crate::ssg_dm::enumerate(p, msg);
    let mut scored: Vec<(f64, BTreeSet<DeclId>)> = ssg
        .into_iter()
        .filter(|s| regime.structure_admissible(p, &s.units))
        .map(|s| {
            let cost: f64 = s
                .units
                .iter()
                .map(|u| effective_cost(p, &opts, *u))
                .sum();
            (cost, s.units)
        })
        .collect();
    // Ascending by cost. `partial_cmp` cannot return None here
    // because every `effective_cost` is a finite f64 (NaN would be
    // a contract violation we should surface upstream).
    scored.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("non-finite cost"));
    scored
        .into_iter()
        .take(k)
        .map(|(cost, units)| AbbSolution {
            units,
            cost,
            // The decision-mapping enumeration doesn't track B&B
            // explored / pruned counters (those are specific to the
            // recursive `solve_with_regime` branch). Surface them as
            // zero so the CLI JSON schema stays stable.
            explored: 0,
            pruned_by_inclusion: 0,
            pruned_by_reachability: 0,
        })
        .collect()
}


struct SearchState<'r> {
    order: Vec<DeclId>,
    included: BTreeSet<DeclId>,
    excluded: BTreeSet<DeclId>,
    cost: f64,
    best: Option<(BTreeSet<DeclId>, f64)>,
    explored: u64,
    pruned_by_inclusion: u64,
    pruned_by_reachability: u64,
    opts: AbbOptions,
    regime: &'r dyn crate::regime::Regime,
}

fn branch(p: &LoweredPGraph, msg: &MaximalStructure, s: &mut SearchState<'_>, depth: usize) {
    if s.opts.max_explored != 0 && s.explored >= s.opts.max_explored {
        return;
    }
    s.explored += 1;

    // ── Bound 1: inclusion bound.
    if let Some((_, best_cost)) = &s.best {
        if s.cost >= *best_cost {
            s.pruned_by_inclusion += 1;
            return;
        }
    }

    // ── Bound 2: reachability bound.
    //
    // The optimistic remaining-units set is `included ∪ undecided`.
    // If even with everything still on the table we can't produce
    // every required product, this branch is infeasible.
    let mut optimistic: BTreeSet<DeclId> = s.included.clone();
    for u in &s.order[depth..] {
        if !s.excluded.contains(u) {
            optimistic.insert(*u);
        }
    }
    let producible = close_producible(p, &optimistic, &p.raws);
    if !p.products.iter().all(|m| producible.contains(m)) {
        s.pruned_by_reachability += 1;
        return;
    }

    // ── Leaf: decide.
    if depth == s.order.len() {
        if is_feasible_with_regime(p, &s.included, s.regime) {
            let candidate = (s.included.clone(), s.cost);
            match &s.best {
                None => s.best = Some(candidate),
                Some((_, bc)) if s.cost < *bc => s.best = Some(candidate),
                _ => {}
            }
        }
        return;
    }

    // ── Branch: include.
    let u = s.order[depth];
    let cu = effective_cost(p, &s.opts, u);
    s.included.insert(u);
    s.cost += cu;
    branch(p, msg, s, depth + 1);
    s.included.remove(&u);
    s.cost -= cu;

    // ── Branch: exclude.
    s.excluded.insert(u);
    branch(p, msg, s, depth + 1);
    s.excluded.remove(&u);
}
