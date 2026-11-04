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
//!
//! # Operator structure
//!
//! [`Abb`] is the operator: the leaf regime, the node budget and the
//! [`ChildOrder`].
//! [`Abb::solve`] is the classic additive-cost ABB. [`BoundedAbb`]
//! composes the operator with a [`StructureObjective`] and owns the
//! single traversal, so the classic ABB is the bounded one under the
//! additive cost. The reachability bound is decided either by the
//! lowered schema's producibility closure or, hypergraph-natively, by
//! firing the operating units as hyperedges over the canonical
//! incidence ([`BoundedAbb::solve_on`], the Boolean-exact
//! [`crate::andor`] fixpoint, equal to the closure).

use std::collections::BTreeSet;

use hymeko::common::ids::DeclId;

use crate::andor::{AndOrWorkspace, BooleanExact, NormError, PropagationConfig, Reliability, propagate};
use crate::lowering::LoweredPGraph;
use crate::msg::{MaximalStructure, close_producible};
use crate::regime::Regime;
use crate::ssg::is_feasible_with_regime;
use crate::view::PGraphView;

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
    /// False when `max_explored` stopped the search; the incumbent is then
    /// not proven optimal.
    pub complete: bool,
}

impl From<ObjectiveSolution> for AbbSolution {
    fn from(s: ObjectiveSolution) -> Self {
        Self {
            units: s.units,
            cost: s.value,
            explored: s.explored,
            pruned_by_inclusion: s.pruned_by_bound,
            pruned_by_reachability: s.pruned_by_reachability,
            complete: s.complete,
        }
    }
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

/// Per-unit cost under the active weights (scalar or weighted-sum).
/// Used by the additive objective and the top-k ranking.
fn effective_cost(p: &LoweredPGraph, cost_weights: Option<&[f64]>, u: DeclId) -> f64 {
    match cost_weights {
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
    Abb::new(regime, opts.max_explored).solve(p, msg, opts.cost_weights.as_deref())
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
            let cost: f64 = s.units.iter().map(|u| effective_cost(p, opts.cost_weights.as_deref(), *u)).sum();
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
            // The enumeration is exhaustive.
            complete: true,
        })
        .collect()
}

/// Caller-supplied objective for [`solve_with_objective`], for objectives that
/// are not a sum of per-unit costs (for example a schedule-dependent peak).
///
/// Contract: for every feasible structure `S` with
/// `included ⊆ S ⊆ included ∪ open` whose `evaluate(S)` is `Some(v)`,
/// `lower_bound(included, open) <= v`. With an admissible bound the returned
/// structure is optimal whenever the search is `complete`.
pub trait StructureObjective {
    /// Lower bound over every completion of a partial include/exclude selection.
    fn lower_bound(&self, included: &BTreeSet<DeclId>, open: &BTreeSet<DeclId>) -> f64;
    /// Exact value of a feasible structure; `None` rejects it.
    fn evaluate(&self, units: &BTreeSet<DeclId>) -> Option<f64>;
}

/// Output of [`solve_with_objective`].
#[derive(Debug, Clone)]
pub struct ObjectiveSolution {
    /// Operating units of the best structure found.
    pub units: BTreeSet<DeclId>,
    /// Its objective value.
    pub value: f64,
    /// Number of nodes explored (diagnostic).
    pub explored: u64,
    /// Nodes pruned because the objective's lower bound reached the incumbent.
    pub pruned_by_bound: u64,
    /// Nodes pruned by the reachability bound.
    pub pruned_by_reachability: u64,
    /// False when `max_explored` stopped the search; the incumbent is then
    /// not proven optimal.
    pub complete: bool,
}

/// Branch-and-bound over the maximal structure with a caller-supplied
/// objective. Reachability pruning and leaf feasibility are those of
/// [`solve_with_regime`]; the inclusion bound is `objective.lower_bound`.
/// `opts.max_explored` bounds the walk; `opts.cost_weights` is unused.
pub fn solve_with_objective(
    p: &LoweredPGraph,
    msg: &MaximalStructure,
    opts: &AbbOptions,
    regime: &dyn crate::regime::Regime,
    objective: &dyn StructureObjective,
) -> Option<ObjectiveSolution> {
    BoundedAbb::new(Abb::new(regime, opts.max_explored), objective).solve(p, msg)
}

/// Which child of a branch point the traversal visits first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChildOrder {
    /// Include first: the classic Friedler order, which reaches a feasible
    /// incumbent quickly.
    #[default]
    IncludeFirst,
    /// The child with the lower objective bound first, include on ties. With
    /// an exact bound the first descent reaches an optimum, which then cuts
    /// every later sibling.
    LowerBoundFirst,
}

