//! P-graph axiom checker — Friedler et al.\ 1992, **canonical
//! semantics** restored 2026-05-19.
//!
//! Prior versions of this module (the audit-flagged form) implemented
//! plausible-but-wrong paraphrases of three of the five axioms; the
//! 2026-05-19 audit (`reports/2026-05-19-pgraph-axiom-audit.md`)
//! documents exactly how the old `check_a{2,4,5}` drifted. The
//! current implementation is verbatim Friedler:
//!
//! - **A1 (S1).** Every required product is represented as an M-node
//!   in the schema.
//! - **A2 (S2).** For every M-node in the schema,
//!   *M has no ancestor* (i.e. no edge `u → m` with `u ∈ O`) ⟺
//!   *M is a raw material* (`m ∈ raws`).
//! - **A3 (S3).** Every O-node in the schema is in the master
//!   catalogue of operating units (when the catalogue is non-empty).
//! - **A4 (S4).** For every O-node in the schema there exists a
//!   directed path from the O-node to some required product.
//! - **A5 (S5).** Every M-node in the schema is incident to at least
//!   one edge (either as the source of an `m → u` edge or as the
//!   destination of a `u → m` edge).
//!
//! The internal `A1..A5` labels are kept (the user chose to fix the
//! semantics rather than rename the variants), but each variant now
//! carries the *canonical* violation payload, not the old paraphrase.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use hymeko::common::ids::DeclId;

use crate::schema::{PGraphSchema, PNodeKind};

/// One axiom check's outcome with its wall-clock duration.
#[derive(Debug, Clone)]
pub struct AxiomTrace {
    /// Short name (`"A1"` … `"A5"`).
    pub name: &'static str,
    /// Wall-clock time spent evaluating this axiom.
    pub duration: Duration,
    /// `None` if the axiom passed, `Some(violation)` if not.
    pub outcome: Option<AxiomViolation>,
}

/// One concrete violation of a P-graph axiom (canonical Friedler 1992
/// semantics; restored 2026-05-19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AxiomViolation {
    /// **A1 (S1).** One or more `required_products` are not present
    /// in the schema as M-nodes.
    MissingProducts {
        /// Required products that were not present.
        missing: Vec<DeclId>,
    },
    /// **A2 (S2).** The raw-material biconditional is violated.
    ///
    /// Both directions are reported in one variant so the consumer
    /// can see the full failure surface in a single check:
    ///
    /// - `non_raw_without_producer`: M-nodes that have no ancestor
    ///   in the schema yet are not in the raw set (the canonical
    ///   *forward* direction of S2). Such a material can never be
    ///   produced by the process.
    /// - `raw_with_producer`: M-nodes that are in the raw set yet
    ///   *are* produced by some O-node inside the schema (the
    ///   canonical *reverse* direction of S2 — a raw material must
    ///   not be produced inside the process; it must be an
    ///   interface to the outside world).
    RawMaterialDirectionFailures {
        /// M-nodes with no producer that are not declared raw.
        non_raw_without_producer: Vec<DeclId>,
        /// M-nodes declared raw that are produced by some O-node.
        raw_with_producer: Vec<DeclId>,
    },
    /// **A3 (S3).** O-nodes that are not in the master catalogue.
    InvalidUnits {
        /// O-nodes failing the unit-catalogue check.
        invalid: Vec<DeclId>,
    },
    /// **A4 (S4).** O-nodes with no directed path leading to any
    /// required product.
    UnitsWithoutPathToProduct {
        /// O-nodes from which no required product is reachable.
        offenders: Vec<DeclId>,
    },
    /// **A5 (S5).** M-nodes that are not incident to any edge —
    /// neither input nor output of any operating unit in the schema.
    IsolatedMaterials {
        /// Isolated M-nodes (no incoming or outgoing edge).
        offenders: Vec<DeclId>,
    },
}

/// Bundle of axiom checks applicable to a [`PGraphSchema`].
///
/// Constructed via [`AxiomBundle::default`] or [`AxiomBundle::new`];
/// the optional `valid_units` whitelist powers A3 (units in the
/// master catalogue). When the whitelist is empty A3 is disabled.
/// `raws` is consumed by both A2 (raw biconditional) and was
/// previously consumed by A5 — A5 no longer needs it under canonical
/// semantics.
#[derive(Debug, Default)]
pub struct AxiomBundle {
    /// **A3** master catalogue of valid operating units. If
    /// non-empty, every O-node must be in this set; if empty, A3 is
    /// skipped.
    pub valid_units: BTreeSet<DeclId>,
    /// **A2** declared raw materials. Used by the biconditional
    /// check: a node is raw if and only if it has no ancestor.
    pub raws: BTreeSet<DeclId>,
}

