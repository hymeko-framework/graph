//! Extension axioms (E-bundle) — orthogonal Friedler 1992 refinements.
//!
//! The five **canonical Friedler 1992 axioms** S1..S5 are implemented in
//! [`crate::axioms`]. This module ships an *extension set* of three
//! additional structural conditions that the pre-2026-05-19 axiom
//! checker conflated with the canonical axioms. After the Pimentel
//! audit corrected the canonical semantics, the previous statements
//! were preserved here as a named, optional bundle so that:
//!
//! 1. downstream NAS / synthesis work can experiment with stricter
//!    structural filters without changing the canonical axiom
//!    semantics;
//! 2. if a future search outcome (e.g.\ HSiKAN architecture
//!    enumeration, GömbSoma circuit synthesis) regresses under the
//!    canonical axioms alone, the extension bundle can be re-enabled
//!    as a hypothesis to test;
//! 3. the prior implementations remain on disk as concrete reference
//!    points, not lost to git history alone.
//!
//! # Compatibility with canonical S1..S5
//!
//! The 2026-05-19 audit (see
//! `reports/2026-05-19-pgraph-axiom-semantics-fix-phase2.md`)
//! established that **none of the extension axioms contradict
//! canonical S1..S5**. Specifically:
//!
//! - **E-StrictNoExcess** (formerly "Old A2": $\forall m \in M$,
//!   $m$ has a directed path to some required product).
//!   Equivalent to canonical $\{S1, S2, S4, S5\}$ ∧ the **strict
//!   no-excess** Friedler refinement (Friedler 1992 §3, named as
//!   an orthogonal strengthener — explicitly *not* one of S1..S5).
//!   Implication chain: enforcing E-StrictNoExcess implies every
//!   non-product material is consumed downstream, which is the
//!   strict no-excess rule.
//! - **E-UnitWellFormed** (formerly "Old A4": $\forall o \in O$,
//!   $\mathrm{in\_deg}(o) \geq 1$ and $\mathrm{out\_deg}(o) \geq 1$).
//!   Orthogonal to S4. Encodes the Friedler 1992 §2 *structural
//!   prerequisite* that an operating unit consumes ≥ 1 input and
//!   produces ≥ 1 output. Useful as a quick well-formedness gate
//!   at schema-construction time.
//! - **E-ConsumedHasProducer** (formerly "Old A5": consumed M-node
//!   without producer and not raw). Strict subset of canonical
//!   A2-forward (only fires on consumed M-nodes; canonical A2
//!   additionally catches *isolated* non-raw M-nodes through A5/S5).
//!   Cheaper one-edge-pass check; useful when full A2 is too
//!   expensive on very large IRs.
//!
//! # Usage
//!
//! ```ignore
//! use hymeko_pgraph::axiom_extensions::ExtensionAxiomBundle;
//! let ext = ExtensionAxiomBundle::new([raw_a, raw_b]);
//! let violations = ext.validate(&schema, &required_products);
//! ```

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hymeko::common::ids::DeclId;

use crate::schema::{PGraphSchema, PNodeKind};

/// One concrete violation of an extension axiom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionAxiomViolation {
    /// **E-StrictNoExcess (former Old A2).** One or more M-nodes
    /// have no directed path to any required product. Implication:
    /// the schema produces excess (waste) materials that are never
    /// consumed downstream nor are required outputs.
    NonReachingMaterials {
        /// M-nodes that fail to reach any required product.
        offenders: Vec<DeclId>,
    },
    /// **E-UnitWellFormed (former Old A4).** One or more O-nodes
    /// have $\mathrm{in\_deg} = 0$ or $\mathrm{out\_deg} = 0$.
    /// Implication: the schema contains units that are not
    /// well-formed under Friedler 1992 §2's structural
    /// prerequisite for operating units.
    UnitsWithDegreeZero {
        /// O-nodes with zero input or zero output.
        offenders: Vec<DeclId>,
    },
    /// **E-ConsumedHasProducer (former Old A5).** One or more
    /// M-nodes are consumed by some unit, have no producer, and
    /// are not declared raw. Strict subset of canonical A2-forward.
    ConsumedMaterialWithoutProducer {
        /// Consumed non-raw M-nodes without any producer.
        offenders: Vec<DeclId>,
    },
}

/// Bundle of *extension* axiom checks (E-axioms).
///
/// Use this **in addition to** [`crate::axioms::AxiomBundle`] when a
/// stricter structural filter is desired. The two bundles are
/// independent — running both on the same schema produces violation
/// lists that are pairwise consistent (no contradictions, by the
/// audit's correctness proof).
#[derive(Debug, Default)]
pub struct ExtensionAxiomBundle {
    /// Raw set — used by E-ConsumedHasProducer to whitelist raw
    /// consumed materials (which legitimately have no producer).
    pub raws: BTreeSet<DeclId>,
}

