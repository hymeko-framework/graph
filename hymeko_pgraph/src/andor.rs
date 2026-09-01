//! Explicit **AND/OR (T-norm / S-norm) producibility algebra** over a borrowed [`PGraphView`].
//!
//! The existing P-graph producibility closure (`msg::close_producible`) is exactly a Boolean AND/OR
//! fixpoint: a unit *fires* when **all** its inputs are available (an AND = T-norm), and a material is
//! available if it is raw **or** **any** producer fires (an OR = S-norm). This module makes that algebra
//! explicit and generic over the norm, while **preserving the exact result**:
//!
//! ```text
//! ready(u)   = T-fold over inputs(u) of avail(m)              // AND
//! avail(m)   = S( init(m) , S-fold over producers p of  T(ready(p), reliability(p)) )   // OR
//! init(m)    = 1 iff m is raw, else 0
//! ```
//!
//! [`BooleanExact`] (`T=min`, `S=max`, values in `{0,1}`) is **proved** equal to `close_producible`
//! (`andor_vv.rs`). The generalized norms ([`Godel`], [`Product`], [`Lukasiewicz`]) are **opt-in,
//! ordering-only** — they may give a relaxation / priority / bound / search order but must **never** change
//! the exact result *set*. The `+/-/~` incidence sign is an incidence **role**, not a fuzzy truth value.
//! `NaN`/`Inf`/out-of-`[0,1]` values are typed errors; the fold order is deterministic; duplicate
//! incidences stay semantic (Product mode does **not** deduplicate).

use std::collections::{BTreeSet, VecDeque};

use hymeko::common::ids::DeclId;

use crate::view::PGraphView;

// =====================================================================================================
// Norm policy
// =====================================================================================================

/// A T-norm (AND) / S-norm (OR) pair, chosen once (monomorphised — no runtime mode boolean, no
/// `Box<dyn>` in the fixpoint loop).
pub trait AndOrNorm {
    /// A stable name for reports.
    const NAME: &'static str;
    /// Whether this norm is the exact Boolean algebra (values confined to `{0,1}`, result set is the
    /// canonical producibility). Only [`BooleanExact`] sets this.
    const EXACT: bool;
    /// T-norm (AND). Associative, commutative, monotone, identity [`Self::t_identity`].
    fn t(&self, a: f64, b: f64) -> f64;
    /// S-norm / conorm (OR). Associative, commutative, monotone, identity [`Self::s_identity`].
    fn s(&self, a: f64, b: f64) -> f64;
    /// AND identity (fold seed): `T(x, t_identity) = x`. `1.0` for all norms here.
    #[inline]
    fn t_identity(&self) -> f64 {
        1.0
    }
    /// OR identity (fold seed): `S(x, s_identity) = x`. `0.0` for all norms here.
    #[inline]
    fn s_identity(&self) -> f64 {
        0.0
    }
}

/// Boolean-exact: `T=min`, `S=max`, values in `{0,1}`. Equal to `close_producible`.
#[derive(Clone, Copy, Default)]
pub struct BooleanExact;
impl AndOrNorm for BooleanExact {
    const NAME: &'static str = "boolean-exact";
    const EXACT: bool = true;
    #[inline]
    fn t(&self, a: f64, b: f64) -> f64 {
        a.min(b)
    }
    #[inline]
    fn s(&self, a: f64, b: f64) -> f64 {
        a.max(b)
    }
}

/// Gödel: `T=min`, `S=max` on `[0,1]` (the fuzzy extension of Boolean; ordering-only).
#[derive(Clone, Copy, Default)]
pub struct Godel;
impl AndOrNorm for Godel {
    const NAME: &'static str = "godel";
    const EXACT: bool = false;
    #[inline]
    fn t(&self, a: f64, b: f64) -> f64 {
        a.min(b)
    }
    #[inline]
    fn s(&self, a: f64, b: f64) -> f64 {
        a.max(b)
    }
}

/// Product: `T(a,b)=a·b`, `S(a,b)=a+b−a·b` (probabilistic; NOT idempotent — duplicate incidences matter).
#[derive(Clone, Copy, Default)]
pub struct Product;
impl AndOrNorm for Product {
    const NAME: &'static str = "product";
    const EXACT: bool = false;
    #[inline]
    fn t(&self, a: f64, b: f64) -> f64 {
        a * b
    }
    #[inline]
    fn s(&self, a: f64, b: f64) -> f64 {
        a + b - a * b
    }
}

/// Łukasiewicz: `T(a,b)=max(0,a+b−1)`, `S(a,b)=min(1,a+b)`.
#[derive(Clone, Copy, Default)]
pub struct Lukasiewicz;
impl AndOrNorm for Lukasiewicz {
    const NAME: &'static str = "lukasiewicz";
    const EXACT: bool = false;
    #[inline]
    fn t(&self, a: f64, b: f64) -> f64 {
        (a + b - 1.0).max(0.0)
    }
    #[inline]
    fn s(&self, a: f64, b: f64) -> f64 {
        (a + b).min(1.0)
    }
}

