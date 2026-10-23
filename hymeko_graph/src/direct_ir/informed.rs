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
//! separable* (`c = w_arc + w_entry`, depending only on the target incidence). Combined with an
//! entry-independent transition policy, arc work is reused when an entry has no better g. Dijkstra
//! therefore expands such an arc once; A* may improve its entry cost and must expand it again.
//! General costs or entry-dependent policies relax every exact entry slot (`O(arity)` per incidence).
//!
//! # Optimality
//!
//! With a **consistent** heuristic the first settle of a node is optimal (no reopen). The optimal-mode
//! entry point [`astar_berge_path`] therefore *requires* `H::CONSISTENT` and returns
//! [`InformedSearchError::InconsistentHeuristicInOptimalMode`] otherwise — fail-closed, never a silent
//! sub-optimal path. A zero heuristic reduces the engine to Dijkstra; unit cost + zero heuristic reduces to
//! the BFS Berge distance.

mod prepared;
pub use prepared::PreparedBergeSearch;

#[cfg(test)]
use hymeko::ir::ir::IrConstruction;
use std::cmp::Ordering;

use hymeko::common::ids::{DeclId, HyperArcId, NodeId};
use hymeko::ir::incidence::{IncidenceIndex, IncidenceRef};
use hymeko::ir::ir::{Ir, ValueR};

use super::traversal::{BergeIncidence, BergeStep, HyperTraversal};
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
    /// A declared global step-cost lower bound was negative or non-finite.
    BadStepBound {
        /// Rejected bound's exact bits.
        value_bits: u64,
    },
    /// Prepared hop/prioritization tables require fewer than u32::MAX nodes.
    PreparationTooLarge {
        /// Requested node count.
        nodes: usize,
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
            Self::BadStepBound { value_bits } => write!(
                f,
                "invalid step lower bound {}",
                f64::from_bits(*value_bits)
            ),
            Self::PreparationTooLarge { nodes } => {
                write!(f, "prepared Berge domain exceeds u32: {nodes} nodes")
            }
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
/// `(arc, slot_v)` and the arc — not on the entry node `from` beyond its `g`. With an entry-independent
/// policy, this permits reuse at an equal or worse entry g, not unconditional first-member closure.
/// A `false` const forces per-member relaxation (for example, metric edge length).
pub trait BergeCost {
    /// See the trait docs. Default: exit-independent (the unit / incidence-weight models).
    const EXIT_INDEPENDENT: bool = true;
    /// Certified lower bound for every legal step. Zero is always valid for this
    /// nonnegative-cost contract. An override must prove the global inequality;
    /// estimates such as entropy must never be returned here.
    fn minimum_step_cost(&self) -> f64 {
        0.0
    }
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
    /// Whether legality is independent of the entry incidence within an arc.
    /// The conservative default preserves arbitrary caller policies. If true,
    /// `allows(from, to)` must agree for every entry incidence and fixed `to`.
    fn entry_independent(&self) -> bool {
        false
    }

    /// `true` iff the step is allowed. `from`/`to` are the resolved incidences of the shared arc.
    fn allows(&self, from: &IncidenceRef<'_>, to: &IncidenceRef<'_>) -> bool;
}

// ---- concrete cost models --------------------------------------------------------------------------

/// Every hyperedge hop costs `1` (Berge distance). Exit-independent.
#[derive(Clone, Copy, Default)]
pub struct UnitCost;
impl BergeCost for UnitCost {
    fn minimum_step_cost(&self) -> f64 {
        1.0
    }
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
    fn minimum_step_cost(&self) -> f64 {
        // Invalid/negative bases do not certify a positive global bound. The
        // actual transition cost is still checked when a legal step is used.
        if self.base.is_finite() && self.base > 0.0 {
            self.base
        } else {
            0.0
        }
    }
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

/// A borrowed, declaration-indexed undirected hop-distance lower bound.
///
/// # Contract
/// `hops` must contain exact undirected hop distances to the search goal on the
/// same IR, covering every node's DeclId. `u32::MAX` denotes a disconnected node.
/// Every legal transition must cost at least `min_step_cost`. Policy restrictions
/// may remove transitions. These conditions make the estimate admissible and
/// consistent; this adapter does not compute or certify the distance table.
/// Missing table entries and disconnected components contribute zero.
#[derive(Clone, Copy, Debug)]
pub struct HopHeuristic<'a> {
    hops: &'a [u32],
    min_step_cost: f64,
}

impl<'a> HopHeuristic<'a> {
    /// Borrow prepared distances without copying or allocating.
    /// # Panics
    /// Panics for a negative/nonfinite step bound or an overflowing finite-hop
    /// estimate. Numeric validation scans the table once, outside search.
    /// The topology/cost coupling in the type's contract remains the caller's duty.
    pub fn new(hops: &'a [u32], min_step_cost: f64) -> Self {
        assert!(
            min_step_cost.is_finite() && min_step_cost >= 0.0,
            "hop step bound must be finite and non-negative"
        );
        assert!(
            hops.iter()
                .all(|&h| h == u32::MAX || (f64::from(h) * min_step_cost).is_finite()),
            "hop estimate overflows"
        );
        Self {
            hops,
            min_step_cost,
        }
    }
}

impl BergeHeuristic for HopHeuristic<'_> {
    const ADMISSIBLE: bool = true;
    const CONSISTENT: bool = true;
    fn h(&self, _ir: &Ir, node: DeclId) -> f64 {
        match self.hops.get(node.raw()).copied() {
            None | Some(u32::MAX) => 0.0,
            Some(hops) => f64::from(hops) * self.min_step_cost,
        }
    }
}