impl AxiomBundle {
    /// Convenience constructor — supply raws and the unit whitelist
    /// in one call. Pass empty sets to disable A3 / make A2 trivial.
    pub fn new(
        raws: impl IntoIterator<Item = DeclId>,
        valid_units: impl IntoIterator<Item = DeclId>,
    ) -> Self {
        Self {
            raws: raws.into_iter().collect(),
            valid_units: valid_units.into_iter().collect(),
        }
    }

    /// Run every axiom check, returning the
    /// (axiom-name, duration, outcome) triples in evaluation order.
    /// The outcome is `None` when the axiom passed and `Some(...)`
    /// when it didn't. All five axioms run unconditionally — the
    /// caller decides whether `Ok` or `Err` is the right summary.
    pub fn validate_timed(
        &self,
        schema: &PGraphSchema,
        required_products: &BTreeSet<DeclId>,
    ) -> Vec<AxiomTrace> {
        let producers = build_producers(schema);
        let adj_forward = build_forward_adj(schema);

        let mut out = Vec::with_capacity(5);

        let t0 = Instant::now();
        let v = check_a1(schema, required_products);
        out.push(AxiomTrace {
            name: "A1",
            duration: t0.elapsed(),
            outcome: v,
        });

        let t0 = Instant::now();
        let v = check_a2(schema, &producers, &self.raws);
        out.push(AxiomTrace {
            name: "A2",
            duration: t0.elapsed(),
            outcome: v,
        });

        let t0 = Instant::now();
        let v = check_a3(schema, &self.valid_units);
        out.push(AxiomTrace {
            name: "A3",
            duration: t0.elapsed(),
            outcome: v,
        });

        let t0 = Instant::now();
        let v = check_a4(schema, &adj_forward, required_products);
        out.push(AxiomTrace {
            name: "A4",
            duration: t0.elapsed(),
            outcome: v,
        });

        let t0 = Instant::now();
        let v = check_a5(schema);
        out.push(AxiomTrace {
            name: "A5",
            duration: t0.elapsed(),
            outcome: v,
        });

        out
    }

    /// Run every axiom check and return the list of violations
    /// encountered. An empty `Ok(())` means the schema satisfies all
    /// five canonical axioms.
    pub fn validate(
        &self,
        schema: &PGraphSchema,
        required_products: &BTreeSet<DeclId>,
    ) -> Result<(), Vec<AxiomViolation>> {
        let producers = build_producers(schema);
        let adj_forward = build_forward_adj(schema);
        let mut violations: Vec<AxiomViolation> = Vec::new();

        if let Some(v) = check_a1(schema, required_products) {
            violations.push(v);
        }
        if let Some(v) = check_a2(schema, &producers, &self.raws) {
            violations.push(v);
        }
        if let Some(v) = check_a3(schema, &self.valid_units) {
            violations.push(v);
        }
        if let Some(v) = check_a4(schema, &adj_forward, required_products) {
            violations.push(v);
        }
        if let Some(v) = check_a5(schema) {
            violations.push(v);
        }

        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }
}

// ─── Per-axiom checks (canonical Friedler semantics) ─────────────────

/// **A1 (S1).** Every required product is an M-node in the schema.
fn check_a1(schema: &PGraphSchema, required_products: &BTreeSet<DeclId>) -> Option<AxiomViolation> {
    let mut missing = Vec::new();
    for prod in required_products {
        match schema.kind(*prod) {
            Some(PNodeKind::Material) => {}
            _ => missing.push(*prod),
        }
    }
    if missing.is_empty() {
        None
    } else {
        Some(AxiomViolation::MissingProducts { missing })
    }
}

