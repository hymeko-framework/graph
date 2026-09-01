//! `PGraphView` — a **borrowed** P-graph domain view over the canonical `Ir` and its exact sparse
//! [`IncidenceIndex`]. No copied topology: all adjacency, degree and directed reachability are answered by
//! querying the incidence in place. P-graph *classification* (material / unit / raw / product) is borrowed
//! domain input (it comes from the `<isa>` lowering, which stays pgraph-owned); the *topology* comes from
//! the canonical signed incidence.
//!
//! # Sign convention (verified against `meta_resolve::build_incidence`)
//!
//! For a unit edge-decl `u`, iterate its arc references: a `-` ref (sign `< 0`) is a **consumed input**
//! (directed edge `m → u`); a `+` ref (sign `> 0`) is a **produced output** (`u → m`); a `~` ref (sign
//! `0`) is a domain error (`NeutralRef` in the existing pipeline) and is reported by [`PGraphView::validate`].
//! Dually, `producers(m) = { u : m is a + ref of u }`, `consumers(m) = { u : m is a - ref of u }`.
//!
//! # Why borrowed, not a second graph
//!
//! This replaces the owned `PGraphSchema.incoming/outgoing/edges` adjacency (four materialised copies
//! across `schema.rs`, `axioms.rs`, `axiom_extensions.rs`) and the two verbatim `reaches_any` BFS copies
//! with borrowed incidence queries — the P1.1 audit's migration target. Introduced additively; the old
//! copies are removed only behind a byte-identical parity gate.

use std::collections::BTreeSet;

use hymeko::common::ids::DeclId;
use hymeko::ir::incidence::IncidenceIndex;
use hymeko::ir::ir::Ir;

use crate::lowering::LoweredPGraph;

/// A borrowed P-graph view: classification sets + the exact incidence index. Zero owned topology.
#[derive(Clone, Copy)]
pub struct PGraphView<'a, 'ir, 'idx> {
    index: &'idx IncidenceIndex<'ir>,
    units: &'a BTreeSet<DeclId>,
    materials: &'a BTreeSet<DeclId>,
    raws: &'a BTreeSet<DeclId>,
    products: &'a BTreeSet<DeclId>,
}

/// A structural fault found while validating a `PGraphView` against the canonical `Ir`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewError {
    /// A unit carries a neutral (`~`) incidence, which has no producer/consumer orientation.
    NeutralUnitRef {
        /// The unit edge-decl.
        unit: DeclId,
        /// The neutral-referenced target.
        target: DeclId,
    },
    /// A unit references a target that is not a classified material.
    NonMaterialTarget {
        /// The unit edge-decl.
        unit: DeclId,
        /// The non-material target.
        target: DeclId,
    },
}

impl<'a, 'ir, 'idx> PGraphView<'a, 'ir, 'idx> {
    /// Build a view from the incidence index and the borrowed classification sets.
    #[must_use]
    pub fn new(
        index: &'idx IncidenceIndex<'ir>,
        units: &'a BTreeSet<DeclId>,
        materials: &'a BTreeSet<DeclId>,
        raws: &'a BTreeSet<DeclId>,
        products: &'a BTreeSet<DeclId>,
    ) -> Self {
        Self {
            index,
            units,
            materials,
            raws,
            products,
        }
    }

    /// Build a view borrowing a lowered P-graph's classification (the classification stays domain-owned;
    /// only the topology is taken from the incidence, not the schema's copied adjacency).
    #[must_use]
    pub fn from_lowered(index: &'idx IncidenceIndex<'ir>, p: &'a LoweredPGraph) -> Self {
        Self::new(index, &p.units, &p.materials, &p.raws, &p.products)
    }

