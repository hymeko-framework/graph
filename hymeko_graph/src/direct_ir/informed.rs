//! Informed (Dijkstra / A*) shortest **Berge path** search directly over the canonical `Ir` and its
//! exact sparse [`IncidenceIndex`] — no materialised Levi/star/clique, no second graph model.
//!
//! # What this is
//!
//! [`berge_shortest_path`](super::HyperTraversal::berge_shortest_path) computes the *unit-cost* Berge
//! distance by BFS. This module generalises it to a **weighted, informed** search: a caller-chosen cost
//! (`C: BergeCost`), heuristic (`H: BergeHeuristic`) and transition policy (`P: TransitionPolicy`), all
//! monomorphised — no runtime mode boolean and no `Box<dyn>` in the hot loop.
//!
//! The state space is the **implicit incidence space** of the `Ir` (`node → (arc,slot) → node`); the same
//! `node → incident arcs → arc members → node` alternating step the BFS engine uses. Nothing dropped:
//! every emitted [`BergeStep`] resolves back to a full [`IncidenceRef`] via `(arc, slot)`.
//!
//! # Determinism (contract)
//!
//! The priority key is `(f = g + h  [f64::total_cmp], discrete_tiebreak, DeclId, HyperArcId, slot)` — all
//! canonical. `f64::total_cmp` is used everywhere (never `partial_cmp`, which hides `NaN`). Equal-cost
//! paths are resolved identically across runs and hosts.
//!
//! # Cost contract
//!
//! Costs are finite and non-negative; `NaN`, `±∞` or a negative value is a typed
//! [`InformedSearchError::BadCost`] (never swallowed). The **primary** cost model is *exit-independent
//! separable* (`c = w_arc + w_entry`, depending only on the target incidence): under it an arc is
//! activated **once** at its first-settled member (`EXIT_INDEPENDENT = true`), keeping the walk
//! `O(nodes + arcs + incidences)`. A general (exit-dependent, e.g. metric) cost sets the const `false` and
//! the engine relaxes each member (`O(arity)` per incident arc), still correct.
//!
//! # Optimality
//!
//! With a **consistent** heuristic the first settle of a node is optimal (no reopen). The optimal-mode
//! entry point [`astar_berge_path`] therefore *requires* `H::CONSISTENT` and returns
//! [`InformedSearchError::InconsistentHeuristicInOptimalMode`] otherwise — fail-closed, never a silent
//! sub-optimal path. A zero heuristic reduces the engine to Dijkstra; unit cost + zero heuristic reduces to
//! the BFS Berge distance.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use hymeko::common::ids::{DeclId, HyperArcId, NodeId};
use hymeko::ir::incidence::{IncidenceIndex, IncidenceRef};
use hymeko::ir::ir::{Ir, ValueR};

use super::traversal::{BergeStep, HyperTraversal};
use super::{DirectIrError, decl_to_node, is_node, node_to_decl};

// =====================================================================================================
// Errors
// =====================================================================================================

/// An informed search could not complete. Ids-only, bounded, deterministic — no source text embedded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InformedSearchError {
    /// The start or goal id is not a valid node of this `Ir`.
    InvalidStart {
        /// The offending node id.
        node: NodeId,
    },
    /// An arc reference targets a non-node declaration; the record is malformed for traversal.
    TargetNotANode {
        /// The arc whose reference is ill-typed.
        arc: HyperArcId,
        /// The reference slot within the arc.
        slot: u32,
        /// The non-node target declaration.
        target: DeclId,
    },
    /// A cost was `NaN`, `±∞`, or negative — rejected rather than compared with `partial_cmp`.
    BadCost {
        /// The arc of the offending transition.
        arc: HyperArcId,
        /// The target reference slot.
        slot: u32,
        /// The raw offending value's bit pattern (avoids `f64: Eq` issues; `to_bits`).
        value_bits: u64,
    },
    /// An inconsistent (or unproven-consistent) heuristic was used in optimal mode; refused so no run can
    /// silently return a sub-optimal path. Use a consistent heuristic (or a future bounded-suboptimal mode).
    InconsistentHeuristicInOptimalMode,
    /// A predecessor chain was internally inconsistent during reconstruction — a bug tripwire.
    PredecessorInconsistency {
        /// The node whose predecessor was missing.
        node: DeclId,
    },
}