/// Pointwise maximum of two borrowed bounds for the same graph, goal and costs.
/// Both component contracts must hold. Guarantees are their conjunction;
/// tie-break values are intentionally not combined (the default remains zero).
#[derive(Clone, Copy)]
pub struct MaxHeuristic<'a, A, B> {
    left: &'a A,
    right: &'a B,
}

impl<'a, A: BergeHeuristic, B: BergeHeuristic> MaxHeuristic<'a, A, B> {
    /// Compose already prepared bounds, without allocating or taking ownership.
    pub fn new(left: &'a A, right: &'a B) -> Self {
        Self { left, right }
    }
}

impl<A: BergeHeuristic, B: BergeHeuristic> BergeHeuristic for MaxHeuristic<'_, A, B> {
    const ADMISSIBLE: bool = A::ADMISSIBLE && B::ADMISSIBLE;
    const CONSISTENT: bool = A::CONSISTENT && B::CONSISTENT;
    fn h(&self, ir: &Ir, node: DeclId) -> f64 {
        self.left.h(ir, node).max(self.right.h(ir, node))
    }
}

// ---- concrete policies -----------------------------------------------------------------------------

/// Every step is legal (classical Berge). The default.
#[derive(Clone, Copy, Default)]
pub struct Undirected;
impl TransitionPolicy for Undirected {
    fn entry_independent(&self) -> bool {
        true
    }
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

/// The historical, **topological** path digest: arcs and arrival coordinates only.
///
/// A newtype, not a bare `u64`, so it cannot be compared with or stored as an
/// [`OrientedMaterialisationDigest`]. Two digests that answer different questions must not be
/// interchangeable by raw integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathHash(u64);

impl PathHash {
    /// The raw value, for a record or a wire format.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// The digest of a path's **oriented materialisation**: every step's arc, entry and exit.
///
/// Distinct from [`PathHash`] in type as well as in domain tag, for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrientedMaterialisationDigest(u64);

impl OrientedMaterialisationDigest {
    /// The raw value, for a record or a wire format.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

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

/// Owns byte accumulation and framing, while each digest retains its distinct
/// typed identity, domain and payload field order.
struct PathDigest(u64);

impl PathDigest {
    fn new(domain: &[u8], start: NodeId) -> Self {
        let mut digest = Self(0xcbf2_9ce4_8422_2325);
        digest.write(domain);
        digest.write(&(start.raw() as u64).to_le_bytes());
        digest
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(mut self, cost: f64) -> u64 {
        self.write(&cost.to_bits().to_le_bytes());
        self.0
    }
}

impl WeightedBergePath {
    /// Path length in hyperedges.
    #[must_use]
    pub fn hyperedge_len(&self) -> usize {
        self.steps.len()
    }
    /// The node sequence, `start` first.
    pub fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::once(self.start).chain(self.steps.iter().map(|s| s.to.node))
    }
    /// A canonical, dependency-free FNV-1a-64 digest over `(arc, slot, sign, node)` steps + total-cost
    /// bits — pins the exact result for tests independent of transient timing. (FNV, not BLAKE3, to avoid
    /// adding a `hymeko_graph` dependency, which would be a core-manifest change; a 64-bit deterministic
    /// digest is sufficient to detect any path change.)
    ///
    /// # What this digest does NOT cover
    ///
    /// Only the **arrival** side of each step: `arc`, `to.slot`, `to.sign`, `to.node`. Two steps
    /// that share an arc and a `to` but were entered at different slots hash **identically** here.
    /// It is the historical, topological digest and must not be presented as the oriented step's
    /// full identity -- [`Self::oriented_materialisation_digest`] is that.
    ///
    /// Its bytes are unchanged from the pre-orientation implementation, so every recorded v1 value
    /// still reproduces.
    #[must_use]
    pub fn path_hash(&self) -> PathHash {
        let mut digest = PathDigest::new(b"HYMEKO-WEIGHTED-BERGE-PATH-v1", self.start);
        // The SAME four quantities in the SAME order as v1: the arc and the step's ARRIVAL
        // coordinate. Only the accessor path changed when the step gained its entry coordinate, so
        // every historical hash is reproduced bit-for-bit.
        for s in &self.steps {
            digest.write(&(s.arc.raw() as u64).to_le_bytes());
            digest.write(&s.to.slot.to_le_bytes());
            digest.write(&i64::from(s.to.sign).to_le_bytes());
            digest.write(&(s.to.node.raw() as u64).to_le_bytes());
        }
        PathHash(digest.finish(self.total_cost))
    }