    /// The borrowed canonical `Ir`.
    #[inline]
    pub fn ir(&self) -> &'ir Ir {
        self.index.ir()
    }

    // ---- classification --------------------------------------------------------------------------
    /// Whether `d` is a classified material.
    #[must_use]
    pub fn is_material(&self, d: DeclId) -> bool {
        self.materials.contains(&d)
    }
    /// Whether `d` is a classified operating unit.
    #[must_use]
    pub fn is_unit(&self, d: DeclId) -> bool {
        self.units.contains(&d)
    }
    /// Whether `d` is a raw material.
    #[must_use]
    pub fn is_raw(&self, d: DeclId) -> bool {
        self.raws.contains(&d)
    }
    /// Whether `d` is a required product.
    #[must_use]
    pub fn is_product(&self, d: DeclId) -> bool {
        self.products.contains(&d)
    }
    /// The unit set.
    #[must_use]
    pub fn units(&self) -> &'a BTreeSet<DeclId> {
        self.units
    }
    /// The material set.
    #[must_use]
    pub fn materials(&self) -> &'a BTreeSet<DeclId> {
        self.materials
    }
    /// The raw-material set.
    #[must_use]
    pub fn raws(&self) -> &'a BTreeSet<DeclId> {
        self.raws
    }
    /// The required-product set.
    #[must_use]
    pub fn products(&self) -> &'a BTreeSet<DeclId> {
        self.products
    }

    // ---- allocation-free directed adjacency (hot path) ------------------------------------------

    /// Call `f` on each **input** material of unit `u` (its `-` refs), including duplicates (a duplicate
    /// incidence stays semantic — it is not deduplicated). Allocation-free.
    pub fn for_each_input(&self, u: DeclId, mut f: impl FnMut(DeclId)) {
        self.for_each_unit_ref(u, |target, sign| {
            if sign < 0 {
                f(target);
            }
        });
    }

    /// Call `f` on each **output** material of unit `u` (its `+` refs), including duplicates.
    pub fn for_each_output(&self, u: DeclId, mut f: impl FnMut(DeclId)) {
        self.for_each_unit_ref(u, |target, sign| {
            if sign > 0 {
                f(target);
            }
        });
    }

    /// Call `f` on each unit that **produces** material `m` (units with a `+` ref to `m`), including
    /// duplicates. Allocation-free (queries `incident_arcs`).
    pub fn for_each_producer(&self, m: DeclId, f: impl FnMut(DeclId)) {
        self.for_each_incident_unit(m, 1, f);
    }

    /// Call `f` on each unit that **consumes** material `m` (units with a `-` ref to `m`), including
    /// duplicates. Allocation-free.
    pub fn for_each_consumer(&self, m: DeclId, f: impl FnMut(DeclId)) {
        self.for_each_incident_unit(m, -1, f);
    }

    fn for_each_unit_ref(&self, u: DeclId, mut f: impl FnMut(DeclId, i8)) {
        let ir = self.ir();
        if let Some((_eid, rec)) = ir.edge_for_decl(u) {
            for &aid in &rec.arcs {
                if let Some(arc) = ir.arc(aid) {
                    for sref in &arc.refs {
                        f(sref.target(), sref.sign());
                    }
                }
            }
        }
    }

    /// For each incident arc of material `m` whose sign matches `want_sign` (`1` produced / `-1` consumed),
    /// yield the owning unit decl. Duplicates preserved.
    fn for_each_incident_unit(&self, m: DeclId, want_sign: i8, mut f: impl FnMut(DeclId)) {
        let ir = self.ir();
        let arcs = self.index.incident_arcs(m);
        let slots = self.index.incident_slots(m);
        for (i, &arc) in arcs.iter().enumerate() {
            let slot = slots[i];
            let sign = self.index.resolve(arc, slot).sign;
            if sign == want_sign {
                let owner = ir.arcs[arc.raw()].in_edge;
                if self.units.contains(&owner) {
                    f(owner);
                }
            }
        }
    }

    // ---- set-returning convenience (tests / parity; may allocate) --------------------------------

    /// Inputs of `u` as a deduplicated set (parity with `LoweredPGraph::inputs`).
    #[must_use]
    pub fn inputs_set(&self, u: DeclId) -> BTreeSet<DeclId> {
        let mut s = BTreeSet::new();
        self.for_each_input(u, |m| {
            s.insert(m);
        });
        s
    }
    /// Outputs of `u` as a deduplicated set (parity with `LoweredPGraph::outputs`).
    #[must_use]
    pub fn outputs_set(&self, u: DeclId) -> BTreeSet<DeclId> {
        let mut s = BTreeSet::new();
        self.for_each_output(u, |m| {
            s.insert(m);
        });
        s
    }
    /// Producers of `m` as a deduplicated set (parity with `LoweredPGraph::producers`).
    #[must_use]
    pub fn producers_set(&self, m: DeclId) -> BTreeSet<DeclId> {
        let mut s = BTreeSet::new();
        self.for_each_producer(m, |u| {
            s.insert(u);
        });
        s
    }
    /// Consumers of `m` as a deduplicated set (parity with `LoweredPGraph::consumers`).
    #[must_use]
    pub fn consumers_set(&self, m: DeclId) -> BTreeSet<DeclId> {
        let mut s = BTreeSet::new();
        self.for_each_consumer(m, |u| {
            s.insert(u);
        });
        s
    }

    /// In-degree of unit `u` (number of input incidences, with multiplicity).
    #[must_use]
    pub fn in_degree(&self, u: DeclId) -> usize {
        let mut n = 0;
        self.for_each_input(u, |_| n += 1);
        n
    }
    /// Out-degree of unit `u` (number of output incidences, with multiplicity).
    #[must_use]
    pub fn out_degree(&self, u: DeclId) -> usize {
        let mut n = 0;
        self.for_each_output(u, |_| n += 1);
        n
    }

    // ---- directed reachability (replaces `reaches_any`; OR-reachability, sign-oriented) ----------

    /// Forward directed reachability from `start`: the set of decls reachable by following produced/consumed
    /// orientation `material → consuming unit → its outputs → …` (the `reaches_any` semantics). This is
    /// **OR**-reachability (a unit is entered if *any* of its inputs is reached) — it is *not* producibility
    /// (which is the AND-closure in [`crate::andor`]); use it as a connectivity primitive / bound.
    #[must_use]
    pub fn forward_reachable(&self, start: DeclId) -> BTreeSet<DeclId> {
        let mut seen: BTreeSet<DeclId> = BTreeSet::new();
        let mut stack = vec![start];
        seen.insert(start);
        while let Some(d) = stack.pop() {
            if self.is_material(d) {
                // material -> consuming units
                self.for_each_consumer(d, |u| {
                    if seen.insert(u) {
                        stack.push(u);
                    }
                });
            } else if self.is_unit(d) {
                // unit -> its outputs
                self.for_each_output(d, |m| {
                    if seen.insert(m) {
                        stack.push(m);
                    }
                });
            }
        }
        seen
    }

    /// Validate the view against the canonical `Ir`: every unit ref is a `+`/`-` to a classified material
    /// (no `~`, no non-material target). Mirrors `meta_resolve::build_incidence`'s fail-closed checks.
    ///
    /// # Errors
    /// [`ViewError`] on the first fault.
    pub fn validate(&self) -> Result<(), ViewError> {
        let ir = self.ir();
        for &u in self.units {
            let mut err = None;
            self.for_each_unit_ref(u, |target, sign| {
                if err.is_some() {
                    return;
                }
                if sign == 0 {
                    err = Some(ViewError::NeutralUnitRef { unit: u, target });
                } else if !self.materials.contains(&target) {
                    err = Some(ViewError::NonMaterialTarget { unit: u, target });
                }
            });
            let _ = ir;
            if let Some(e) = err {
                return Err(e);
            }
        }
        Ok(())
    }
}