impl From<DirectIrError> for InformedSearchError {
    fn from(e: DirectIrError) -> Self {
        match e {
            DirectIrError::InvalidStart { node } => Self::InvalidStart { node },
            DirectIrError::TargetNotANode { arc, slot, target } => {
                Self::TargetNotANode { arc, slot, target }
            }
            DirectIrError::PredecessorInconsistency { node } => {
                Self::PredecessorInconsistency { node }
            }
            // The informed search never invokes dual/enumeration budgets; map defensively.
            DirectIrError::InvalidStartArc { arc } => Self::TargetNotANode {
                arc,
                slot: 0,
                target: DeclId::NONE,
            },
            DirectIrError::BudgetExceeded { .. } => Self::InconsistentHeuristicInOptimalMode,
        }
    }
}

impl core::fmt::Display for InformedSearchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidStart { node } => write!(f, "invalid start/goal node id {}", node.raw()),
            Self::TargetNotANode { arc, slot, target } => write!(
                f,
                "arc {} slot {slot} references non-node declaration {}",
                arc.raw(),
                target.raw()
            ),
            Self::BadCost {
                arc,
                slot,
                value_bits,
            } => write!(
                f,
                "non-finite or negative cost {} at arc {} slot {slot}",
                f64::from_bits(*value_bits),
                arc.raw()
            ),
            Self::InconsistentHeuristicInOptimalMode => {
                write!(f, "inconsistent heuristic used in optimal (A*) mode")
            }
            Self::PredecessorInconsistency { node } => {
                write!(
                    f,
                    "predecessor chain inconsistent at declaration {}",
                    node.raw()
                )
            }
        }
    }
}

impl std::error::Error for InformedSearchError {}

// =====================================================================================================
// Cost, heuristic, policy contracts
// =====================================================================================================

/// The incremental, non-negative cost of *arriving at a target incidence*.
///
/// `EXIT_INDEPENDENT = true` (the default) asserts the cost depends only on the target incidence
/// `(arc, slot_v)` and the arc — not on the entry node `from` beyond its `g`. Under that assertion the
/// engine activates each arc once (at its first-settled member); a `false` const forces per-member
/// relaxation (correct for exit-dependent costs like metric edge length).
pub trait BergeCost {
    /// See the trait docs. Default: exit-independent (the unit / incidence-weight models).
    const EXIT_INDEPENDENT: bool = true;
    /// Cost of stepping *into* `to` from node `from`. Must be finite and `>= 0`; the engine rejects
    /// `NaN`/`±∞`/negative as [`InformedSearchError::BadCost`].
    fn step_cost(&self, from: DeclId, to: &IncidenceRef<'_>) -> f64;
}

/// An admissible (ideally consistent) lower bound on the remaining cost to the goal.
pub trait BergeHeuristic {
    /// Whether `h(n) <= h*(n)` for all `n` (never over-estimates). Optimal-mode search needs this.
    const ADMISSIBLE: bool;
    /// Whether `h(n) <= c(n->n') + h(n')` for all legal steps. Optimal mode *requires* this (no reopen).
    const CONSISTENT: bool;
    /// The estimate at `node` (the goal is captured by the closure that builds the heuristic). Must be
    /// finite and `>= 0`; a non-finite value is treated as `0` (still admissible).
    fn h(&self, ir: &Ir, node: DeclId) -> f64;
    /// A discrete, deterministic tie-break (smaller first) that never enters `f`. Default `0` (unused).
    /// A C3 (tie-break-only) quantity is surfaced here, never in [`Self::h`].
    #[inline]
    fn tiebreak(&self, _ir: &Ir, _node: DeclId) -> u32 {
        0
    }
}

/// Whether a Berge step `from -> to` (two incidences of the same arc) is legal. A **filter only** — it
/// never changes a cost.
pub trait TransitionPolicy {
    /// `true` iff the step is allowed. `from`/`to` are the resolved incidences of the shared arc.
    fn allows(&self, from: &IncidenceRef<'_>, to: &IncidenceRef<'_>) -> bool;
}

// ---- concrete cost models --------------------------------------------------------------------------

/// Every hyperedge hop costs `1` (Berge distance). Exit-independent.
#[derive(Clone, Copy, Default)]
pub struct UnitCost;
impl BergeCost for UnitCost {
    #[inline]
    fn step_cost(&self, _from: DeclId, _to: &IncidenceRef<'_>) -> f64 {
        1.0
    }
}

/// `base + |weight(target incidence)|`. Exit-independent; weights come from the resolved incidence.
#[derive(Clone, Copy)]
pub struct IncidenceWeightCost {
    /// A non-negative per-arc base added to every hop (use `1.0` to keep hop-count pressure, `0.0` for a
    /// pure weight sum).
    pub base: f64,
}
impl Default for IncidenceWeightCost {
    fn default() -> Self {
        Self { base: 0.0 }
    }
}
impl BergeCost for IncidenceWeightCost {
    #[inline]
    fn step_cost(&self, _from: DeclId, to: &IncidenceRef<'_>) -> f64 {
        self.base + first_num_weight(to.weights).abs()
    }
}

/// A caller position provider keyed by **canonical `DeclId`** (never a second IR state). Positions may be
/// any fixed dimension; a node without a position yields `None`.
pub trait PositionProvider {
    /// The coordinate of `node`, or `None` if unpositioned.
    fn position(&self, node: DeclId) -> Option<&[f64]>;
}

/// Metric (Euclidean) edge-length cost, coupled with [`MetricHeuristic`]. **Exit-dependent** (depends on
/// `from`), so it forces per-member relaxation. A traversed incidence whose endpoints are not both
/// positioned yields `NaN`, surfaced as [`InformedSearchError::BadCost`] (fail-closed — the metric model
/// requires a total position provider on the reachable component).
#[derive(Clone, Copy)]
pub struct MetricStepCost<'p, P: PositionProvider>(pub &'p P);
impl<P: PositionProvider> BergeCost for MetricStepCost<'_, P> {
    const EXIT_INDEPENDENT: bool = false;
    #[inline]
    fn step_cost(&self, from: DeclId, to: &IncidenceRef<'_>) -> f64 {
        match (self.0.position(from), self.0.position(to.target)) {
            (Some(a), Some(b)) => euclidean(a, b),
            _ => f64::NAN, // rejected by the engine as BadCost
        }
    }
}