    /// The digest of the path's **oriented materialisation**: start, then every step's `arc`,
    /// `from` and `to`, then the cost bits.
    ///
    /// Separately domain-separated from [`Self::path_hash`], and a different type, so the two can
    /// never be compared or stored as one another. Two steps sharing an arc and a `to` but entered
    /// at different slots produce **different** values here -- which is the whole reason it exists.
    #[must_use]
    pub fn oriented_materialisation_digest(&self) -> OrientedMaterialisationDigest {
        let mut digest = PathDigest::new(b"HYMEKO-ORIENTED-BERGE-MATERIALISATION-v1", self.start);
        for s in &self.steps {
            digest.write(&(s.arc.raw() as u64).to_le_bytes());
            for end in [s.from, s.to] {
                digest.write(&(end.node.raw() as u64).to_le_bytes());
                digest.write(&end.slot.to_le_bytes());
                digest.write(&i64::from(end.sign).to_le_bytes());
            }
        }
        OrientedMaterialisationDigest(digest.finish(self.total_cost))
    }
}

/// Per-query instrumentation (never gates results; observability for the benchmark).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchStats {
    /// Improving relaxations omitted because their certified f exceeds a
    /// complete-route incumbent (prepared witness or discovered goal), outside
    /// its rounding envelope. Equal-cost ties are preserved.
    pub bound_pruned: usize,
    /// Whole hyperarc expansions omitted by a certified Berge completion bound.
    pub bound_pruned_arcs: usize,
    /// Incidence transitions checked while probing the prepared goal-tree route.
    /// Includes a rejected transition; this work is part of the query.
    pub witness_steps: usize,
    /// A complete legal route supplied a finite initial incumbent cost.
    pub seeded_bound: bool,
    /// Nodes popped-and-settled (each once).
    pub expanded_nodes: usize,
    /// Arc expansions. Entry-independent policies and costs reuse an expansion
    /// unless a later entry has a strictly smaller g (possible under A*).
    pub expanded_arcs: usize,
    /// Incidences resolved (member visits) during relaxation.
    pub resolved_incidences: usize,
    /// Frontier insertions/updates (start entry plus relaxations that improved `g`).
    pub pushes: usize,
    /// Heap pops (including stale ones).
    pub pops: usize,
    /// Pops discarded as stale; an indexed frontier does not accumulate superseded entries.
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

/// Binary min-heap with one current entry per declaration. Inverse positions
/// use sparse-set membership: a stale position is valid only if it points back
/// to the same live declaration, so clear needs no domain-sized reset.
#[derive(Debug, Default)]
struct IndexedFrontier {
    entries: Vec<Entry>,
    positions: Vec<usize>,
}

impl IndexedFrontier {
    fn ensure(&mut self, declarations: usize) {
        if self.positions.len() < declarations {
            self.positions.resize(declarations, 0);
        }
        if self.entries.capacity() < declarations {
            self.entries.reserve(declarations - self.entries.len());
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn insert(&mut self, entry: Entry) {
        let slot = self.positions[entry.node.raw()];
        if self
            .entries
            .get(slot)
            .is_some_and(|old| old.node == entry.node)
        {
            let old = std::mem::replace(&mut self.entries[slot], entry);
            match entry.cmp(&old) {
                Ordering::Less => self.sift_up(slot),
                Ordering::Greater => self.sift_down(slot),
                Ordering::Equal => {}
            }
        } else {
            let slot = self.entries.len();
            self.entries.push(entry);
            self.positions[entry.node.raw()] = slot;
            self.sift_up(slot);
        }
    }

    fn pop(&mut self) -> Option<Entry> {
        let last = self.entries.pop()?;
        if self.entries.is_empty() {
            return Some(last);
        }
        let first = std::mem::replace(&mut self.entries[0], last);
        self.positions[last.node.raw()] = 0;
        self.sift_down(0);
        Some(first)
    }

    fn swap(&mut self, a: usize, b: usize) {
        self.entries.swap(a, b);
        self.positions[self.entries[a].node.raw()] = a;
        self.positions[self.entries[b].node.raw()] = b;
    }

    fn sift_up(&mut self, mut slot: usize) {
        while slot > 0 {
            let parent = (slot - 1) / 2;
            if self.entries[parent] <= self.entries[slot] {
                break;
            }
            self.swap(parent, slot);
            slot = parent;
        }
    }

    fn sift_down(&mut self, mut slot: usize) {
        loop {
            let left = 2 * slot + 1;
            if left >= self.entries.len() {
                break;
            }
            let right = left + 1;
            let child = if right < self.entries.len() && self.entries[right] < self.entries[left] {
                right
            } else {
                left
            };
            if self.entries[slot] <= self.entries[child] {
                break;
            }
            self.swap(slot, child);
            slot = child;
        }
    }
}

/// Caller-owned scratch for informed search. Typed sparse visitation clears in
/// O(1) without generation rollover. Preparation reserves each full ID domain
/// and one heap entry per declaration. Storage is retained between queries.
#[derive(Debug, Default)]
pub struct InformedSearchWorkspace {
    seen: super::workspace::SparseVisited<hymeko::common::ids::DeclTag>,
    active_arcs: super::workspace::SparseVisited<hymeko::common::ids::HyperArcTag>,
    g: Vec<f64>,
    settled: Vec<bool>,
    arc_entry_g: Vec<f64>,
    pred_node: Vec<DeclId>,
    pred_arc: Vec<HyperArcId>,
    pred_slot: Vec<u32>,
    /// The slot the predecessor occupied in `pred_arc` -- the step's ENTRY coordinate.
    ///
    /// Recorded because the expansion is the last place it is known: a node may occupy several
    /// slots of one hyperedge, so the reconstruction cannot recover it by scanning `refs`.
    pred_entry_slot: Vec<u32>,
    heap: IndexedFrontier,
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
        self.heap.ensure(n_decls);
        self.seen.ensure(n_decls);
        self.active_arcs.ensure(n_arcs);
        if self.g.len() < n_decls {
            self.g.resize(n_decls, f64::INFINITY);
            self.settled.resize(n_decls, false);
            self.pred_node.resize(n_decls, DeclId::NONE);
            self.pred_arc.resize(n_decls, HyperArcId::NONE);
            self.pred_slot.resize(n_decls, 0);
            self.pred_entry_slot.resize(n_decls, 0);
        }
        if self.arc_entry_g.len() < n_arcs {
            self.arc_entry_g.resize(n_arcs, f64::INFINITY);
        }
    }

    fn begin(&mut self) {
        self.seen.clear();
        self.active_arcs.clear();
        self.heap.clear();
        self.stats = SearchStats::default();
    }

    #[inline]
    fn g_valid(&self, d: DeclId) -> bool {
        self.seen.contains(d)
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
        if self.seen.insert(d) {
            self.settled[d.raw()] = false;
        }
    }
    #[inline]
    fn settled(&self, d: DeclId) -> bool {
        self.g_valid(d) && self.settled[d.raw()]
    }
    #[inline]
    fn mark_settled(&mut self, d: DeclId) {
        debug_assert!(self.g_valid(d));
        self.settled[d.raw()] = true;
    }
    #[inline]
    fn arc_active(&self, a: HyperArcId) -> bool {
        self.active_arcs.contains(a)
    }
    #[inline]
    fn mark_arc_active(&mut self, a: HyperArcId, entry_g: f64) {
        self.active_arcs.insert(a);
        self.arc_entry_g[a.raw()] = entry_g;
    }
    #[inline]
    fn set_pred(
        &mut self,
        node: DeclId,
        pred: DeclId,
        arc: HyperArcId,
        entry_slot: u32,
        slot: u32,
    ) {
        let i = node.raw();
        self.pred_entry_slot[i] = entry_slot;
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
    InformedSearch::new(traversal, cost, heuristic, policy)?.run(
        start,
        goal,
        ws,
        BergeBounds::default(),
    )
}

/// Query-specific certificates over the borrowed IR, never another graph.
struct BergeBounds<'a> {
    upper: f64,
    arc_hops: &'a [u32],
    minimum_step: f64,
    witness_steps: usize,
    comparison_scale: f64,
    pruning_limit: f64,
}

impl Default for BergeBounds<'_> {
    fn default() -> Self {
        Self {
            upper: f64::INFINITY,
            arc_hops: &[],
            minimum_step: 0.0,
            witness_steps: 0,
            comparison_scale: 1.0,
            pruning_limit: f64::INFINITY,
        }
    }
}

impl BergeBounds<'_> {
    fn for_domain(mut self, nodes: usize) -> Self {
        // Positive path sums contain at most V-1 additions. Bound formation
        // adds a multiplication and an addition. Use a conservative relative
        // roundoff envelope on BOTH sides, rounded outward. Ambiguous near-ties
        // are explored, never discarded (e.g. 9*0.1 versus nine additions).
        // EPSILON is twice the round-to-nearest unit roundoff; q therefore
        // also covers the usual gamma_n = n*u/(1-n*u) bound when q < 1/2.
        let q = (nodes as f64 + 2.0) * f64::EPSILON;
        self.comparison_scale = if q < 0.5 {
            ((1.0 + q) / (1.0 - q)).next_up()
        } else {
            f64::INFINITY
        };
        self.refresh_limit();
        self
    }