// =====================================================================================================
// Reliability
// =====================================================================================================

/// Per-unit reliability in `[0,1]` (a producer's own availability factor). Default `1.0` (fully reliable
/// — the exact-Boolean case).
pub trait Reliability {
    /// Reliability of unit `u`.
    fn of(&self, u: DeclId) -> f64;
}

/// Every unit fully reliable (`1.0`) — the exact-Boolean reliability.
#[derive(Clone, Copy, Default)]
pub struct UnitReliability;
impl Reliability for UnitReliability {
    #[inline]
    fn of(&self, _u: DeclId) -> f64 {
        1.0
    }
}

// =====================================================================================================
// Errors, config, result
// =====================================================================================================

/// A propagation could not complete.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NormError {
    /// A value was `NaN`, `±∞`, or outside `[0,1]`.
    BadValue {
        /// The offending material.
        material: DeclId,
        /// Its bit pattern (avoids `f64: Eq`).
        value_bits: u64,
    },
    /// A fuzzy propagation did not `ε`-converge within the iteration limit.
    NonConvergent {
        /// The iteration limit that was reached.
        iters: usize,
    },
}

/// Propagation stop policy.
#[derive(Clone, Copy)]
pub struct PropagationConfig {
    /// Minimum strict increase for a change to propagate (exact: `0.0`; fuzzy: e.g. `1e-9`).
    pub eps: f64,
    /// Maximum worklist iterations before [`NormError::NonConvergent`].
    pub max_iters: usize,
}

impl PropagationConfig {
    /// Exact-Boolean policy: `eps = 0`, and a monotone `{0,1}` fixpoint is finite, so the cap only guards
    /// against a logic bug (generously sized from the graph).
    #[must_use]
    pub fn boolean_exact(n_units: usize, n_materials: usize) -> Self {
        Self {
            eps: 0.0,
            max_iters: (n_units + 1) * (n_materials + 1) + n_units + 16,
        }
    }
    /// Fuzzy policy: an `ε` threshold and an explicit iteration cap.
    #[must_use]
    pub fn fuzzy(eps: f64, max_iters: usize) -> Self {
        Self { eps, max_iters }
    }
}

/// The result of a propagation: per-material availability + the producible set (availability `> eps`).
#[derive(Debug, Clone)]
pub struct Availability {
    avail: Vec<f64>,
    producible: BTreeSet<DeclId>,
    /// Worklist iterations performed (observability).
    pub iterations: usize,
}

impl Availability {
    /// Availability of material `m` (`0.0` for an unknown/untracked decl).
    #[must_use]
    pub fn of(&self, m: DeclId) -> f64 {
        self.avail.get(m.raw()).copied().unwrap_or(0.0)
    }
    /// The producible material set (availability strictly positive) — for [`BooleanExact`] this equals
    /// `close_producible`.
    #[must_use]
    pub fn producible(&self) -> &BTreeSet<DeclId> {
        &self.producible
    }
}

// =====================================================================================================
// Workspace + fixpoint
// =====================================================================================================

/// Caller-owned reusable scratch for the propagation. Dense, `DeclId`-indexed, generation-stamped; a warm
/// run allocates nothing (arrays + queue retained, marks cleared in O(1) amortised).
#[derive(Debug, Default)]
pub struct AndOrWorkspace {
    stamp: u32,
    avail: Vec<f64>,
    avail_gen: Vec<u32>,
    queued_gen: Vec<u32>,
    queue: VecDeque<DeclId>,
    /// Reused per-unit scratch for the (material, new-value) updates of one firing; `take`/restore keeps
    /// its capacity so a warm run allocates nothing.
    pending: Vec<(DeclId, f64)>,
}

impl AndOrWorkspace {
    /// A fresh workspace.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure(&mut self, n: usize) {
        if self.avail.len() < n {
            self.avail.resize(n, 0.0);
            self.avail_gen.resize(n, 0);
            self.queued_gen.resize(n, 0);
        }
    }

    fn begin(&mut self) {
        if self.stamp == u32::MAX {
            self.avail_gen.iter_mut().for_each(|g| *g = 0);
            self.queued_gen.iter_mut().for_each(|g| *g = 0);
            self.stamp = 1;
        } else {
            self.stamp += 1;
        }
        self.queue.clear();
    }

    #[inline]
    fn get(&self, d: DeclId, s_identity: f64) -> f64 {
        if self
            .avail_gen
            .get(d.raw())
            .is_some_and(|&g| g == self.stamp)
        {
            self.avail[d.raw()]
        } else {
            s_identity
        }
    }
    #[inline]
    fn set(&mut self, d: DeclId, v: f64) {
        self.avail[d.raw()] = v;
        self.avail_gen[d.raw()] = self.stamp;
    }
    #[inline]
    fn is_queued(&self, u: DeclId) -> bool {
        self.queued_gen
            .get(u.raw())
            .is_some_and(|&g| g == self.stamp)
    }
    #[inline]
    fn enqueue(&mut self, u: DeclId) {
        if !self.is_queued(u) {
            self.queued_gen[u.raw()] = self.stamp;
            self.queue.push_back(u);
        }
    }
    #[inline]
    fn dequeue(&mut self) -> Option<DeclId> {
        let u = self.queue.pop_front()?;
        self.queued_gen[u.raw()] = 0; // allow re-enqueue on a later strict increase
        Some(u)
    }
}