impl ChildOrder {
    fn include_first(self, include: f64, exclude: f64) -> bool {
        match self {
            Self::IncludeFirst => true,
            Self::LowerBoundFirst => include <= exclude,
        }
    }
}

/// The ABB operator: Friedler include/exclude branch-and-bound over a
/// maximal structure, with the reachability bound, the regime's leaf
/// feasibility, a node budget and a child order.
#[derive(Clone, Copy)]
pub struct Abb<'r> {
    regime: &'r dyn Regime,
    max_explored: u64,
    order: ChildOrder,
}

impl<'r> Abb<'r> {
    /// An operator admitting leaves by `regime`; `max_explored` caps the
    /// explored nodes (0 = unlimited). Children are visited include first.
    pub fn new(regime: &'r dyn Regime, max_explored: u64) -> Self {
        Self { regime, max_explored, order: ChildOrder::default() }
    }

    /// The operator visiting the children of each branch point in `order`.
    pub fn with_order(self, order: ChildOrder) -> Self {
        Self { order, ..self }
    }

    /// The classic ABB: minimum [`AdditiveCost`].
    pub fn solve(
        &self,
        p: &LoweredPGraph,
        msg: &MaximalStructure,
        cost_weights: Option<&[f64]>,
    ) -> Option<AbbSolution> {
        let cost = AdditiveCost::new(p, cost_weights);
        BoundedAbb::new(*self, &cost).solve(p, msg).map(AbbSolution::from)
    }
}

/// [`Abb`] bounded by a caller-supplied [`StructureObjective`]: its lower
/// bound prunes, its value ranks the feasible leaves.
#[derive(Clone, Copy)]
pub struct BoundedAbb<'a> {
    abb: Abb<'a>,
    objective: &'a dyn StructureObjective,
}

impl<'a> BoundedAbb<'a> {
    /// The operator `abb` bounded by `objective`.
    pub fn new(abb: Abb<'a>, objective: &'a dyn StructureObjective) -> Self {
        Self { abb, objective }
    }

    /// Search with the reachability bound decided by the lowered schema's
    /// producibility closure.
    pub fn solve(&self, p: &LoweredPGraph, msg: &MaximalStructure) -> Option<ObjectiveSolution> {
        self.search(p, msg, &mut SchemaClosure(p))
    }

    /// Hypergraph-native search: the reachability bound fires the selected
    /// operating units as hyperedges over the canonical incidence. `view`
    /// classifies `p`, e.g. [`PGraphView::from_lowered`].
    ///
    /// # Errors
    /// The [`NormError`] of the first failed propagation; the view's
    /// classification is then inconsistent with its `Ir`.
    pub fn solve_on(
        &self,
        view: &PGraphView<'_, '_, '_>,
        p: &LoweredPGraph,
        msg: &MaximalStructure,
    ) -> Result<Option<ObjectiveSolution>, NormError> {
        let mut firing = UnitFiring::new(*view);
        let found = self.search(p, msg, &mut firing);
        firing.error.map_or(Ok(found), Err)
    }

    fn search(
        &self,
        p: &LoweredPGraph,
        msg: &MaximalStructure,
        reach: &mut dyn Producibility,
    ) -> Option<ObjectiveSolution> {
        let mut search = Search {
            p,
            order: msg.units.iter().copied().collect(),
            bounded: *self,
            reach,
            included: BTreeSet::new(),
            best: None,
            explored: 0,
            pruned_by_bound: 0,
            pruned_by_reachability: 0,
            stopped: false,
        };
        let bound = self.objective.lower_bound(&search.included, &msg.units);
        search.branch(0, &msg.units, bound);
        let Search { best, explored, pruned_by_bound, pruned_by_reachability, stopped, .. } = search;
        best.map(|(units, value)| ObjectiveSolution {
            units,
            value,
            explored,
            pruned_by_bound,
            pruned_by_reachability,
            complete: !stopped,
        })
    }
}

/// The classic additive cost: the scalar unit costs, or `cost_weights`
/// dotted with the unit cost vectors. The included units' cost bounds every
/// completion (unit costs are non-negative).
#[derive(Clone, Copy)]
pub struct AdditiveCost<'p> {
    p: &'p LoweredPGraph,
    cost_weights: Option<&'p [f64]>,
}

impl<'p> AdditiveCost<'p> {
    /// The additive cost of `p` under optional `cost_weights`.
    pub fn new(p: &'p LoweredPGraph, cost_weights: Option<&'p [f64]>) -> Self {
        Self { p, cost_weights }
    }
}