impl ExtensionAxiomBundle {
    /// Construct with an explicit raw set.
    pub fn new(raws: impl IntoIterator<Item = DeclId>) -> Self {
        Self {
            raws: raws.into_iter().collect(),
        }
    }

    /// Run every E-axiom and return the list of violations
    /// encountered. An empty `Ok(())` means the schema satisfies
    /// every extension axiom.
    pub fn validate(
        &self,
        schema: &PGraphSchema,
        required_products: &BTreeSet<DeclId>,
    ) -> Result<(), Vec<ExtensionAxiomViolation>> {
        let adj = build_forward_adj(schema);
        let mut violations = Vec::new();

        if let Some(v) = check_strict_no_excess(schema, &adj, required_products) {
            violations.push(v);
        }
        if let Some(v) = check_unit_well_formed(schema) {
            violations.push(v);
        }
        if let Some(v) = check_consumed_has_producer(schema, &self.raws) {
            violations.push(v);
        }

        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }
}

// ─── Per-extension checks ────────────────────────────────────────────

/// **E-StrictNoExcess (former Old A2).** Every M-node has a directed
/// path to some required product.
fn check_strict_no_excess(
    schema: &PGraphSchema,
    adj: &BTreeMap<DeclId, Vec<DeclId>>,
    required_products: &BTreeSet<DeclId>,
) -> Option<ExtensionAxiomViolation> {
    if required_products.is_empty() {
        return None;
    }
    let mut offenders = Vec::new();
    for m in schema.m_nodes() {
        if required_products.contains(&m) {
            continue;
        }
        if !reaches_any(adj, m, required_products) {
            offenders.push(m);
        }
    }
    if offenders.is_empty() {
        None
    } else {
        Some(ExtensionAxiomViolation::NonReachingMaterials { offenders })
    }
}

/// **E-UnitWellFormed (former Old A4).** Every O-node has
/// `in_degree ≥ 1` and `out_degree ≥ 1`.
fn check_unit_well_formed(schema: &PGraphSchema) -> Option<ExtensionAxiomViolation> {
    let mut offenders = Vec::new();
    for o in schema.o_nodes() {
        if schema.in_degree(o) == 0 || schema.out_degree(o) == 0 {
            offenders.push(o);
        }
    }
    if offenders.is_empty() {
        None
    } else {
        Some(ExtensionAxiomViolation::UnitsWithDegreeZero { offenders })
    }
}

/// **E-ConsumedHasProducer (former Old A5).** Every consumed
/// non-raw M-node has at least one producer.
fn check_consumed_has_producer(
    schema: &PGraphSchema,
    raws: &BTreeSet<DeclId>,
) -> Option<ExtensionAxiomViolation> {
    let mut consumed: BTreeSet<DeclId> = BTreeSet::new();
    let mut produced: BTreeSet<DeclId> = BTreeSet::new();
    for (_, src, dst) in schema.edges() {
        match (schema.kind(src), schema.kind(dst)) {
            (Some(PNodeKind::Material), _) => {
                consumed.insert(src);
            }
            (_, Some(PNodeKind::Material)) => {
                produced.insert(dst);
            }
            _ => {}
        }
    }
    let mut offenders = Vec::new();
    for m in schema.m_nodes() {
        if consumed.contains(&m) && !produced.contains(&m) && !raws.contains(&m) {
            offenders.push(m);
        }
    }
    if offenders.is_empty() {
        None
    } else {
        Some(ExtensionAxiomViolation::ConsumedMaterialWithoutProducer { offenders })
    }
}

// ─── Helpers (mirror axioms.rs, kept private so the canonical and
//     extension modules don't share private types). ──────────────────

fn build_forward_adj(schema: &PGraphSchema) -> BTreeMap<DeclId, Vec<DeclId>> {
    let mut adj: BTreeMap<DeclId, Vec<DeclId>> = BTreeMap::new();
    for (_, src, dst) in schema.edges() {
        adj.entry(src).or_default().push(dst);
    }
    adj
}