// ---- concrete heuristics ---------------------------------------------------------------------------

/// `h ≡ 0`. Admissible and consistent; reduces A* to Dijkstra.
#[derive(Clone, Copy, Default)]
pub struct ZeroHeuristic;
impl BergeHeuristic for ZeroHeuristic {
    const ADMISSIBLE: bool = true;
    const CONSISTENT: bool = true;
    #[inline]
    fn h(&self, _ir: &Ir, _node: DeclId) -> f64 {
        0.0
    }
}

/// `h(n) = ||pos(n) - pos(goal)||`, coupled with [`MetricStepCost`]. Admissible and consistent by the
/// triangle inequality. A node (or goal) without a position contributes `0` (still admissible).
#[derive(Clone, Copy)]
pub struct MetricHeuristic<'p, P: PositionProvider> {
    /// The position provider (canonical `DeclId` keyed).
    pub provider: &'p P,
    /// The goal's coordinate, resolved once at construction (`None` if the goal is unpositioned → `h=0`).
    pub goal_pos: Option<&'p [f64]>,
}
impl<'p, P: PositionProvider> MetricHeuristic<'p, P> {
    /// Build the heuristic for a specific `goal`.
    #[must_use]
    pub fn new(provider: &'p P, goal: DeclId) -> Self {
        let goal_pos = provider.position(goal);
        Self { provider, goal_pos }
    }
}
impl<P: PositionProvider> BergeHeuristic for MetricHeuristic<'_, P> {
    const ADMISSIBLE: bool = true;
    const CONSISTENT: bool = true;
    #[inline]
    fn h(&self, _ir: &Ir, node: DeclId) -> f64 {
        match (self.provider.position(node), self.goal_pos) {
            (Some(p), Some(g)) => euclidean(p, g),
            _ => 0.0,
        }
    }
}

// ---- concrete policies -----------------------------------------------------------------------------

/// Every step is legal (classical Berge). The default.
#[derive(Clone, Copy, Default)]
pub struct Undirected;
impl TransitionPolicy for Undirected {
    #[inline]
    fn allows(&self, _from: &IncidenceRef<'_>, _to: &IncidenceRef<'_>) -> bool {
        true
    }
}

/// Directed: `from` must be an output/producer incidence (sign `-1`) and `to` an input/consumer
/// incidence (sign `+1`) — one Berge step of a producer→consumer orientation. `~` (0) is not a producer.
#[derive(Clone, Copy, Default)]
pub struct DirectedProducerConsumer;
impl TransitionPolicy for DirectedProducerConsumer {
    #[inline]
    fn allows(&self, from: &IncidenceRef<'_>, to: &IncidenceRef<'_>) -> bool {
        from.sign < 0 && to.sign > 0
    }
}