impl StructureObjective for AdditiveCost<'_> {
    fn lower_bound(&self, included: &BTreeSet<DeclId>, _open: &BTreeSet<DeclId>) -> f64 {
        included.iter().map(|&u| effective_cost(self.p, self.cost_weights, u)).sum()
    }
    /// The bound with nothing left open is exact.
    fn evaluate(&self, units: &BTreeSet<DeclId>) -> Option<f64> {
        Some(self.lower_bound(units, &BTreeSet::new()))
    }
}

/// Whether a unit set can still produce every required product.
trait Producibility {
    fn reaches_products(&mut self, units: &BTreeSet<DeclId>) -> bool;
}

/// The lowered schema's producibility closure.
struct SchemaClosure<'p>(&'p LoweredPGraph);

impl Producibility for SchemaClosure<'_> {
    fn reaches_products(&mut self, units: &BTreeSet<DeclId>) -> bool {
        let producible = close_producible(self.0, units, &self.0.raws);
        self.0.products.iter().all(|m| producible.contains(m))
    }
}

/// The units fire as hyperedges over the incidence: the Boolean AND/OR
/// fixpoint with every unit outside the set at reliability 0.
struct UnitFiring<'a, 'ir, 'idx> {
    view: PGraphView<'a, 'ir, 'idx>,
    ws: AndOrWorkspace,
    cfg: PropagationConfig,
    error: Option<NormError>,
}

impl<'a, 'ir, 'idx> UnitFiring<'a, 'ir, 'idx> {
    fn new(view: PGraphView<'a, 'ir, 'idx>) -> Self {
        let cfg = PropagationConfig::boolean_exact(view.units().len(), view.materials().len());
        Self { view, ws: AndOrWorkspace::new(), cfg, error: None }
    }
}

/// Reliability 1 exactly for the units of a selection.
struct Selected<'s>(&'s BTreeSet<DeclId>);

impl Reliability for Selected<'_> {
    fn of(&self, u: DeclId) -> f64 {
        if self.0.contains(&u) { 1.0 } else { 0.0 }
    }
}

impl Producibility for UnitFiring<'_, '_, '_> {
    fn reaches_products(&mut self, units: &BTreeSet<DeclId>) -> bool {
        if self.error.is_some() {
            return false;
        }
        match propagate(&self.view, &BooleanExact, &Selected(units), &mut self.ws, self.cfg) {
            Ok(avail) => self.view.products().iter().all(|m| avail.producible().contains(m)),
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }
}

/// The state of one [`BoundedAbb`] traversal.
struct Search<'s> {
    p: &'s LoweredPGraph,
    order: Vec<DeclId>,
    bounded: BoundedAbb<'s>,
    reach: &'s mut dyn Producibility,
    included: BTreeSet<DeclId>,
    best: Option<(BTreeSet<DeclId>, f64)>,
    explored: u64,
    pruned_by_bound: u64,
    pruned_by_reachability: u64,
    stopped: bool,
}

impl Search<'_> {
    /// The node deciding `order[depth]`: `open` is the undecided suffix and
    /// `bound` its lower bound, computed by the parent to order the children.
    fn branch(&mut self, depth: usize, open: &BTreeSet<DeclId>, bound: f64) {
        let budget = self.bounded.abb.max_explored;
        if budget != 0 && self.explored >= budget {
            self.stopped = true;
            return;
        }
        self.explored += 1;
        // An infinite bound admits no evaluable completion, incumbent or not.
        if bound == f64::INFINITY || self.best.as_ref().is_some_and(|(_, best)| bound >= *best) {
            self.pruned_by_bound += 1;
            return;
        }
        let optimistic: BTreeSet<DeclId> = self.included.union(open).copied().collect();
        if !self.reach.reaches_products(&optimistic) {
            self.pruned_by_reachability += 1;
            return;
        }
        if depth == self.order.len() {
            self.leaf();
            return;
        }
        let u = self.order[depth];
        let mut rest = open.clone();
        rest.remove(&u);
        let exclude = self.bounded.objective.lower_bound(&self.included, &rest);
        self.included.insert(u);
        let include = self.bounded.objective.lower_bound(&self.included, &rest);
        if self.bounded.abb.order.include_first(include, exclude) {
            self.branch(depth + 1, &rest, include);
            self.included.remove(&u);
            self.branch(depth + 1, &rest, exclude);
        } else {
            self.included.remove(&u);
            self.branch(depth + 1, &rest, exclude);
            self.included.insert(u);
            self.branch(depth + 1, &rest, include);
            self.included.remove(&u);
        }
    }

    fn leaf(&mut self) {
        if !is_feasible_with_regime(self.p, &self.included, self.bounded.abb.regime) {
            return;
        }
        let Some(value) = self.bounded.objective.evaluate(&self.included) else {
            return;
        };
        if self.best.as_ref().is_none_or(|(_, best)| value < *best) {
            self.best = Some((self.included.clone(), value));
        }
    }
}