/// **A2 (S2).** `M has no ancestor ⟺ M is raw`. Both directions.
///
/// `producers[m]` is the set of O-nodes with an edge `u → m` —
/// pre-computed once per `validate()` call.
fn check_a2(
    schema: &PGraphSchema,
    producers: &BTreeMap<DeclId, BTreeSet<DeclId>>,
    raws: &BTreeSet<DeclId>,
) -> Option<AxiomViolation> {
    let mut non_raw_without_producer = Vec::new();
    let mut raw_with_producer = Vec::new();
    for m in schema.m_nodes() {
        let has_ancestor = producers.get(&m).is_some_and(|p| !p.is_empty());
        let is_raw = raws.contains(&m);
        // Forward direction (canonical): has_ancestor ⇒ is_raw is
        // FALSE — i.e. ¬has_ancestor ⇒ is_raw. Contrapositive: if
        // ¬is_raw then must has_ancestor. Violation: ¬is_raw ∧
        // ¬has_ancestor.
        if !is_raw && !has_ancestor {
            non_raw_without_producer.push(m);
        }
        // Reverse direction (canonical): is_raw ⇒ ¬has_ancestor.
        // Violation: is_raw ∧ has_ancestor.
        if is_raw && has_ancestor {
            raw_with_producer.push(m);
        }
    }
    if non_raw_without_producer.is_empty() && raw_with_producer.is_empty() {
        None
    } else {
        Some(AxiomViolation::RawMaterialDirectionFailures {
            non_raw_without_producer,
            raw_with_producer,
        })
    }
}

/// **A3 (S3).** Every O-node is in the master catalogue.
fn check_a3(schema: &PGraphSchema, valid_units: &BTreeSet<DeclId>) -> Option<AxiomViolation> {
    if valid_units.is_empty() {
        return None;
    }
    let mut invalid = Vec::new();
    for o in schema.o_nodes() {
        if !valid_units.contains(&o) {
            invalid.push(o);
        }
    }
    if invalid.is_empty() {
        None
    } else {
        Some(AxiomViolation::InvalidUnits { invalid })
    }
}

/// **A4 (S4).** From every O-node a required product is reachable
/// in the directed schema.
///
/// `adj` is the pre-computed forward adjacency `node → successors`.
/// With no required products the axiom is vacuous (no offender
/// possible).
fn check_a4(
    schema: &PGraphSchema,
    adj: &BTreeMap<DeclId, Vec<DeclId>>,
    required_products: &BTreeSet<DeclId>,
) -> Option<AxiomViolation> {
    if required_products.is_empty() {
        return None;
    }
    let mut offenders = Vec::new();
    for o in schema.o_nodes() {
        if !reaches_any(adj, o, required_products) {
            offenders.push(o);
        }
    }
    if offenders.is_empty() {
        None
    } else {
        Some(AxiomViolation::UnitsWithoutPathToProduct { offenders })
    }
}