/// Directed: the reverse orientation (consumer→producer), for backward search from a goal.
#[derive(Clone, Copy, Default)]
pub struct DirectedConsumerProducer;
impl TransitionPolicy for DirectedConsumerProducer {
    #[inline]
    fn allows(&self, from: &IncidenceRef<'_>, to: &IncidenceRef<'_>) -> bool {
        from.sign > 0 && to.sign < 0
    }
}

/// Sign-compatible: forbid a step between two strictly opposite-signed incidences unless permitted;
/// `~` (0) is permissive. `same_sign_only = true` additionally requires the two signs to match.
#[derive(Clone, Copy, Default)]
pub struct SignCompatible {
    /// Require the two incidence signs to be equal (ignoring `~`).
    pub same_sign_only: bool,
}
impl TransitionPolicy for SignCompatible {
    #[inline]
    fn allows(&self, from: &IncidenceRef<'_>, to: &IncidenceRef<'_>) -> bool {
        if from.sign == 0 || to.sign == 0 {
            return true;
        }
        if self.same_sign_only {
            from.sign == to.sign
        } else {
            true
        }
    }
}

// =====================================================================================================
// Result + stats
// =====================================================================================================

/// A weighted Berge path: the alternating steps plus the optimal total cost. Each step resolves to a full
/// [`IncidenceRef`] via `(arc, slot)`.
#[derive(Debug, Clone, PartialEq)]
pub struct WeightedBergePath {
    /// The origin node.
    pub start: NodeId,
    /// The alternating steps in order from `start` to the goal.
    pub steps: Vec<BergeStep>,
    /// The optimal total cost (`g[goal]`).
    pub total_cost: f64,
}

impl WeightedBergePath {
    /// Path length in hyperedges.
    #[must_use]
    pub fn hyperedge_len(&self) -> usize {
        self.steps.len()
    }
    /// The node sequence, `start` first.
    pub fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::once(self.start).chain(self.steps.iter().map(|s| s.node))
    }
    /// A canonical, dependency-free FNV-1a-64 digest over `(arc, slot, sign, node)` steps + total-cost
    /// bits — pins the exact result for tests independent of transient timing. (FNV, not BLAKE3, to avoid
    /// adding a `hymeko_graph` dependency, which would be a core-manifest change; a 64-bit deterministic
    /// digest is sufficient to detect any path change.)
    #[must_use]
    pub fn path_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |bytes: &[u8]| {
            for &b in bytes {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        mix(b"HYMEKO-WEIGHTED-BERGE-PATH-v1");
        mix(&(self.start.raw() as u64).to_le_bytes());
        for s in &self.steps {
            mix(&(s.arc.raw() as u64).to_le_bytes());
            mix(&s.slot.to_le_bytes());
            mix(&i64::from(s.sign).to_le_bytes());
            mix(&(s.node.raw() as u64).to_le_bytes());
        }
        mix(&self.total_cost.to_bits().to_le_bytes());
        h
    }
}

/// Per-query instrumentation (never gates results; observability for the benchmark).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchStats {
    /// Nodes popped-and-settled (each once).
    pub expanded_nodes: usize,
    /// Arcs expanded (activated). With `EXIT_INDEPENDENT`, each at most once.
    pub expanded_arcs: usize,
    /// Incidences resolved (member visits) during relaxation.
    pub resolved_incidences: usize,
    /// Heap pushes (relaxations that improved `g`).
    pub pushes: usize,
    /// Heap pops (including stale ones).
    pub pops: usize,
    /// Pops discarded as stale (lazy-deletion overhead).
    pub stale_pops: usize,
    /// Heuristic `h(..)` evaluations during relaxation.
    pub heuristic_evals: usize,
    /// Settled nodes re-encountered with a strictly better `g` (a genuine reopen). Provably `0` with a
    /// consistent heuristic; counted to make that guarantee observable rather than assumed.
    pub reopened: usize,
}

// =====================================================================================================
// Workspace
// =====================================================================================================