fn reaches_any(
    adj: &BTreeMap<DeclId, Vec<DeclId>>,
    from: DeclId,
    targets: &BTreeSet<DeclId>,
) -> bool {
    let mut visited: BTreeSet<DeclId> = BTreeSet::new();
    let mut q: VecDeque<DeclId> = VecDeque::new();
    q.push_back(from);
    visited.insert(from);
    while let Some(v) = q.pop_front() {
        if let Some(ns) = adj.get(&v) {
            for &n in ns {
                if targets.contains(&n) {
                    return true;
                }
                if visited.insert(n) {
                    q.push_back(n);
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{PGraphSchema, PNodeKind};
    use hymeko::common::ids::EdgeId;
    use std::collections::BTreeMap;

    fn d(i: usize) -> DeclId {
        DeclId::new(i)
    }
    fn e(i: usize) -> EdgeId {
        EdgeId::new(i)
    }

    // ─── E-StrictNoExcess ───────────────────────────────────────

    /// E-StrictNoExcess catches a by-product M-node that canonical
    /// S1..S5 does *not* catch. This is the central exhibit: the
    /// schema is canonical-feasible but the extension axiom flags
    /// the waste material.
    #[test]
    fn strict_no_excess_catches_canonical_feasible_byproduct() {
        // raw d(0) → U1 → product d(2) AND U1 also → by-product d(3)
        // (a non-product, non-consumed material). Canonical S4
        // passes (U1 reaches d(2)); canonical S2 passes; canonical
        // S5 passes (d(3) has incident edge from U1). But d(3) does
        // NOT reach any product, so the strict-no-excess refinement
        // fires.
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material),
            (d(1), PNodeKind::OperatingUnit),
            (d(2), PNodeKind::Material),
            (d(3), PNodeKind::Material), // by-product
        ]);
        let edges = BTreeMap::from([
            (e(0), (d(0), d(1))),
            (e(1), (d(1), d(2))),
            (e(2), (d(1), d(3))),
        ]);
        let schema = PGraphSchema::try_new(kinds, edges).unwrap();
        let ext = ExtensionAxiomBundle::new([d(0)]);
        let products = BTreeSet::from([d(2)]);
        let v = ext.validate(&schema, &products).expect_err("must err");
        assert!(v.iter().any(|x| matches!(
            x, ExtensionAxiomViolation::NonReachingMaterials { offenders }
                if offenders.contains(&d(3))
        )));

        // Sanity: the canonical bundle ACCEPTS this same schema.
        let bundle = crate::axioms::AxiomBundle::new([d(0)], []);
        bundle.validate(&schema, &products).expect(
            "canonical S1..S5 should accept a by-product schema; \
             strict-no-excess is an orthogonal extension",
        );
    }

    // ─── E-UnitWellFormed ──────────────────────────────────────

    /// E-UnitWellFormed catches a unit that consumes from nothing
    /// (zero in-degree). Canonical S4 passes if the unit's outputs
    /// reach products — so the canonical bundle is silent here.
    #[test]
    fn unit_well_formed_catches_zero_in_degree() {
        // U1 has NO inputs (a "source" unit) but produces the
        // product directly. Canonical S4 passes (U1 → d(2)).
        // E-UnitWellFormed fires on U1.
        let kinds = BTreeMap::from([
            (d(1), PNodeKind::OperatingUnit),
            (d(2), PNodeKind::Material), // product
        ]);
        let edges = BTreeMap::from([(e(0), (d(1), d(2)))]);
        let schema = PGraphSchema::try_new(kinds, edges).unwrap();
        let ext = ExtensionAxiomBundle::new([]);
        let products = BTreeSet::from([d(2)]);
        let v = ext.validate(&schema, &products).expect_err("must err");
        assert!(v.iter().any(|x| matches!(
            x, ExtensionAxiomViolation::UnitsWithDegreeZero { offenders }
                if offenders.contains(&d(1))
        )));
    }

    /// E-UnitWellFormed catches a unit that produces to nothing
    /// (zero out-degree). Canonical S4 *also* fires here (no path
    /// to product), but the two violations are reported
    /// independently.
    #[test]
    fn unit_well_formed_catches_zero_out_degree() {
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material), // raw
            (d(1), PNodeKind::OperatingUnit),
            (d(2), PNodeKind::Material),      // product (unreachable)
            (d(3), PNodeKind::OperatingUnit), // produces the product
        ]);
        let edges = BTreeMap::from([
            (e(0), (d(0), d(1))), // U1 consumes d(0), produces nothing
            (e(1), (d(0), d(3))),
            (e(2), (d(3), d(2))),
        ]);
        let schema = PGraphSchema::try_new(kinds, edges).unwrap();
        let ext = ExtensionAxiomBundle::new([d(0)]);
        let products = BTreeSet::from([d(2)]);
        let v = ext.validate(&schema, &products).expect_err("must err");
        assert!(v.iter().any(|x| matches!(
            x, ExtensionAxiomViolation::UnitsWithDegreeZero { offenders }
                if offenders.contains(&d(1))
        )));
    }

    // ─── E-ConsumedHasProducer ─────────────────────────────────

    /// E-ConsumedHasProducer fires the same way canonical A2-forward
    /// does on a consumed non-raw without a producer — confirming
    /// they overlap. This test exists so a reader can verify the
    /// "strict subset of canonical A2-forward" claim from the
    /// docstring.
    #[test]
    fn consumed_has_producer_overlap_with_canonical_a2() {
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material),
            (d(1), PNodeKind::OperatingUnit),
            (d(2), PNodeKind::Material),
            (d(3), PNodeKind::Material), // consumed, no producer, not raw
            (d(4), PNodeKind::OperatingUnit),
        ]);
        let edges = BTreeMap::from([
            (e(0), (d(0), d(1))),
            (e(1), (d(1), d(2))),
            (e(2), (d(3), d(4))), // d(3) consumed by U2
            (e(3), (d(4), d(2))),
        ]);
        let schema = PGraphSchema::try_new(kinds, edges).unwrap();
        let ext = ExtensionAxiomBundle::new([d(0)]); // d(3) NOT raw
        let products = BTreeSet::from([d(2)]);
        let v = ext.validate(&schema, &products).expect_err("must err");
        assert!(v.iter().any(|x| matches!(
            x, ExtensionAxiomViolation::ConsumedMaterialWithoutProducer { offenders }
                if offenders.contains(&d(3))
        )));

        // Canonical A2 ALSO catches this exact case (consumed
        // is irrelevant; ¬raw ∧ ¬has-ancestor is the canonical
        // forward direction).
        let bundle = crate::axioms::AxiomBundle::new([d(0)], []);
        let v_canon = bundle.validate(&schema, &products).expect_err("must err");
        assert!(v_canon.iter().any(|x| matches!(
            x, crate::axioms::AxiomViolation::RawMaterialDirectionFailures {
                non_raw_without_producer, ..
            } if non_raw_without_producer.contains(&d(3))
        )));
    }

    /// E-ConsumedHasProducer is silent on an ISOLATED non-raw M-node
    /// (no incident edge at all). Canonical A2-forward catches it
    /// (via A5's prerequisite path). This pins the strict-subset
    /// relationship: canonical catches the isolated case, E does not.
    #[test]
    fn consumed_has_producer_silent_on_isolated_m_node() {
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material),
            (d(1), PNodeKind::OperatingUnit),
            (d(2), PNodeKind::Material),
            (d(3), PNodeKind::Material), // isolated non-raw, not consumed
        ]);
        let edges = BTreeMap::from([(e(0), (d(0), d(1))), (e(1), (d(1), d(2)))]);
        let schema = PGraphSchema::try_new(kinds, edges).unwrap();
        let ext = ExtensionAxiomBundle::new([d(0)]);
        let products = BTreeSet::from([d(2)]);
        // E-ConsumedHasProducer accepts (d(3) is not in `consumed`).
        let consumed_check = check_consumed_has_producer(&schema, &ext.raws);
        assert!(
            consumed_check.is_none(),
            "E-ConsumedHasProducer must be silent on an isolated M-node"
        );

        // Canonical bundle FAILS (A2 forward + A5).
        let bundle = crate::axioms::AxiomBundle::new([d(0)], []);
        let err = bundle
            .validate(&schema, &products)
            .expect_err("canonical must err");
        assert!(err.iter().any(|x| matches!(
            x, crate::axioms::AxiomViolation::IsolatedMaterials { offenders }
                if offenders.contains(&d(3))
        )));
    }

    // ─── Compatibility theorem (no-contradiction) ──────────────

    /// Theorem: a schema that satisfies canonical S1..S5 AND
    /// strict-no-excess AND unit-well-formedness satisfies every
    /// extension axiom. (Demonstrated on a concrete fixture.)
    #[test]
    fn canonical_plus_extension_compatible_on_chapter_4_style_fixture() {
        // Two raws → one unit → one product. Satisfies S1..S5
        // and every extension axiom (no by-product, unit has both
        // in and out, no consumed-without-producer).
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material),
            (d(1), PNodeKind::Material),
            (d(2), PNodeKind::OperatingUnit),
            (d(3), PNodeKind::Material),
        ]);
        let edges = BTreeMap::from([
            (e(0), (d(0), d(2))),
            (e(1), (d(1), d(2))),
            (e(2), (d(2), d(3))),
        ]);
        let schema = PGraphSchema::try_new(kinds, edges).unwrap();
        let products = BTreeSet::from([d(3)]);
        // Canonical OK.
        crate::axioms::AxiomBundle::new([d(0), d(1)], [])
            .validate(&schema, &products)
            .unwrap();
        // Extension OK.
        ExtensionAxiomBundle::new([d(0), d(1)])
            .validate(&schema, &products)
            .unwrap();
    }
}