    fn after_arc(&self, arc: HyperArcId) -> f64 {
        match self.arc_hops.get(arc.raw()).copied() {
            None => 0.0,
            Some(u32::MAX) => f64::INFINITY,
            Some(hops) => (f64::from(hops) + 1.0) * self.minimum_step,
        }
    }

    fn refresh_limit(&mut self) {
        // An absent/nonrepresentable upper certificate cannot justify pruning.
        self.pruning_limit = if self.upper.is_finite() && self.comparison_scale.is_finite() {
            (self.upper * self.comparison_scale).next_up()
        } else {
            f64::INFINITY
        };
    }

    fn observe_goal(&mut self, cost: f64) {
        if cost < self.upper {
            self.upper = cost;
            self.refresh_limit();
        }
    }

    #[inline]
    fn prunes(&self, lower: f64) -> bool {
        lower > self.pruning_limit
    }
}

/// The sole informed-search engine. Public facades select costs, heuristics and
/// certificates; they share expansion, frontier, validation and reconstruction.
struct InformedSearch<'a, 'index, 'ir, C, H, P> {
    traversal: &'a HyperTraversal<'index, 'ir>,
    cost: &'a C,
    heuristic: &'a H,
    policy: &'a P,
}

impl<'a, 'index, 'ir, C: BergeCost, H: BergeHeuristic, P: TransitionPolicy>
    InformedSearch<'a, 'index, 'ir, C, H, P>
{
    fn new(
        traversal: &'a HyperTraversal<'index, 'ir>,
        cost: &'a C,
        heuristic: &'a H,
        policy: &'a P,
    ) -> Result<Self, InformedSearchError> {
        if !H::CONSISTENT || !H::ADMISSIBLE {
            return Err(InformedSearchError::InconsistentHeuristicInOptimalMode);
        }
        Ok(Self {
            traversal,
            cost,
            heuristic,
            policy,
        })
    }

    fn step_cost(&self, from: DeclId, to: &IncidenceRef<'_>) -> Result<f64, InformedSearchError> {
        let cost = self.cost.step_cost(from, to);
        if !cost.is_finite() || cost < 0.0 {
            return Err(InformedSearchError::BadCost {
                arc: to.arc,
                slot: to.slot,
                value_bits: cost.to_bits(),
            });
        }
        Ok(cost)
    }

    fn run(
        &self,
        start: NodeId,
        goal: NodeId,
        ws: &mut InformedSearchWorkspace,
        bounds: BergeBounds<'_>,
    ) -> Result<Option<WeightedBergePath>, InformedSearchError> {
        let Self {
            traversal,
            heuristic,
            policy,
            ..
        } = self;
        let ir = traversal.ir();
        let index = traversal.index();
        let start_decl = node_to_decl(ir, start)?;
        let goal_decl = node_to_decl(ir, goal)?;
        let mut bounds = bounds.for_domain(ir.nodes.len());
        ws.ensure(ir.decl_nodes.len(), ir.arcs.len());
        ws.begin();
        ws.stats.witness_steps = bounds.witness_steps;
        ws.stats.seeded_bound = bounds.upper.is_finite();

        if start_decl == goal_decl {
            return Ok(Some(WeightedBergePath {
                start,
                steps: Vec::new(),
                total_cost: 0.0,
            }));
        }

        ws.set_g(start_decl, 0.0);
        let h0 = finite_or_zero(heuristic.h(ir, start_decl));
        ws.heap.insert(Entry {
            key: Key {
                f: h0,
                tie: heuristic.tiebreak(ir, start_decl),
                decl: start_decl.raw(),
                arc: HyperArcId::NONE.raw(),
                slot: 0,
            },
            node: start_decl,
            g_at_push: 0.0,
        });
        ws.stats.pushes += 1;

        while let Some(entry) = ws.heap.pop() {
            ws.stats.pops += 1;
            let node = entry.node;
            // Retain the settled-state guard; superseded frontier entries are replaced in place.
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
                return reconstruct(ws, index, ir, start, start_decl, goal_decl, total_cost)
                    .map(Some);
            }
            let gu = ws.g(node);
            let arcs = index.incident_arcs(node);
            let entry_slots = index.incident_slots(node);
            debug_assert_eq!(arcs.len(), entry_slots.len());
            for (&arc, &entry_slot) in arcs.iter().zip(entry_slots) {
                // Every permitted exit is among these undirected members. A lower
                // bound over all of them also bounds any policy-filtered subset.
                if !bounds.arc_hops.is_empty() && bounds.prunes(gu + bounds.after_arc(arc)) {
                    ws.stats.bound_pruned_arcs += 1;
                    continue;
                }
                if C::EXIT_INDEPENDENT && policy.entry_independent() {
                    // A* settles by f, so the first member need not have the best g.
                    // Only an entry with no lower g is dominated by prior work.
                    if ws.arc_active(arc) && gu >= ws.arc_entry_g[arc.raw()] {
                        continue;
                    }
                    ws.mark_arc_active(arc, gu);
                }
                ws.stats.expanded_arcs += 1;
                let from_iref = index.resolve(arc, entry_slot);
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
                    let c = self.step_cost(node, &to_iref)?;
                    let tentative = gu + c;
                    if tentative < ws.g(target) {
                        ws.stats.heuristic_evals += 1;
                        let f = tentative + finite_or_zero(heuristic.h(ir, target));
                        if bounds.prunes(f) {
                            ws.stats.bound_pruned += 1;
                            continue;
                        }
                        ws.set_g(target, tentative);
                        ws.set_pred(target, node, arc, entry_slot, slot as u32);
                        if target == goal_decl {
                            bounds.observe_goal(tentative);
                        }
                        ws.heap.insert(Entry {
                            key: Key {
                                f,
                                tie: heuristic.tiebreak(ir, target),
                                decl: target.raw(),
                                arc: arc.raw(),
                                slot: slot as u32,
                            },
                            node: target,
                            g_at_push: tentative,
                        });
                        ws.stats.pushes += 1;
                    }
                }
            }
        }
        Ok(None)
    }
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
        let pred_entry_slot = ws.pred_entry_slot[i];
        let node_id = decl_to_node(ir, cur)
            .ok_or(InformedSearchError::PredecessorInconsistency { node: cur })?;
        // BOTH ends from their RECORDED coordinates. Nothing is recovered by scanning `refs`, and
        // the entry end is not inferred from `pred_node`: a node may sit in several slots of one
        // arc, so only the coordinate the expansion stored identifies it.
        let from = BergeIncidence::at(ir, pred_arc, pred_entry_slot)
            .ok_or(InformedSearchError::PredecessorInconsistency { node: cur })?;
        let iref = index.resolve(pred_arc, pred_slot);
        steps_rev.push(BergeStep {
            arc: pred_arc,
            from,
            to: BergeIncidence {
                node: node_id,
                slot: pred_slot,
                sign: iref.sign,
            },
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

    #[test]
    fn berge_certificates_preserve_roundoff_ties_and_require_a_finite_incumbent() {
        let hops = [0, 2, u32::MAX];
        let mut bounds = BergeBounds {
            upper: 10.0,
            arc_hops: &hops,
            minimum_step: 0.25,
            ..BergeBounds::default()
        }
        .for_domain(10);
        assert_eq!(bounds.after_arc(HyperArcId::new(0)), 0.25);
        assert_eq!(bounds.after_arc(HyperArcId::new(1)), 0.75);
        assert_eq!(bounds.after_arc(HyperArcId::new(2)), f64::INFINITY);
        assert_eq!(bounds.after_arc(HyperArcId::new(3)), 0.0);
        assert!(!bounds.prunes(10.0));
        assert!(!bounds.prunes(10.0_f64.next_up()));
        assert!(bounds.prunes(11.0));
        bounds.observe_goal(8.0);
        assert!(bounds.prunes(9.0));
        bounds.observe_goal(12.0);
        assert!(
            bounds.prunes(9.0),
            "a worse goal must not loosen the incumbent"
        );
        assert!(!BergeBounds::default().for_domain(10).prunes(f64::MAX));
        assert!(
            !BergeBounds {
                upper: 1.0,
                ..BergeBounds::default()
            }
            .for_domain(usize::MAX)
            .prunes(2.0)
        );
    }

    #[test]
    fn sparse_queries_reset_g_settlement_and_arc_activation() {
        let mut ws = InformedSearchWorkspace::new();
        ws.ensure(8, 4);
        for round in 0..64 {
            ws.begin();
            let node = DeclId::new(round % 8);
            let arc = HyperArcId::new(round % 4);
            assert_eq!(ws.g(node), f64::INFINITY);
            assert!(!ws.settled(node));
            assert!(!ws.arc_active(arc));
            assert!(!ws.settled(DeclId::NONE));
            ws.set_g(node, 3.0);
            ws.mark_settled(node);
            ws.set_g(node, 2.0);
            assert!(ws.settled(node), "same-query updates keep settlement");
            assert_eq!(ws.g(node), 2.0);
            ws.mark_arc_active(arc, 3.0);
            ws.mark_arc_active(arc, 2.0);
            assert!(ws.arc_active(arc));
            assert_eq!(ws.arc_entry_g[arc.raw()], 2.0);
        }
        ws.ensure(16, 9);
        ws.begin();
        ws.set_g(DeclId::new(15), 7.0);
        ws.mark_arc_active(HyperArcId::new(8), 7.0);
        assert!(!ws.settled(DeclId::new(15)));
        assert_eq!(ws.g(DeclId::new(15)), 7.0);
        assert!(ws.arc_active(HyperArcId::new(8)));
    }
    use hymeko::ir::incidence::build_incidence_index;
    use hymeko::ir::ir::{AnnoR, ArcRec, NodeRec, SignedRefR};

    fn pop_minimum(frontier: &mut IndexedFrontier, reference: &mut [Option<Entry>]) {
        // Independent queue oracle: scan the current optional entries for the
        // minimum under the already-established canonical Key order.
        let expected = reference.iter().flatten().copied().min();
        let actual = frontier.pop();
        assert_eq!(actual, expected);
        if let Some(entry) = expected {
            assert_eq!(
                actual.unwrap().g_at_push.to_bits(),
                entry.g_at_push.to_bits()
            );
            reference[entry.node.raw()] = None;
        }
    }

    #[test]
    fn indexed_frontier_matches_minimum_scan_and_reuses_capacity() {
        let mut frontier = IndexedFrontier::default();
        assert!(frontier.pop().is_none());
        let mut seed = 0x5eed_u64;
        for size in [17, 97, 8] {
            frontier.ensure(size);
            frontier.clear();
            let allocation = frontier.entries.as_ptr();
            let capacity = frontier.entries.capacity();
            let mut reference = vec![None; size];
            for step in 0..2048 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                match (seed >> 32) % 11 {
                    0 => {
                        frontier.clear();
                        reference.fill(None);
                    }
                    1..=3 => pop_minimum(&mut frontier, &mut reference),
                    _ => {
                        let node = (seed >> 16) as usize % size;
                        let entry = Entry {
                            key: Key {
                                f: ((seed >> 24) % 101) as f64 - 50.0,
                                tie: ((seed >> 40) % 3) as u32,
                                decl: node,
                                arc: (seed >> 48) as usize,
                                slot: (step % 5) as u32,
                            },
                            node: DeclId::new(node),
                            g_at_push: step as f64,
                        };
                        frontier.insert(entry);
                        reference[node] = Some(entry);
                    }
                }
                assert_eq!(frontier.entries.len(), reference.iter().flatten().count());
                assert!(frontier.entries.len() <= size);
                assert_eq!(frontier.entries.as_ptr(), allocation);
                assert_eq!(frontier.entries.capacity(), capacity);
                for (slot, entry) in frontier.entries.iter().enumerate() {
                    assert_eq!(frontier.positions[entry.node.raw()], slot);
                    if slot > 0 {
                        assert!(frontier.entries[(slot - 1) / 2] <= *entry);
                    }
                }
            }
            while reference.iter().any(Option::is_some) {
                pop_minimum(&mut frontier, &mut reference);
            }
            pop_minimum(&mut frontier, &mut reference);
        }
    }

    #[test]
    fn indexed_frontier_replaces_payload_without_changing_an_equal_key() {
        let mut frontier = IndexedFrontier::default();
        frontier.ensure(1);
        let mut entry = Entry {
            key: Key {
                f: 2.0,
                tie: 0,
                decl: 0,
                arc: 0,
                slot: 0,
            },
            node: DeclId::new(0),
            g_at_push: 2.0,
        };
        frontier.insert(entry);
        entry.g_at_push = 1.0;
        frontier.insert(entry);
        assert_eq!(frontier.entries.len(), 1);
        assert_eq!(frontier.pop().unwrap().g_at_push, 1.0);
        assert!(frontier.pop().is_none());
    }

    #[test]
    fn improved_frontier_entries_do_not_leave_stale_pops() {
        let mut ir = chain3();
        let mut shortcut = ir.arcs[0].clone();
        let mut draft = ir.reference_draft(&shortcut.refs[1]).unwrap();
        assert_eq!(draft.sign(), 1);
        draft.atom.target = DeclId::new(2);
        draft.atom.weights = Some(vec![hymeko::ir::ir::ValueR::Num(20.0)]);
        shortcut.refs[1] = ir.store_reference(draft);
        ir.arcs.push(shortcut);
        let goal = NodeId::new(ir.nodes.len());
        let declaration = ir.push_decl(ir.decl_draft(DeclId::new(0)).unwrap());
        ir.nodes.push(NodeRec::new(
            declaration,
            hymeko::common::ids::BaseListId::NONE,
        ));
        ir.bind_node_decl(declaration, goal).unwrap();
        let index = build_incidence_index(&ir).unwrap();
        let mut workspace = InformedSearchWorkspace::new();
        let result = dijkstra_berge_path(
            &HyperTraversal::new(&index),
            NodeId::new(0),
            goal,
            &IncidenceWeightCost { base: 1.0 },
            &Undirected,
            &mut workspace,
        )
        .unwrap();
        assert!(result.is_none());
        let stats = workspace.stats();
        assert_eq!(
            stats.pushes, 4,
            "start, two discoveries, then a cheaper route"
        );
        assert_eq!(stats.pops, 3);
        assert_eq!(stats.expanded_nodes, 3);
        assert_eq!(stats.stale_pops, 0);
    }

    /// A tiny 3-node chain: arc0 = {n0,n1}, arc1 = {n1,n2}. Unit-cost distance n0->n2 is 2.
    fn chain3() -> Ir {
        let mut ir = Ir::default();
        for _ in 0..3 {
            IrConstruction::new(&mut ir)
                .append_node(
                    hymeko::common::ids::SymId::new(0),
                    AnnoR::default(),
                    Vec::new(),
                )
                .expect("bind");
        }
        for (a, b) in [(0usize, 1usize), (1, 2)] {
            ir.arcs.push(ArcRec::unowned(vec![
                SignedRefR::plain(DeclId::new(a), hymeko::ir::ir::IncidenceSign::Plus),
                SignedRefR::plain(DeclId::new(b), hymeko::ir::ir::IncidenceSign::Plus),
            ]));
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
            // BOTH coordinates must address a live reference of the same arc.
            assert!((s.from.slot as usize) < ir.arcs[s.arc.raw()].refs.len());
            assert!((s.to.slot as usize) < ir.arcs[s.arc.raw()].refs.len());
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
            IrConstruction::new(&mut ir)
                .append_node(
                    hymeko::common::ids::SymId::new(0),
                    AnnoR::default(),
                    Vec::new(),
                )
                .unwrap();
        }
        for (a, b) in [(0usize, 1usize), (2, 3)] {
            ir.arcs.push(ArcRec::unowned(vec![
                SignedRefR::plain(DeclId::new(a), hymeko::ir::ir::IncidenceSign::Plus),
                SignedRefR::plain(DeclId::new(b), hymeko::ir::ir::IncidenceSign::Plus),
            ]));
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