/// **A5 (S5).** Every M-node is incident to ≥ 1 edge in the schema.
fn check_a5(schema: &PGraphSchema) -> Option<AxiomViolation> {
    let mut offenders = Vec::new();
    for m in schema.m_nodes() {
        if schema.in_degree(m) == 0 && schema.out_degree(m) == 0 {
            offenders.push(m);
        }
    }
    if offenders.is_empty() {
        None
    } else {
        Some(AxiomViolation::IsolatedMaterials { offenders })
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────

/// For each M-node `m`, record the set of O-nodes `u` such that the
/// edge `u → m` exists in the schema. M-nodes with no producer are
/// omitted from the map.
fn build_producers(schema: &PGraphSchema) -> BTreeMap<DeclId, BTreeSet<DeclId>> {
    let mut out: BTreeMap<DeclId, BTreeSet<DeclId>> = BTreeMap::new();
    for (_, src, dst) in schema.edges() {
        if let (Some(PNodeKind::OperatingUnit), Some(PNodeKind::Material)) =
            (schema.kind(src), schema.kind(dst))
        {
            out.entry(dst).or_default().insert(src);
        }
    }
    out
}

/// Build the forward adjacency map `src → [dst, ...]` of the schema.
fn build_forward_adj(schema: &PGraphSchema) -> BTreeMap<DeclId, Vec<DeclId>> {
    let mut adj: BTreeMap<DeclId, Vec<DeclId>> = BTreeMap::new();
    for (_, src, dst) in schema.edges() {
        adj.entry(src).or_default().push(dst);
    }
    adj
}

/// BFS in the pre-built adjacency: does `from` reach any node in
/// `targets`? `from` itself does not count.
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

    // ─── A1 ─────────────────────────────────────────────────────

    /// A1 caught: a required product is not in the schema.
    #[test]
    fn a1_catches_missing_product() {
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material),
            (d(1), PNodeKind::OperatingUnit),
            (d(2), PNodeKind::Material),
        ]);
        let edges = BTreeMap::from([(e(0), (d(0), d(1))), (e(1), (d(1), d(2)))]);
        let schema = PGraphSchema::try_new(kinds, edges).expect("bipartite OK");
        let bundle = AxiomBundle::new([d(0)], []);
        let products = BTreeSet::from([d(99)]); // not in the schema
        let v = bundle.validate(&schema, &products).expect_err("must err");
        assert!(v.iter().any(|x| matches!(
            x, AxiomViolation::MissingProducts { missing } if missing.contains(&d(99))
        )));
    }

    // ─── A2 (S2 biconditional, both directions) ────────────────

    /// A2 forward direction: a non-raw material with no producer is
    /// flagged. (The canonical "no-ancestor → raw" → contrapositive
    /// "¬raw → has-ancestor".)
    #[test]
    fn a2_forward_catches_non_raw_with_no_producer() {
        // d(0) is raw, d(1) is U, d(2) is product, d(3) is a non-raw
        // material with no producer at all.
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material),
            (d(1), PNodeKind::OperatingUnit),
            (d(2), PNodeKind::Material),
            (d(3), PNodeKind::Material),      // dangling, non-raw
            (d(4), PNodeKind::OperatingUnit), // touches d(3) so A5 is silent
        ]);
        let edges = BTreeMap::from([
            (e(0), (d(0), d(1))),
            (e(1), (d(1), d(2))),
            (e(2), (d(3), d(4))), // d(3) is consumed by U2 but never produced
            (e(3), (d(4), d(2))),
        ]);
        let schema = PGraphSchema::try_new(kinds, edges).expect("bipartite OK");
        let bundle = AxiomBundle::new([d(0)], []); // only d(0) is raw
        let products = BTreeSet::from([d(2)]);
        let v = bundle.validate(&schema, &products).expect_err("must err");
        assert!(v.iter().any(|x| matches!(
            x,
            AxiomViolation::RawMaterialDirectionFailures {
                non_raw_without_producer, ..
            } if non_raw_without_producer.contains(&d(3))
        )));
    }

    /// A2 reverse direction: a declared raw material that *is*
    /// produced inside the schema is flagged. (The canonical
    /// "raw → no-ancestor" direction.)
    #[test]
    fn a2_reverse_catches_raw_produced_inside() {
        // d(0) is declared raw BUT a unit produces it inside the
        // schema — Friedler's S2 says that's a violation: raws must
        // be an interface to the outside.
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material),      // declared raw
            (d(1), PNodeKind::OperatingUnit), // U1 produces d(0)
            (d(2), PNodeKind::Material),      // some intermediate
            (d(3), PNodeKind::OperatingUnit), // U2 consumes d(0), produces product
            (d(4), PNodeKind::Material),      // product
        ]);
        let edges = BTreeMap::from([
            (e(0), (d(2), d(1))),
            (e(1), (d(1), d(0))), // U1 → d(0): d(0) has an ancestor!
            (e(2), (d(0), d(3))),
            (e(3), (d(3), d(4))),
        ]);
        let schema = PGraphSchema::try_new(kinds, edges).expect("bipartite OK");
        let bundle = AxiomBundle::new([d(0), d(2)], []); // d(0) AND d(2) raw
        let products = BTreeSet::from([d(4)]);
        let v = bundle.validate(&schema, &products).expect_err("must err");
        assert!(v.iter().any(|x| matches!(
            x,
            AxiomViolation::RawMaterialDirectionFailures {
                raw_with_producer, ..
            } if raw_with_producer.contains(&d(0))
        )));
    }

    /// A2 passes on a well-formed P-graph where all non-raw materials
    /// have producers AND all raws have no producer.
    #[test]
    fn a2_passes_on_well_formed_pgraph() {
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material), // raw
            (d(1), PNodeKind::Material), // raw
            (d(2), PNodeKind::OperatingUnit),
            (d(3), PNodeKind::Material), // product (produced)
        ]);
        let edges = BTreeMap::from([
            (e(0), (d(0), d(2))),
            (e(1), (d(1), d(2))),
            (e(2), (d(2), d(3))),
        ]);
        let schema = PGraphSchema::try_new(kinds, edges).unwrap();
        let bundle = AxiomBundle::new([d(0), d(1)], []);
        let products = BTreeSet::from([d(3)]);
        bundle.validate(&schema, &products).unwrap();
    }

    // ─── A3 ─────────────────────────────────────────────────────

    /// A3 caught: an O-node not in the master catalogue.
    #[test]
    fn a3_catches_unwhitelisted_unit() {
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material),
            (d(1), PNodeKind::OperatingUnit), // in catalogue
            (d(2), PNodeKind::Material),
            (d(3), PNodeKind::OperatingUnit), // NOT in catalogue
        ]);
        let edges = BTreeMap::from([
            (e(0), (d(0), d(1))),
            (e(1), (d(1), d(2))),
            (e(2), (d(0), d(3))),
            (e(3), (d(3), d(2))),
        ]);
        let schema = PGraphSchema::try_new(kinds, edges).unwrap();
        let bundle = AxiomBundle::new([d(0)], [d(1)]);
        let products = BTreeSet::from([d(2)]);
        let v = bundle.validate(&schema, &products).expect_err("must err");
        assert!(v.iter().any(|x| matches!(
            x, AxiomViolation::InvalidUnits { invalid } if invalid.contains(&d(3))
        )));
    }

    // ─── A4 (S4: O-node has path to product) ──────────────────

    /// A4 caught: an O-node whose every output dead-ends in a
    /// non-product material. Old A4 (degree ≥ 1 in AND ≥ 1 out)
    /// missed this — that's why the rewrite matters.
    #[test]
    fn a4_catches_dead_branch_o_node() {
        // Live path: raw d(0) → U1 → product d(2). Dead branch: raw
        // d(3) → U2 → dead intermediate d(4) (a material, but NOT a
        // product and NOT consumed downstream). U2 has ≥ 1 input
        // (d(3)) and ≥ 1 output (d(4)) — the OLD A4 passes; canonical
        // S4 fails.
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material),      // raw
            (d(1), PNodeKind::OperatingUnit), // U1, live
            (d(2), PNodeKind::Material),      // product
            (d(3), PNodeKind::Material),      // raw of dead branch
            (d(4), PNodeKind::Material),      // dead intermediate
            (d(5), PNodeKind::OperatingUnit), // U2, dead
        ]);
        let edges = BTreeMap::from([
            (e(0), (d(0), d(1))),
            (e(1), (d(1), d(2))),
            (e(2), (d(3), d(5))),
            (e(3), (d(5), d(4))), // U2 outputs to d(4), which never reaches d(2)
        ]);
        let schema = PGraphSchema::try_new(kinds, edges).unwrap();
        let bundle = AxiomBundle::new([d(0), d(3)], []);
        let products = BTreeSet::from([d(2)]);
        let v = bundle.validate(&schema, &products).expect_err("must err");
        assert!(v.iter().any(|x| matches!(
            x,
            AxiomViolation::UnitsWithoutPathToProduct { offenders }
                if offenders.contains(&d(5))
        )));
    }

    /// A4 silent when every O-node reaches a product.
    #[test]
    fn a4_passes_when_every_o_reaches_product() {
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material),
            (d(1), PNodeKind::OperatingUnit),
            (d(2), PNodeKind::Material),
        ]);
        let edges = BTreeMap::from([(e(0), (d(0), d(1))), (e(1), (d(1), d(2)))]);
        let schema = PGraphSchema::try_new(kinds, edges).unwrap();
        let bundle = AxiomBundle::new([d(0)], []);
        let products = BTreeSet::from([d(2)]);
        bundle.validate(&schema, &products).unwrap();
    }

    // ─── A5 (S5: every M-node touches some unit) ──────────────

    /// A5 caught: a completely isolated M-node (no in, no out).
    /// Old A5 missed this — it only fired when the material was
    /// *consumed* and not produced.
    #[test]
    fn a5_catches_isolated_material() {
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material), // raw
            (d(1), PNodeKind::OperatingUnit),
            (d(2), PNodeKind::Material), // product
            (d(3), PNodeKind::Material), // ISOLATED — no edges at all
        ]);
        let edges = BTreeMap::from([(e(0), (d(0), d(1))), (e(1), (d(1), d(2)))]);
        let schema = PGraphSchema::try_new(kinds, edges).unwrap();
        let bundle = AxiomBundle::new([d(0), d(3)], []); // d(3) declared raw
        let products = BTreeSet::from([d(2)]);
        let v = bundle.validate(&schema, &products).expect_err("must err");
        assert!(v.iter().any(|x| matches!(
            x, AxiomViolation::IsolatedMaterials { offenders } if offenders.contains(&d(3))
        )));
    }

    /// A5 silent on a material that is only an output (still has
    /// ≥ 1 incident edge).
    #[test]
    fn a5_silent_on_unit_output_only_material() {
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material), // raw, consumed
            (d(1), PNodeKind::OperatingUnit),
            (d(2), PNodeKind::Material), // produced only
        ]);
        let edges = BTreeMap::from([(e(0), (d(0), d(1))), (e(1), (d(1), d(2)))]);
        let schema = PGraphSchema::try_new(kinds, edges).unwrap();
        let bundle = AxiomBundle::new([d(0)], []);
        let products = BTreeSet::from([d(2)]);
        bundle.validate(&schema, &products).unwrap();
    }

    // ─── Worked example (all five axioms pass) ───────────────

    /// Two raws → one unit → one product. Should satisfy A1..A5.
    #[test]
    fn worked_example_passes_all_axioms() {
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
        let schema = PGraphSchema::try_new(kinds, edges).expect("bipartite OK");
        let bundle = AxiomBundle::new([d(0), d(1)], []);
        let products = BTreeSet::from([d(3)]);
        bundle
            .validate(&schema, &products)
            .expect("worked example satisfies S1..S5");
    }

    // ─── Cross-axiom: a schema that fails multiple at once ───

    /// A schema constructed to fail A2, A4, AND A5 simultaneously,
    /// proving every check runs independently and reports its own
    /// finding.
    #[test]
    fn multi_axiom_failure_surfaces_each_violation() {
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material),      // raw
            (d(1), PNodeKind::OperatingUnit), // live U1
            (d(2), PNodeKind::Material),      // product
            (d(3), PNodeKind::Material),      // ISOLATED, not raw → A5 + A2 forward
            (d(4), PNodeKind::OperatingUnit), // dead U2 → A4
            (d(5), PNodeKind::Material),      // dead branch output
            (d(6), PNodeKind::Material),      // raw of dead branch
        ]);
        let edges = BTreeMap::from([
            (e(0), (d(0), d(1))),
            (e(1), (d(1), d(2))),
            (e(2), (d(6), d(4))),
            (e(3), (d(4), d(5))),
        ]);
        let schema = PGraphSchema::try_new(kinds, edges).unwrap();
        let bundle = AxiomBundle::new([d(0), d(6)], []); // d(3) NOT raw
        let products = BTreeSet::from([d(2)]);
        let v = bundle.validate(&schema, &products).expect_err("must err");
        // A2 forward: d(3) is non-raw and has no producer (also no
        // incident edge, but A2 doesn't care about A5's premise).
        assert!(v.iter().any(|x| matches!(
            x,
            AxiomViolation::RawMaterialDirectionFailures {
                non_raw_without_producer, ..
            } if non_raw_without_producer.contains(&d(3))
        )));
        // A4: U2 (d(4)) does not reach product d(2).
        assert!(v.iter().any(|x| matches!(
            x,
            AxiomViolation::UnitsWithoutPathToProduct { offenders }
                if offenders.contains(&d(4))
        )));
        // A5: d(3) is isolated.
        assert!(v.iter().any(|x| matches!(
            x,
            AxiomViolation::IsolatedMaterials { offenders }
                if offenders.contains(&d(3))
        )));
    }

    // ─── Timed entry point still surfaces all five names ────

    #[test]
    fn validate_timed_returns_five_traces_in_order() {
        let kinds = BTreeMap::from([
            (d(0), PNodeKind::Material),
            (d(1), PNodeKind::OperatingUnit),
            (d(2), PNodeKind::Material),
        ]);
        let edges = BTreeMap::from([(e(0), (d(0), d(1))), (e(1), (d(1), d(2)))]);
        let schema = PGraphSchema::try_new(kinds, edges).unwrap();
        let bundle = AxiomBundle::new([d(0)], []);
        let products = BTreeSet::from([d(2)]);
        let traces = bundle.validate_timed(&schema, &products);
        assert_eq!(traces.len(), 5);
        let names: Vec<&str> = traces.iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["A1", "A2", "A3", "A4", "A5"]);
        for t in &traces {
            assert!(t.outcome.is_none(), "{} unexpected violation", t.name);
        }
    }
}