/// Run the AND/OR availability fixpoint over `view` under norm `norm` and per-unit `reliability`, into the
/// caller-owned `ws`. Returns the per-material availability and the producible set.
///
/// # Preconditions
/// `view` classification is consistent with the `Ir` (see [`PGraphView::validate`]).
///
/// # Errors
/// [`NormError::BadValue`] if any computed availability is `NaN`/`±∞`/outside `[0,1]`;
/// [`NormError::NonConvergent`] if a fuzzy run exceeds `cfg.max_iters` without `ε`-convergence.
pub fn propagate<N: AndOrNorm, R: Reliability>(
    view: &PGraphView<'_, '_, '_>,
    norm: &N,
    reliability: &R,
    ws: &mut AndOrWorkspace,
    cfg: PropagationConfig,
) -> Result<Availability, NormError> {
    let n = view.ir().decl_nodes.len();
    ws.ensure(n);
    ws.begin();
    let s_id = norm.s_identity();

    // init: raws to 1, other materials to the OR identity (0). Seed the worklist with every unit.
    for &m in view.materials() {
        ws.set(m, if view.is_raw(m) { 1.0 } else { s_id });
    }
    for &u in view.units() {
        ws.enqueue(u);
    }

    let mut iterations = 0usize;
    // Reuse the workspace's `pending` buffer across firings (take now, restore before returning).
    let mut pending = std::mem::take(&mut ws.pending);
    while let Some(u) = ws.dequeue() {
        iterations += 1;
        if iterations > cfg.max_iters {
            ws.pending = pending;
            return Err(NormError::NonConvergent {
                iters: cfg.max_iters,
            });
        }
        // ready(u) = T-fold over inputs (empty inputs => t_identity = 1: an input-free unit is ready).
        let mut ready = norm.t_identity();
        view.for_each_input(u, |m| {
            ready = norm.t(ready, ws.get(m, s_id));
        });
        if ready <= s_id {
            continue; // no positive contribution
        }
        let contrib = norm.t(ready, reliability.of(u));
        if contrib <= s_id {
            continue;
        }
        // For each output, S-combine and record a strict increase (read-only on ws in this closure).
        pending.clear();
        view.for_each_output(u, |m| {
            let cur = ws.get(m, s_id);
            let next = norm.s(cur, contrib);
            if next > cur + cfg.eps {
                pending.push((m, next));
            }
        });
        for &(m, next) in &pending {
            if !next.is_finite() || !(0.0..=1.0).contains(&next) {
                ws.pending = std::mem::take(&mut pending);
                return Err(NormError::BadValue {
                    material: m,
                    value_bits: next.to_bits(),
                });
            }
            ws.set(m, next);
            view.for_each_consumer(m, |c| ws.enqueue(c));
        }
    }
    ws.pending = pending; // restore capacity for the next warm run

    // materialise the result: availability per material + the producible set.
    let mut avail = vec![0.0; n];
    let mut producible = BTreeSet::new();
    for &m in view.materials() {
        let v = ws.get(m, s_id);
        avail[m.raw()] = v;
        if v > s_id {
            producible.insert(m);
        }
    }
    Ok(Availability {
        avail,
        producible,
        iterations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truth_tables() {
        let b = BooleanExact;
        assert_eq!(b.t(1.0, 1.0), 1.0);
        assert_eq!(b.t(1.0, 0.0), 0.0);
        assert_eq!(b.s(0.0, 0.0), 0.0);
        assert_eq!(b.s(1.0, 0.0), 1.0);
        let p = Product;
        assert!((p.t(0.5, 0.5) - 0.25).abs() < 1e-12);
        assert!((p.s(0.5, 0.5) - 0.75).abs() < 1e-12);
        // Product is NOT idempotent (duplicate incidences matter):
        assert!((p.s(0.5, 0.5) - 0.5).abs() > 1e-6);
        let l = Lukasiewicz;
        assert_eq!(l.t(0.3, 0.4), 0.0);
        assert!((l.s(0.6, 0.7) - 1.0).abs() < 1e-12);
    }
}