/// Priority key: `(f, tiebreak, decl, arc, slot)`, ordered by `f64::total_cmp` then lexicographically.
#[derive(Debug, Clone, Copy)]
struct Key {
    f: f64,
    tie: u32,
    decl: usize,
    arc: usize,
    slot: u32,
}
impl PartialEq for Key {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Key {
    fn cmp(&self, o: &Self) -> Ordering {
        self.f
            .total_cmp(&o.f)
            .then(self.tie.cmp(&o.tie))
            .then(self.decl.cmp(&o.decl))
            .then(self.arc.cmp(&o.arc))
            .then(self.slot.cmp(&o.slot))
    }
}

/// A heap entry, ordered by [`Key`] alone (the `node`/`g` payload does not participate in ordering).
#[derive(Debug, Clone, Copy)]
struct Entry {
    key: Key,
    node: DeclId,
    g_at_push: f64,
}
impl PartialEq for Entry {
    fn eq(&self, o: &Self) -> bool {
        self.key == o.key
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Entry {
    fn cmp(&self, o: &Self) -> Ordering {
        self.key.cmp(&o.key)
    }
}

/// Caller-owned scratch for informed search. Dense, `DeclId`/`HyperArcId`-indexed, generation-stamped;
/// a warm query allocates nothing (the heap `Vec` and the arrays are retained, marks cleared in O(1)
/// amortised). No per-node/per-incidence `HashMap`.
#[derive(Debug, Default)]
pub struct InformedSearchWorkspace {
    stamp: u32,
    g: Vec<f64>,
    g_gen: Vec<u32>,
    settled_gen: Vec<u32>,
    arc_active_gen: Vec<u32>,
    pred_node: Vec<DeclId>,
    pred_arc: Vec<HyperArcId>,
    pred_slot: Vec<u32>,
    heap: BinaryHeap<Reverse<Entry>>,
    stats: SearchStats,
}

impl InformedSearchWorkspace {
    /// A fresh workspace. Grows on first use.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The stats of the most recent query.
    #[must_use]
    pub fn stats(&self) -> SearchStats {
        self.stats
    }

    fn ensure(&mut self, n_decls: usize, n_arcs: usize) {
        if self.g.len() < n_decls {
            self.g.resize(n_decls, f64::INFINITY);
            self.g_gen.resize(n_decls, 0);
            self.settled_gen.resize(n_decls, 0);
            self.pred_node.resize(n_decls, DeclId::NONE);
            self.pred_arc.resize(n_decls, HyperArcId::NONE);
            self.pred_slot.resize(n_decls, 0);
        }
        if self.arc_active_gen.len() < n_arcs {
            self.arc_active_gen.resize(n_arcs, 0);
        }
    }

    fn begin(&mut self) {
        if self.stamp == u32::MAX {
            self.g_gen.iter_mut().for_each(|g| *g = 0);
            self.settled_gen.iter_mut().for_each(|g| *g = 0);
            self.arc_active_gen.iter_mut().for_each(|g| *g = 0);
            self.stamp = 1;
        } else {
            self.stamp += 1;
        }
        self.heap.clear();
        self.stats = SearchStats::default();
    }

    #[inline]
    fn g_valid(&self, d: DeclId) -> bool {
        self.g_gen.get(d.raw()).is_some_and(|&g| g == self.stamp)
    }
    #[inline]
    fn g(&self, d: DeclId) -> f64 {
        if self.g_valid(d) {
            self.g[d.raw()]
        } else {
            f64::INFINITY
        }
    }
    #[inline]
    fn set_g(&mut self, d: DeclId, v: f64) {
        self.g[d.raw()] = v;
        self.g_gen[d.raw()] = self.stamp;
    }
    #[inline]
    fn settled(&self, d: DeclId) -> bool {
        self.settled_gen
            .get(d.raw())
            .is_some_and(|&g| g == self.stamp)
    }
    #[inline]
    fn mark_settled(&mut self, d: DeclId) {
        self.settled_gen[d.raw()] = self.stamp;
    }
    #[inline]
    fn arc_active(&self, a: HyperArcId) -> bool {
        self.arc_active_gen
            .get(a.raw())
            .is_some_and(|&g| g == self.stamp)
    }
    #[inline]
    fn mark_arc_active(&mut self, a: HyperArcId) {
        self.arc_active_gen[a.raw()] = self.stamp;
    }
    #[inline]
    fn set_pred(&mut self, node: DeclId, pred: DeclId, arc: HyperArcId, slot: u32) {
        let i = node.raw();
        self.pred_node[i] = pred;
        self.pred_arc[i] = arc;
        self.pred_slot[i] = slot;
    }
}

// =====================================================================================================
// The engine
// =====================================================================================================

/// Optimal informed shortest **Berge path** from `start` to `goal` under `cost`, `heuristic` and `policy`.
///
/// Returns `Ok(None)` if `goal` is unreachable under `policy`. Requires a **consistent** heuristic
/// (`H::CONSISTENT`); otherwise [`InformedSearchError::InconsistentHeuristicInOptimalMode`] — no run may
/// silently return a sub-optimal path. A [`ZeroHeuristic`] gives Dijkstra; [`UnitCost`] + [`ZeroHeuristic`]
/// gives the BFS Berge distance. Stats for the run are in [`InformedSearchWorkspace::stats`].
///
/// # Preconditions
/// `start`/`goal` are node ids of `traversal.ir()`; the workspace may be fresh or reused.
///
/// # Errors
/// [`InformedSearchError::InvalidStart`] (bad endpoint), [`InformedSearchError::TargetNotANode`]
/// (malformed arc reference), [`InformedSearchError::BadCost`] (non-finite/negative cost),
/// [`InformedSearchError::InconsistentHeuristicInOptimalMode`].
pub fn astar_berge_path<C, H, P>(
    traversal: &HyperTraversal<'_, '_>,
    start: NodeId,
    goal: NodeId,
    cost: &C,
    heuristic: &H,
    policy: &P,
    ws: &mut InformedSearchWorkspace,
) -> Result<Option<WeightedBergePath>, InformedSearchError>
where
    C: BergeCost,
    H: BergeHeuristic,
    P: TransitionPolicy,
{
    if !H::CONSISTENT {
        return Err(InformedSearchError::InconsistentHeuristicInOptimalMode);
    }
    let ir = traversal.ir();
    let index = traversal.index();
    let start_decl = node_to_decl(ir, start)?;
    let goal_decl = node_to_decl(ir, goal)?;
    ws.ensure(ir.decl_nodes.len(), ir.arcs.len());
    ws.begin();

    if start_decl == goal_decl {
        return Ok(Some(WeightedBergePath {
            start,
            steps: Vec::new(),
            total_cost: 0.0,
        }));
    }

    ws.set_g(start_decl, 0.0);
    let h0 = finite_or_zero(heuristic.h(ir, start_decl));
    ws.heap.push(Reverse(Entry {
        key: Key {
            f: h0,
            tie: heuristic.tiebreak(ir, start_decl),
            decl: start_decl.raw(),
            arc: HyperArcId::NONE.raw(),
            slot: 0,
        },
        node: start_decl,
        g_at_push: 0.0,
    }));
    ws.stats.pushes += 1;

    while let Some(Reverse(entry)) = ws.heap.pop() {
        ws.stats.pops += 1;
        let node = entry.node;
        // Lazy deletion: skip a stale entry (already settled, or superseded by a better g).
        if ws.settled(node) || entry.g_at_push > ws.g(node) {
            if ws.settled(node) && entry.g_at_push < ws.g(node) {
                ws.stats.reopened += 1; // never happens with a consistent heuristic; observed, not assumed
            }
            ws.stats.stale_pops += 1;
            continue;
        }
        ws.mark_settled(node);
        ws.stats.expanded_nodes += 1;
        if node == goal_decl {
            let total_cost = ws.g(goal_decl);
            return reconstruct(ws, index, ir, start, start_decl, goal_decl, total_cost).map(Some);
        }
        let gu = ws.g(node);
        let arcs = index.incident_arcs(node);
        for &arc in arcs {
            if C::EXIT_INDEPENDENT {
                if ws.arc_active(arc) {
                    continue;
                }
                ws.mark_arc_active(arc);
            }
            ws.stats.expanded_arcs += 1;
            let from_iref = index.resolve(arc, u_slot_in(ir, index, node, arc));
            let refs = &ir.arcs[arc.raw()].refs;
            for (slot, sref) in refs.iter().enumerate() {
                let target = sref.target();
                if !is_node(ir, target) {
                    return Err(InformedSearchError::TargetNotANode {
                        arc,
                        slot: slot as u32,
                        target,
                    });
                }
                if target == node {
                    continue; // self / entry incidence — no self-transition
                }
                let to_iref = index.resolve(arc, slot as u32);
                ws.stats.resolved_incidences += 1;
                if !policy.allows(&from_iref, &to_iref) {
                    continue;
                }
                let c = cost.step_cost(node, &to_iref);
                if !c.is_finite() || c < 0.0 {
                    return Err(InformedSearchError::BadCost {
                        arc,
                        slot: slot as u32,
                        value_bits: c.to_bits(),
                    });
                }
                let tentative = gu + c;
                if tentative < ws.g(target) {
                    ws.set_g(target, tentative);
                    ws.set_pred(target, node, arc, slot as u32);
                    ws.stats.heuristic_evals += 1;
                    let f = tentative + finite_or_zero(heuristic.h(ir, target));
                    ws.heap.push(Reverse(Entry {
                        key: Key {
                            f,
                            tie: heuristic.tiebreak(ir, target),
                            decl: target.raw(),
                            arc: arc.raw(),
                            slot: slot as u32,
                        },
                        node: target,
                        g_at_push: tentative,
                    }));
                    ws.stats.pushes += 1;
                }
            }
        }
    }
    Ok(None)
}

/// Uniform-cost / Dijkstra shortest Berge path — [`astar_berge_path`] with a [`ZeroHeuristic`]. The
/// reference optimal engine the A* results are checked against.
///
/// # Errors
/// As [`astar_berge_path`] (a zero heuristic never triggers the consistency error).
pub fn dijkstra_berge_path<C, P>(
    traversal: &HyperTraversal<'_, '_>,
    start: NodeId,
    goal: NodeId,
    cost: &C,
    policy: &P,
    ws: &mut InformedSearchWorkspace,
) -> Result<Option<WeightedBergePath>, InformedSearchError>
where
    C: BergeCost,
    P: TransitionPolicy,
{
    astar_berge_path(traversal, start, goal, cost, &ZeroHeuristic, policy, ws)
}

fn reconstruct(
    ws: &InformedSearchWorkspace,
    index: &IncidenceIndex<'_>,
    ir: &Ir,
    start: NodeId,
    start_decl: DeclId,
    goal_decl: DeclId,
    total_cost: f64,
) -> Result<WeightedBergePath, InformedSearchError> {
    let mut steps_rev: Vec<BergeStep> = Vec::new();
    let mut cur = goal_decl;
    while cur != start_decl {
        let i = cur.raw();
        let pred_arc = ws.pred_arc[i];
        let pred_slot = ws.pred_slot[i];
        let pred_node = ws.pred_node[i];
        if pred_arc.is_none() {
            return Err(InformedSearchError::PredecessorInconsistency { node: cur });
        }
        let node_id = decl_to_node(ir, cur)
            .ok_or(InformedSearchError::PredecessorInconsistency { node: cur })?;
        let iref = index.resolve(pred_arc, pred_slot);
        steps_rev.push(BergeStep {
            arc: pred_arc,
            slot: pred_slot,
            sign: iref.sign,
            node: node_id,
        });
        cur = pred_node;
    }
    steps_rev.reverse();
    Ok(WeightedBergePath {
        start,
        steps: steps_rev,
        total_cost,
    })
}

// =====================================================================================================
// small helpers
// =====================================================================================================

/// The slot of `node` within `arc` (its incidence coordinate on that arc). The index stores the parallel
/// `incident_arcs`/`incident_slots`; find the matching arc. This is a small linear scan over `node`'s
/// degree (typically tiny); it is only done once per (settled node, incident arc).
#[inline]
fn u_slot_in(_ir: &Ir, index: &IncidenceIndex<'_>, node: DeclId, arc: HyperArcId) -> u32 {
    let arcs = index.incident_arcs(node);
    let slots = index.incident_slots(node);
    for (i, &a) in arcs.iter().enumerate() {
        if a == arc {
            return slots[i];
        }
    }
    0
}

/// First top-level `Num` weight of an incidence, or `0.0`. Deterministic; nested lists are ignored (the
/// fixtures attach a single scalar).
#[inline]
fn first_num_weight(weights: Option<&[ValueR]>) -> f64 {
    weights
        .and_then(|ws| {
            ws.iter().find_map(|v| match v {
                ValueR::Num(n) => Some(*n),
                _ => None,
            })
        })
        .unwrap_or(0.0)
}

#[inline]
fn finite_or_zero(x: f64) -> f64 {
    if x.is_finite() && x >= 0.0 { x } else { 0.0 }
}

#[inline]
fn euclidean(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    let mut acc = 0.0;
    for i in 0..n {
        let d = a[i] - b[i];
        acc += d * d;
    }
    acc.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hymeko::ir::incidence::build_incidence_index;
    use hymeko::ir::ir::{AnnoR, ArcRec, DeclKind, DeclNode, NodeRec, RefAtomR, SignedRefR};

    /// A tiny 3-node chain: arc0 = {n0,n1}, arc1 = {n1,n2}. Unit-cost distance n0->n2 is 2.
    fn chain3() -> Ir {
        let mut ir = Ir::default();
        for _ in 0..3 {
            let d = ir.push_decl(DeclNode {
                kind: DeclKind::Node,
                name: hymeko::common::ids::SymId::new(0),
                parent: DeclId::NONE,
                first_child: DeclId::NONE,
                last_child: DeclId::NONE,
                next_sibling: DeclId::NONE,
                anno: AnnoR::default(),
            });
            let nid = NodeId::new(ir.nodes.len());
            ir.nodes.push(NodeRec::new(d, Vec::new()));
            ir.bind_node_decl(d, nid).expect("bind");
        }
        for (a, b) in [(0usize, 1usize), (1, 2)] {
            ir.arcs.push(ArcRec {
                anno: AnnoR::default(),
                in_edge: DeclId::NONE,
                refs: vec![
                    SignedRefR::Plus(RefAtomR {
                        target: DeclId::new(a),
                        anno: AnnoR::default(),
                        weights: None,
                    }),
                    SignedRefR::Plus(RefAtomR {
                        target: DeclId::new(b),
                        anno: AnnoR::default(),
                        weights: None,
                    }),
                ],
            });
        }
        ir
    }

    #[test]
    fn unit_cost_distance_and_reconstruct() {
        let ir = chain3();
        let idx = build_incidence_index(&ir).unwrap();
        let trav = HyperTraversal::new(&idx);
        let mut ws = InformedSearchWorkspace::new();
        let p = dijkstra_berge_path(
            &trav,
            NodeId::new(0),
            NodeId::new(2),
            &UnitCost,
            &Undirected,
            &mut ws,
        )
        .unwrap()
        .expect("reachable");
        assert_eq!(p.total_cost, 2.0);
        assert_eq!(p.hyperedge_len(), 2);
        assert_eq!(
            p.nodes().map(|n| n.raw()).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        // every step resolves to an exact incidence (arc,slot)
        for s in &p.steps {
            assert!(s.arc.raw() < ir.arcs.len());
            assert!((s.slot as usize) < ir.arcs[s.arc.raw()].refs.len());
        }
    }

    #[test]
    fn start_equals_goal_is_zero() {
        let ir = chain3();
        let idx = build_incidence_index(&ir).unwrap();
        let trav = HyperTraversal::new(&idx);
        let mut ws = InformedSearchWorkspace::new();
        let p = dijkstra_berge_path(
            &trav,
            NodeId::new(1),
            NodeId::new(1),
            &UnitCost,
            &Undirected,
            &mut ws,
        )
        .unwrap()
        .unwrap();
        assert_eq!(p.total_cost, 0.0);
        assert_eq!(p.steps.len(), 0);
    }

    #[test]
    fn unreachable_is_none() {
        // two disjoint arcs: {0,1} and {2,3}
        let mut ir = Ir::default();
        for _ in 0..4 {
            let d = ir.push_decl(DeclNode {
                kind: DeclKind::Node,
                name: hymeko::common::ids::SymId::new(0),
                parent: DeclId::NONE,
                first_child: DeclId::NONE,
                last_child: DeclId::NONE,
                next_sibling: DeclId::NONE,
                anno: AnnoR::default(),
            });
            let nid = NodeId::new(ir.nodes.len());
            ir.nodes.push(NodeRec::new(d, Vec::new()));
            ir.bind_node_decl(d, nid).unwrap();
        }
        for (a, b) in [(0usize, 1usize), (2, 3)] {
            ir.arcs.push(ArcRec {
                anno: AnnoR::default(),
                in_edge: DeclId::NONE,
                refs: vec![
                    SignedRefR::Plus(RefAtomR {
                        target: DeclId::new(a),
                        anno: AnnoR::default(),
                        weights: None,
                    }),
                    SignedRefR::Plus(RefAtomR {
                        target: DeclId::new(b),
                        anno: AnnoR::default(),
                        weights: None,
                    }),
                ],
            });
        }
        let idx = build_incidence_index(&ir).unwrap();
        let trav = HyperTraversal::new(&idx);
        let mut ws = InformedSearchWorkspace::new();
        let r = dijkstra_berge_path(
            &trav,
            NodeId::new(0),
            NodeId::new(3),
            &UnitCost,
            &Undirected,
            &mut ws,
        )
        .unwrap();
        assert!(r.is_none());
    }
}
