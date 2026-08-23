//! Axiom witness — concrete diagnostic of what canonical Friedler 1992
//! S1..S5 vs. the orthogonal extension bundle report on every shipped
//! P-graph fixture, and on the MSG/ABB engine output.
//!
//! Pulls the previously hidden divergence into a runnable, asserted
//! form. The exercise revealed a real semantic fact about the
//! published textbook examples and the engine:
//!
//! 1. **Chapter 6** is the only textbook fixture that is *fully*
//!    canonical-feasible as-shipped. It is the "clean" example.
//! 2. **HDA** (toluene → benzene with disposal sink) is **not**
//!    canonical-feasible: the `@Disposal` unit consumes Methane and
//!    produces nothing, so it violates **canonical S4** ("every
//!    O-node has a path to a product"). The engine selects this
//!    unit anyway because *strict no-excess feasibility* requires
//!    a consumer for the Methane by-product. Both readings are
//!    internally consistent; the divergence is real.
//! 3. **Chapter 4 examples 1 and 3** are "messy" textbook fixtures
//!    that include intentionally-unfeasible parts (orphan
//!    intermediates, raws that are also produced inside, dead unit
//!    branches) so MSG/SSG/ABB have something to prune. After MSG
//!    prunes, the surviving subgraph passes canonical S1..S5.
//!
//! Run with `cargo test --test axiom_witness -- --nocapture` to see
//! the per-fixture diagnostic table.

use std::collections::{BTreeMap, BTreeSet};

use hymeko::common::ids::{DeclId, EdgeId};
use hymeko_pgraph::dump::{DumpAlgorithm, analyze_source_with_full_options};
use hymeko_pgraph::msg::MaximalStructureOptions;
use hymeko_pgraph::{
    AbbOptions, AxiomBundle, AxiomViolation, ExtensionAxiomBundle, ExtensionAxiomViolation,
    LoweredPGraph, PGraphSchema, PNodeKind, abb_solve, lower, maximal_structure,
};
use parser::parse_description;

const HDA_SRC: &str = include_str!("../../data/pgraph/hda.hymeko");
const CHAPTER4_1: &str = include_str!("../../data/pgraph/Chapter4/example4_1.hymeko");
const CHAPTER4_3: &str = include_str!("../../data/pgraph/Chapter4/example4_3.hymeko");
const CHAPTER6_1: &str = include_str!("../../data/pgraph/Chapter6/example6_1.hymeko");
// HSIKAN + Gömb architecture-search P-graphs used by the
// `run_gomb_msg_sweep` driver. Their canonical-feasibility is the
// "gain" question — whether the audit affects what those NAS runs
// see as feasible.
const HSIKAN_SWEEP: &str = include_str!("../../data/hsikan/sweep_msg.hymeko");
const GOMB_SWEEP: &str = include_str!("../../data/hsikan/sweep_msg_gomb.hymeko");
// Phase 6: by-product-injected HSIKAN sweep — exercises the
// strict-vs-relaxed divergence on architecture selection.
const HSIKAN_BYPRODUCT: &str = include_str!("../../data/hsikan/sweep_msg_byproduct.hymeko");

// ─── Helpers ────────────────────────────────────────────────────────

fn parse_and_lower(src: &str) -> LoweredPGraph {
    let d = parse_description(src).expect("parser must accept");
    lower(&d).expect("lowering must succeed")
}

fn validate_canonical(p: &LoweredPGraph) -> Result<(), Vec<AxiomViolation>> {
    AxiomBundle::new(p.raws.iter().copied(), []).validate(&p.schema, &p.products)
}

fn validate_extension(p: &LoweredPGraph) -> Result<(), Vec<ExtensionAxiomViolation>> {
    ExtensionAxiomBundle::new(p.raws.iter().copied()).validate(&p.schema, &p.products)
}

fn project_schema(p: &LoweredPGraph, units: &BTreeSet<DeclId>) -> PGraphSchema {
    let mut kinds: BTreeMap<DeclId, PNodeKind> = BTreeMap::new();
    let mut materials: BTreeSet<DeclId> = BTreeSet::new();
    for u in units {
        kinds.insert(*u, PNodeKind::OperatingUnit);
        materials.extend(p.inputs(*u).iter().copied());
        materials.extend(p.outputs(*u).iter().copied());
    }
    materials.extend(p.products.iter().copied());
    for m in &materials {
        kinds.insert(*m, PNodeKind::Material);
    }
    let mut edges: BTreeMap<EdgeId, (DeclId, DeclId)> = BTreeMap::new();
    let mut next_eid = 0usize;
    for (_, src, dst) in p.schema.edges() {
        let keep = match (p.schema.kind(src), p.schema.kind(dst)) {
            (Some(PNodeKind::Material), Some(PNodeKind::OperatingUnit)) => {
                materials.contains(&src) && units.contains(&dst)
            }
            (Some(PNodeKind::OperatingUnit), Some(PNodeKind::Material)) => {
                units.contains(&src) && materials.contains(&dst)
            }
            _ => false,
        };
        if keep {
            edges.insert(EdgeId::new(next_eid), (src, dst));
            next_eid += 1;
        }
    }
    PGraphSchema::try_new(kinds, edges).expect("projected schema must be bipartite")
}

fn validate_subschema_canonical(
    sub: &PGraphSchema,
    p: &LoweredPGraph,
) -> Result<(), Vec<AxiomViolation>> {
    // Restrict raws to those still present in the projected schema —
    // an unused raw from the original LoweredPGraph doesn't appear in
    // the projection and would otherwise be missing for the
    // S2-reverse check.
    let raws_in_proj: BTreeSet<DeclId> = p
        .raws
        .iter()
        .copied()
        .filter(|r| sub.kind(*r).is_some())
        .collect();
    AxiomBundle::new(raws_in_proj.iter().copied(), []).validate(sub, &p.products)
}

// ─── Per-fixture: chapter 6 is the clean baseline ───────────────────

#[test]
fn chapter6_full_schema_passes_canonical() {
    // Chapter 6 example 1 is the cleanest textbook fixture — it
    // passes canonical S1..S5 as shipped. The extension bundle
    // fires E-NoExcess on material B (a by-product of @O2 that no
    // unit consumes); MSG correctly prunes the producer of B and
    // the engine output passes BOTH bundles (see next test).
    let p = parse_and_lower(CHAPTER6_1);
    validate_canonical(&p).expect("Chapter 6: full schema must satisfy canonical S1..S5");
    // Extension catches the by-product B.
    let ext = validate_extension(&p)
        .expect_err("Chapter 6 full schema has a by-product → extension must flag E-NoExcess");
    assert!(
        ext.iter()
            .any(|x| matches!(x, ExtensionAxiomViolation::NonReachingMaterials { .. }))
    );
}

#[test]
fn chapter6_engine_output_satisfies_canonical() {
    let p = parse_and_lower(CHAPTER6_1);
    let msg = maximal_structure(&p);
    let sol = abb_solve(&p, &msg).expect("Chapter 6 must have an ABB solution");
    // Canonical optimum is {O2,O5,O7} at 9.0 (the buggy strict default
    // previously returned {O1,O3,O6} at 18.0 from a too-small MSG).
    assert!(
        (sol.cost - 9.0).abs() < 1e-9,
        "Chapter 6 canonical cost optimum (got {})",
        sol.cost
    );
    let proj = project_schema(&p, &sol.units);
    validate_subschema_canonical(&proj, &p)
        .expect("Chapter 6 ABB output must satisfy canonical S1..S5");
    // Note: canonical ABB does NOT enforce the no-excess extension bundle
    // (no-excess is not an axiom); a cost-optimal structure may vent a
    // by-product. The extension bundle is exercised by the strict-mode
    // tests and `byproduct_filter_phase11`.
}

// ─── HDA: canonical-S4 fails on Disposal sink (real divergence) ─────

#[test]
fn hda_full_schema_violates_canonical_s4_on_disposal_sink() {
    // The HDA fixture contains an @Disposal unit that consumes
    // Methane and produces nothing. Canonical S4 demands every
    // O-node have a directed path to a product; Disposal cannot.
    // This is the headline divergence: the textbook fixture is
    // engineering-feasible but canonical-S4-infeasible.
    let p = parse_and_lower(HDA_SRC);
    let v = validate_canonical(&p).expect_err("HDA must fail canonical S4");
    let disposal_id = p.name_to_decl["Disposal"];
    let s4_hit = v.iter().any(|x| {
        matches!(
            x, AxiomViolation::UnitsWithoutPathToProduct { offenders }
                if offenders.contains(&disposal_id)
        )
    });
    assert!(s4_hit, "the canonical violation must name @Disposal: {v:?}");
}

#[test]
fn hda_engine_output_excludes_disposal_and_passes_canonical_s4() {
    // Post-2026-05-27: the canonical MSG excludes the Disposal sink (it
    // reaches no product, axiom S4), so it is not available to ABB in any
    // mode. The default (canonical) optimum {Mixer,Reactor} therefore
    // satisfies canonical S4. Even the strict no-waste opt-in can no
    // longer pull Disposal in (it is not in the maximal structure): the
    // old strict-vs-canonical S4 divergence on disposal sinks is gone.
    let p = parse_and_lower(HDA_SRC);
    let msg = maximal_structure(&p);
    let disposal_id = p.name_to_decl["Disposal"];
    assert!(
        !msg.units.contains(&disposal_id),
        "Disposal must be pruned by canonical MSG"
    );

    let sol = abb_solve(&p, &msg).expect("HDA must have an ABB solution under default opts");
    let proj = project_schema(&p, &sol.units);
    validate_subschema_canonical(&proj, &p)
        .expect("canonical HDA ABB output must satisfy S1..S5 (no Disposal sink)");

    // Strict no-waste opt-in: also cannot include Disposal (not in MSG),
    // so its output passes canonical S4 too.
    let strict = hymeko_pgraph::abb::solve_with_options(
        &p,
        &msg,
        hymeko_pgraph::abb::AbbOptions {
            strict_no_excess: true,
            ..Default::default()
        },
    )
    .expect("strict HDA ABB must find the no-waste route");
    assert!(!strict.units.contains(&disposal_id));
}

#[test]
fn hda_engine_output_under_relaxed_no_excess_drops_disposal_and_passes_canonical() {
    // With strict_no_excess turned OFF, ABB picks the cheapest
    // feasible cover that no longer requires Disposal (the
    // by-product Methane is vented instead of consumed). The
    // surviving units now all satisfy canonical S4.
    let p = parse_and_lower(HDA_SRC);
    let msg = hymeko_pgraph::msg::maximal_structure_with_options(
        &p,
        hymeko_pgraph::msg::MaximalStructureOptions {
            strict_no_excess: false,
        },
    );
    let opts = AbbOptions {
        strict_no_excess: false,
        ..AbbOptions::default()
    };
    let sol = hymeko_pgraph::abb::solve_with_options(&p, &msg, opts)
        .expect("HDA relaxed must be feasible");
    let proj = project_schema(&p, &sol.units);
    // Disposal must not be in the selection.
    let disposal_id = p.name_to_decl["Disposal"];
    assert!(
        !sol.units.contains(&disposal_id),
        "relaxed ABB must drop Disposal under strict_no_excess=false"
    );
    validate_subschema_canonical(&proj, &p)
        .expect("HDA relaxed ABB output must satisfy canonical S1..S5");
}

// ─── Chapter 4: messy textbook fixtures — MSG prunes to canonical-feasible ─

#[test]
fn chapter4_1_full_schema_is_intentionally_messy() {
    // The fixture comments name it as "materials=18, units=11" but
    // includes orphan intermediates (K, L, N, Q, V have no producer)
    // and a raw (H) that is also produced inside the schema. These
    // are MSG-pruning targets, not bugs.
    let p = parse_and_lower(CHAPTER4_1);
    let v = validate_canonical(&p).expect_err("Chapter 4-1 must surface canonical violations");
    // Expect at least one of each: S2-forward, S2-reverse, S4, S5.
    let has_s2_fwd = v.iter().any(|x| {
        matches!(
            x, AxiomViolation::RawMaterialDirectionFailures { non_raw_without_producer, .. }
                if !non_raw_without_producer.is_empty()
        )
    });
    let has_s2_rev = v.iter().any(|x| {
        matches!(
            x, AxiomViolation::RawMaterialDirectionFailures { raw_with_producer, .. }
                if !raw_with_producer.is_empty()
        )
    });
    let has_s4 = v
        .iter()
        .any(|x| matches!(x, AxiomViolation::UnitsWithoutPathToProduct { .. }));
    let has_s5 = v
        .iter()
        .any(|x| matches!(x, AxiomViolation::IsolatedMaterials { .. }));
    assert!(
        has_s2_fwd && has_s2_rev && has_s4 && has_s5,
        "chapter 4-1 surfaces all four canonical violation kinds: {v:?}"
    );
}

#[test]
fn chapter4_1_engine_output_satisfies_canonical_after_msg_prune() {
    let p = parse_and_lower(CHAPTER4_1);
    let msg = maximal_structure(&p);
    let sol = abb_solve(&p, &msg).expect("Chapter 4-1 ABB must succeed");
    let proj = project_schema(&p, &sol.units);
    validate_subschema_canonical(&proj, &p)
        .expect("Chapter 4-1 ABB output must satisfy canonical after MSG prune");
}

#[test]
fn chapter4_3_canonical_msg_keeps_29_units() {
    // Book Example 3.3 (Fig. 4.13): a non-degenerate maximal structure.
    // The canonical MSG keeps 29 of the 35 declared units and ABB finds
    // a feasible (here zero-cost — the .pgip carries no costs) optimum.
    // Pre-2026-05-27 the buggy strict default wrongly collapsed this to
    // 0 units and ABB returned None.
    let p = parse_and_lower(CHAPTER4_3);
    let msg = maximal_structure(&p);
    assert_eq!(
        msg.units.len(),
        29,
        "Chapter 4-3 canonical MSG must keep 29 units"
    );
    assert!(
        abb_solve(&p, &msg).is_some(),
        "ABB on the non-degenerate MSG must find a solution"
    );
}

#[test]
fn chapter4_3_engine_output_under_relaxed_satisfies_canonical() {
    // With relaxed no-excess, MSG keeps units (existing
    // `chapter4_3_strict_collapses_relaxed_does_not` test).
    // Verify the relaxed-mode engine output passes canonical
    // S1..S5.
    let p = parse_and_lower(CHAPTER4_3);
    let msg = hymeko_pgraph::msg::maximal_structure_with_options(
        &p,
        hymeko_pgraph::msg::MaximalStructureOptions {
            strict_no_excess: false,
        },
    );
    let opts = AbbOptions {
        strict_no_excess: false,
        ..AbbOptions::default()
    };
    let Some(sol) = hymeko_pgraph::abb::solve_with_options(&p, &msg, opts) else {
        // If even relaxed mode is infeasible, just document that.
        eprintln!("Chapter 4-3: even relaxed-mode is infeasible — no engine output to verify");
        return;
    };
    let proj = project_schema(&p, &sol.units);
    validate_subschema_canonical(&proj, &p)
        .expect("Chapter 4-3 relaxed ABB output must satisfy canonical S1..S5");
}

// ─── Synthetic divergence witnesses (no-fixture-dependence) ─────────

#[test]
fn synthetic_byproduct_canonical_passes_extension_fails() {
    // raw R -> U produces both product P and by-product B.
    // Canonical S1..S5 all pass (B is incident to U via U->B);
    // extension E-StrictNoExcess fires (B has no path to P).
    fn d(i: usize) -> DeclId {
        DeclId::new(i)
    }
    fn e(i: usize) -> EdgeId {
        EdgeId::new(i)
    }
    let kinds = BTreeMap::from([
        (d(0), PNodeKind::Material),
        (d(1), PNodeKind::OperatingUnit),
        (d(2), PNodeKind::Material),
        (d(3), PNodeKind::Material),
    ]);
    let edges = BTreeMap::from([
        (e(0), (d(0), d(1))),
        (e(1), (d(1), d(2))),
        (e(2), (d(1), d(3))),
    ]);
    let schema = PGraphSchema::try_new(kinds, edges).unwrap();
    let products = BTreeSet::from([d(2)]);
    AxiomBundle::new([d(0)], [])
        .validate(&schema, &products)
        .expect("canonical accepts a by-product schema");
    let err = ExtensionAxiomBundle::new([d(0)])
        .validate(&schema, &products)
        .expect_err("extension must catch the by-product");
    assert!(err.iter().any(|v| matches!(
        v, ExtensionAxiomViolation::NonReachingMaterials { offenders }
            if offenders.contains(&d(3))
    )));
}

#[test]
fn synthetic_source_unit_canonical_passes_extension_fails() {
    // U has zero inputs but produces P. Canonical S4 satisfied;
    // extension E-UnitWellFormed fires (in-degree 0).
    fn d(i: usize) -> DeclId {
        DeclId::new(i)
    }
    fn e(i: usize) -> EdgeId {
        EdgeId::new(i)
    }
    let kinds = BTreeMap::from([
        (d(1), PNodeKind::OperatingUnit),
        (d(2), PNodeKind::Material),
    ]);
    let edges = BTreeMap::from([(e(0), (d(1), d(2)))]);
    let schema = PGraphSchema::try_new(kinds, edges).unwrap();
    let products = BTreeSet::from([d(2)]);
    AxiomBundle::new([], [])
        .validate(&schema, &products)
        .expect("canonical accepts a source unit");
    let err = ExtensionAxiomBundle::new([])
        .validate(&schema, &products)
        .expect_err("extension must flag zero-in-degree unit");
    assert!(err.iter().any(|v| matches!(
        v, ExtensionAxiomViolation::UnitsWithDegreeZero { offenders }
            if offenders.contains(&d(1))
    )));
}

// ─── Engine minimality verification ─────────────────────────────────
//
// "Minimal" has two precise meanings the engine must satisfy:
//
//   1. MSG-minimality: the set of operating units returned by
//      `maximal_structure` is exactly the set of units that appear
//      in SOME combinatorially-feasible solution structure. That is,
//      `msg.units == ⋃ s.units` taken over every `s ∈ ssg_enumerate`.
//      Equivalently: every unit dropped by MSG cannot appear in any
//      feasible solution; every unit kept can appear in at least one.
//
//   2. ABB-optimality: the solution returned by `abb_solve` has cost
//      ≤ every solution structure enumerated by SSG.

fn brute_force_minimum_cost(p: &LoweredPGraph) -> Option<(BTreeSet<DeclId>, f64)> {
    let msg = maximal_structure(p);
    let ssg = hymeko_pgraph::ssg_enumerate(p, &msg);
    let mut best: Option<(BTreeSet<DeclId>, f64)> = None;
    for s in ssg {
        let cost: f64 = s
            .units
            .iter()
            .map(|u| p.costs.get(u).copied().unwrap_or(1.0))
            .sum();
        match &best {
            None => best = Some((s.units, cost)),
            Some((_, bc)) if cost < *bc => best = Some((s.units, cost)),
            _ => {}
        }
    }
    best
}

fn assert_minimality(p: &LoweredPGraph, fixture_name: &str) {
    let msg = maximal_structure(p);
    // (1) MSG-minimality: the maximal structure IS the union of all
    //     (structural) solution-structures — book Definition 3.3. Use the
    //     decision-mapping SSG, which enumerates structural solution-
    //     structures (axioms S1..S5, cycles admitted). The brute SSG's
    //     bootstrap-from-raws feasibility would wrongly exclude valid
    //     cyclic substructures and under-count the union.
    let dm = hymeko_pgraph::ssg_dm_enumerate(p, &msg);
    let mut ssg_union: BTreeSet<DeclId> = BTreeSet::new();
    for s in &dm {
        ssg_union.extend(s.units.iter().copied());
    }
    assert_eq!(
        msg.units, ssg_union,
        "{fixture_name}: MSG-minimality violated — \
         msg.units must equal the union of all solution-structure unit sets"
    );

    // (2) ABB-optimality: ABB matches brute-force SSG minimum.
    let abb = abb_solve(p, &msg);
    let brute = brute_force_minimum_cost(p);
    match (&abb, &brute) {
        (Some(a), Some((_, bc))) => {
            assert!(
                (a.cost - bc).abs() < 1e-9,
                "{fixture_name}: ABB-optimality violated — \
                 ABB cost = {} but brute-force min = {}",
                a.cost,
                bc
            );
        }
        (None, None) => {
            // Both agree: infeasible.
        }
        _ => panic!(
            "{fixture_name}: ABB/SSG disagree on feasibility (abb={:?}, brute={:?})",
            abb.is_some(),
            brute.is_some()
        ),
    }
}

#[test]
fn engine_is_minimal_on_chapter6() {
    let p = parse_and_lower(CHAPTER6_1);
    assert_minimality(&p, "Chapter6/ex1");
}

#[test]
fn engine_is_minimal_on_hda_strict() {
    let p = parse_and_lower(HDA_SRC);
    assert_minimality(&p, "HDA");
}

#[test]
fn engine_is_minimal_on_chapter4_1() {
    let p = parse_and_lower(CHAPTER4_1);
    assert_minimality(&p, "Chapter4/ex1");
}

#[test]
fn engine_is_minimal_on_hsikan_architecture_sweep() {
    let p = parse_and_lower(HSIKAN_SWEEP);
    assert_minimality(&p, "HSIKAN/sweep_msg");
}

#[test]
fn engine_is_minimal_on_gomb_architecture_sweep() {
    let p = parse_and_lower(GOMB_SWEEP);
    assert_minimality(&p, "Gomb/sweep_msg");
}

// ─── Canonical-feasibility certificate for HSIKAN/Gömb sweeps ───────

#[test]
fn hsikan_sweep_full_schema_is_canonical_feasible() {
    // The HSIKAN architecture-search P-graph (sweep_msg.hymeko) has:
    //   - raws: gpu_memory, train_time
    //   - intermediates: cycle_quality, embedding_quality
    //   - product: auc_score
    //   - units: 3 cycle-topk × 3 model-h × 2 train-length = 8 units
    // Every non-raw is produced; no raw is produced inside; every
    // unit has a path to auc_score; every material is incident.
    // Therefore the SEARCH SPACE the engine traverses is already
    // canonical S1..S5 compliant.
    let p = parse_and_lower(HSIKAN_SWEEP);
    validate_canonical(&p).expect("HSIKAN sweep_msg.hymeko must satisfy canonical S1..S5");
    validate_extension(&p)
        .expect("HSIKAN sweep_msg.hymeko must satisfy the extension bundle (no by-products)");
}

#[test]
fn hsikan_sweep_engine_output_is_canonical_feasible() {
    // ABB's selected architecture on the HSIKAN sweep must be a
    // canonical-feasible subgraph of the canonical-feasible full
    // schema.
    let p = parse_and_lower(HSIKAN_SWEEP);
    let msg = maximal_structure(&p);
    let sol = abb_solve(&p, &msg).expect("HSIKAN sweep must produce an ABB solution");
    let proj = project_schema(&p, &sol.units);
    validate_subschema_canonical(&proj, &p)
        .expect("HSIKAN architecture-search engine output must satisfy canonical S1..S5");
    let raws_in_proj: BTreeSet<DeclId> = p
        .raws
        .iter()
        .copied()
        .filter(|r| proj.kind(*r).is_some())
        .collect();
    ExtensionAxiomBundle::new(raws_in_proj.iter().copied())
        .validate(&proj, &p.products)
        .expect("HSIKAN engine output must satisfy the extension bundle");
}

#[test]
fn gomb_sweep_full_schema_is_canonical_feasible() {
    let p = parse_and_lower(GOMB_SWEEP);
    validate_canonical(&p).expect("Gömb sweep_msg_gomb.hymeko must satisfy canonical S1..S5");
    validate_extension(&p).expect("Gömb sweep_msg_gomb.hymeko must satisfy the extension bundle");
}

#[test]
fn gomb_sweep_engine_output_is_canonical_feasible() {
    let p = parse_and_lower(GOMB_SWEEP);
    let msg = maximal_structure(&p);
    let sol = abb_solve(&p, &msg).expect("Gömb sweep must produce an ABB solution");
    let proj = project_schema(&p, &sol.units);
    validate_subschema_canonical(&proj, &p)
        .expect("Gömb architecture-search engine output must satisfy canonical S1..S5");
    let raws_in_proj: BTreeSet<DeclId> = p
        .raws
        .iter()
        .copied()
        .filter(|r| proj.kind(*r).is_some())
        .collect();
    ExtensionAxiomBundle::new(raws_in_proj.iter().copied())
        .validate(&proj, &p.products)
        .expect("Gömb engine output must satisfy the extension bundle");
}

// ─── Diagnostic dump (printed under `--nocapture`) ──────────────────

#[test]
fn diagnostic_dump_canonical_vs_extension_vs_engine() {
    use std::fmt::Write;
    let mut report = String::new();
    for (name, src) in [
        ("HDA", HDA_SRC),
        ("Chapter4/ex1", CHAPTER4_1),
        ("Chapter4/ex3", CHAPTER4_3),
        ("Chapter6/ex1", CHAPTER6_1),
        ("HSIKAN/sweep_msg", HSIKAN_SWEEP),
        ("Gomb/sweep_msg", GOMB_SWEEP),
    ] {
        let p = parse_and_lower(src);
        let canon_full = validate_canonical(&p);
        let ext_full = validate_extension(&p);
        let msg = maximal_structure(&p);
        let abb = abb_solve(&p, &msg);
        let (canon_eng, ext_eng) = match &abb {
            Some(sol) => {
                let proj = project_schema(&p, &sol.units);
                let raws_in_proj: BTreeSet<DeclId> = p
                    .raws
                    .iter()
                    .copied()
                    .filter(|r| proj.kind(*r).is_some())
                    .collect();
                let canon =
                    AxiomBundle::new(raws_in_proj.iter().copied(), []).validate(&proj, &p.products);
                let ext = ExtensionAxiomBundle::new(raws_in_proj.iter().copied())
                    .validate(&proj, &p.products);
                (canon, ext)
            }
            None => (Ok(()), Ok(())),
        };
        writeln!(report, "\n[{name}]").unwrap();
        writeln!(
            report,
            "  full schema    canonical = {}",
            outcome_brief(&canon_full)
        )
        .unwrap();
        writeln!(
            report,
            "  full schema    extension = {}",
            outcome_brief_ext(&ext_full)
        )
        .unwrap();
        writeln!(report, "  MSG units = {}", msg.units.len()).unwrap();
        if let Some(s) = &abb {
            writeln!(report, "  ABB units = {} (cost {})", s.units.len(), s.cost).unwrap();
            writeln!(
                report,
                "  engine output  canonical = {}",
                outcome_brief(&canon_eng)
            )
            .unwrap();
            writeln!(
                report,
                "  engine output  extension = {}",
                outcome_brief_ext(&ext_eng)
            )
            .unwrap();
        } else {
            writeln!(report, "  ABB = NONE (infeasible)").unwrap();
        }
    }
    println!("{report}");
}

fn outcome_brief(r: &Result<(), Vec<AxiomViolation>>) -> String {
    match r {
        Ok(()) => "PASS".to_string(),
        Err(v) => {
            let mut tags: Vec<&'static str> = Vec::new();
            for it in v {
                tags.push(match it {
                    AxiomViolation::MissingProducts { .. } => "S1",
                    AxiomViolation::RawMaterialDirectionFailures { .. } => "S2",
                    AxiomViolation::InvalidUnits { .. } => "S3",
                    AxiomViolation::UnitsWithoutPathToProduct { .. } => "S4",
                    AxiomViolation::IsolatedMaterials { .. } => "S5",
                });
            }
            format!("FAIL [{}]", tags.join(","))
        }
    }
}

// ─── Phase 7: dump JSON carries the Friedler certificate ────────────

#[test]
fn dump_dto_strict_mode_byproduct_emits_canonical_pass_extension_fail() {
    // Phase 7 added canonical / extension certificate fields to
    // `PgraphAnalysisJson`. This test pins their behaviour on the
    // by-product fixture under the default (strict) mode.
    let analysis = analyze_source_with_full_options(
        HSIKAN_BYPRODUCT,
        DumpAlgorithm::Abb,
        MaximalStructureOptions {
            strict_no_excess: true,
        },
        AbbOptions::default(),
    );
    assert!(analysis.ok);
    // Full schema: canonical PASS (the by-product has an incident
    // edge, so S5 still holds); extension E-NoExcess FAIL.
    assert_eq!(analysis.canonical_full.status, "PASS");
    assert_eq!(analysis.extension_full.status, "FAIL");
    assert!(
        analysis
            .extension_full
            .violation_tags
            .contains(&"E-NoExcess".to_string())
    );
    // ABB sub-schema: canonical PASS (Disposal-style violation
    // doesn't apply — there's no sink unit) and extension PASS
    // (strict mode dropped the producer of the by-product).
    let cabb = analysis
        .canonical_abb_subschema
        .as_ref()
        .expect("ABB subschema cert must be present in abb mode");
    assert_eq!(cabb.status, "PASS");
    let eabb = analysis.extension_abb_subschema.as_ref().unwrap();
    assert_eq!(
        eabb.status, "PASS",
        "strict mode must drop the by-product producer; extension must accept"
    );
    // strict_no_excess echo field.
    assert!(analysis.strict_no_excess);
}

#[test]
fn dump_dto_relaxed_mode_byproduct_keeps_byproduct_extension_fails_on_subschema() {
    // Under relaxed mode the cheap cycle_topk_m4 survives → the
    // by-product material is part of the ABB sub-schema → extension
    // E-NoExcess fires on the sub-schema (not just the full schema).
    let analysis = analyze_source_with_full_options(
        HSIKAN_BYPRODUCT,
        DumpAlgorithm::Abb,
        MaximalStructureOptions {
            strict_no_excess: false,
        },
        AbbOptions {
            strict_no_excess: false,
            ..AbbOptions::default()
        },
    );
    assert!(analysis.ok);
    assert_eq!(analysis.canonical_full.status, "PASS");
    assert_eq!(analysis.extension_full.status, "FAIL");
    let cabb = analysis.canonical_abb_subschema.as_ref().unwrap();
    let eabb = analysis.extension_abb_subschema.as_ref().unwrap();
    assert_eq!(
        cabb.status, "PASS",
        "canonical S1..S5 still accepts the relaxed selection"
    );
    assert_eq!(
        eabb.status, "FAIL",
        "extension E-NoExcess must fire on the relaxed sub-schema"
    );
    assert!(!analysis.strict_no_excess);
}

#[test]
fn dump_dto_phase10_multicost_fields_echo() {
    // Phase 10 added cost_dimensions / cost_weights_echo /
    // abb_cost_breakdown to PgraphAnalysisJson. This test pins
    // their behaviour on the new HSIKAN multi-cost fixture.
    const MULTICOST: &str = include_str!("../../data/hsikan/sweep_msg_multicost.hymeko");
    // Scalar fallback: no weights supplied → echo is None.
    let scalar = analyze_source_with_full_options(
        MULTICOST,
        DumpAlgorithm::Abb,
        MaximalStructureOptions::default(),
        AbbOptions::default(),
    );
    assert_eq!(
        scalar.cost_dimensions,
        vec![
            "gpu_cost".to_string(),
            "quality_drop".to_string(),
            "time_cost".to_string()
        ],
        "cost_dimensions must be alphabetised"
    );
    assert!(
        scalar.cost_weights_echo.is_none(),
        "scalar path must echo cost_weights_echo = None"
    );
    assert!(
        scalar.abb_cost_breakdown.is_some(),
        "abb_cost_breakdown must surface even on the scalar path"
    );

    // Weighted path: (0, 1, 0) weights quality_drop → ABB picks
    // the quality-minimising architecture.
    let weighted = analyze_source_with_full_options(
        MULTICOST,
        DumpAlgorithm::Abb,
        MaximalStructureOptions::default(),
        AbbOptions {
            cost_weights: Some(vec![0.0, 1.0, 0.0]),
            ..AbbOptions::default()
        },
    );
    assert_eq!(weighted.cost_weights_echo, Some(vec![0.0, 1.0, 0.0]));
    let breakdown = weighted
        .abb_cost_breakdown
        .as_ref()
        .expect("must have breakdown");
    // Quality-only weight picks m64+h16+long, quality_drop sum = 10.
    let qd = breakdown
        .iter()
        .find(|(d, _)| d == "quality_drop")
        .map(|(_, v)| *v)
        .unwrap();
    assert!(
        (qd - 10.0).abs() < 1e-9,
        "quality-weighted ABB must pick quality_drop=10 selection; got {qd}"
    );
}

#[test]
fn dump_dto_msg_algorithm_omits_abb_subschema_certs() {
    // When algorithm is just MSG (no ABB selection), the
    // ABB-subschema certs are `None` but the full-schema certs
    // still emit.
    let analysis = analyze_source_with_full_options(
        HSIKAN_SWEEP,
        DumpAlgorithm::Msg,
        MaximalStructureOptions::default(),
        AbbOptions::default(),
    );
    assert!(analysis.ok);
    assert_eq!(analysis.canonical_full.status, "PASS");
    assert_eq!(analysis.extension_full.status, "PASS");
    assert!(analysis.canonical_abb_subschema.is_none());
    assert!(analysis.extension_abb_subschema.is_none());
}

// ─── Phase 6: empirical NAS-divergence test ─────────────────────────
//
// Question the user posed: "did HSIKAN/Gömb gain anything from the
// canonical-axiom fix?" Phase 5's answer was: no direct gain on the
// existing sweeps (they have no by-products, so canonical, extension,
// and engine all agree). Phase 6 builds a counterfactual: a HSIKAN
// sweep WITH a by-product, and measures whether the engine's
// strict-vs-relaxed mode now picks materially different architectures.

#[test]
fn byproduct_sweep_is_canonical_feasible_both_bundles() {
    // Sanity: the canonical bundle accepts the by-product schema
    // (canonical S5 says by-product just needs ≥ 1 incident edge —
    // it has one, from cycle_topk_m4). The extension bundle's
    // E-StrictNoExcess fires on the by-product.
    let p = parse_and_lower(HSIKAN_BYPRODUCT);
    validate_canonical(&p).expect("by-product sweep must satisfy canonical S1..S5");
    let ext = validate_extension(&p)
        .expect_err("extension bundle must flag the injected redundancy_byproduct");
    assert!(ext.iter().any(|x| matches!(
        x, ExtensionAxiomViolation::NonReachingMaterials { offenders }
            if offenders.iter().any(|m|
                p.decl_to_name.get(m).map(|s| s.as_str()) == Some("redundancy_byproduct"))
    )));
}

#[test]
fn engine_selection_diverges_under_strict_vs_relaxed_on_byproduct() {
    // The core empirical claim.
    let p = parse_and_lower(HSIKAN_BYPRODUCT);

    // (a) strict_no_excess = true (default).
    let msg_strict = hymeko_pgraph::msg::maximal_structure_with_options(
        &p,
        hymeko_pgraph::msg::MaximalStructureOptions {
            strict_no_excess: true,
        },
    );
    let sol_strict = hymeko_pgraph::abb::solve_with_options(
        &p,
        &msg_strict,
        AbbOptions {
            strict_no_excess: true,
            ..AbbOptions::default()
        },
    )
    .expect("strict mode must still find a solution");

    // (b) strict_no_excess = false.
    let msg_relaxed = hymeko_pgraph::msg::maximal_structure_with_options(
        &p,
        hymeko_pgraph::msg::MaximalStructureOptions {
            strict_no_excess: false,
        },
    );
    let sol_relaxed = hymeko_pgraph::abb::solve_with_options(
        &p,
        &msg_relaxed,
        AbbOptions {
            strict_no_excess: false,
            ..AbbOptions::default()
        },
    )
    .expect("relaxed mode must find a solution");

    // Engine selections must differ in cost (strict pays the
    // by-product penalty; relaxed doesn't).
    assert!(
        sol_strict.cost > sol_relaxed.cost,
        "strict cost ({}) must exceed relaxed cost ({}) when a \
         by-product penalises the cheap path",
        sol_strict.cost,
        sol_relaxed.cost
    );

    // And the specific divergence: relaxed picks cycle_topk_m4
    // (cost 10); strict drops it and picks cycle_topk_m16 (cost 40).
    let m4 = p.name_to_decl["cycle_topk_m4"];
    let m16 = p.name_to_decl["cycle_topk_m16"];
    assert!(
        sol_relaxed.units.contains(&m4),
        "relaxed mode must pick the cheap cycle_topk_m4"
    );
    assert!(
        !sol_strict.units.contains(&m4),
        "strict mode must drop cycle_topk_m4 (it leaks a by-product)"
    );
    assert!(
        sol_strict.units.contains(&m16),
        "strict mode must fall back to cycle_topk_m16"
    );

    // Both selections satisfy canonical S1..S5 on their own projected
    // sub-schemas (the by-product never makes it into the strict
    // sub-schema; in the relaxed sub-schema it stays incident to
    // cycle_topk_m4 and canonical S5 still accepts it).
    let proj_strict = project_schema(&p, &sol_strict.units);
    let proj_relaxed = project_schema(&p, &sol_relaxed.units);
    validate_subschema_canonical(&proj_strict, &p).expect("strict sub-schema canonical-feasible");
    validate_subschema_canonical(&proj_relaxed, &p).expect("relaxed sub-schema canonical-feasible");

    // The extension bundle however catches relaxed mode (it still
    // contains the by-product M-node).
    let raws_relaxed: BTreeSet<DeclId> = p
        .raws
        .iter()
        .copied()
        .filter(|r| proj_relaxed.kind(*r).is_some())
        .collect();
    let ext_relaxed = ExtensionAxiomBundle::new(raws_relaxed.iter().copied())
        .validate(&proj_relaxed, &p.products);
    assert!(
        ext_relaxed.is_err(),
        "extension bundle must reject relaxed selection (by-product survives)"
    );

    // And the strict sub-schema is clean under extension too,
    // because cycle_topk_m4 (and therefore the by-product) is gone.
    let raws_strict: BTreeSet<DeclId> = p
        .raws
        .iter()
        .copied()
        .filter(|r| proj_strict.kind(*r).is_some())
        .collect();
    ExtensionAxiomBundle::new(raws_strict.iter().copied())
        .validate(&proj_strict, &p.products)
        .expect("strict sub-schema must satisfy extension bundle");

    eprintln!(
        "[divergence] strict picks {:?} cost={}; relaxed picks {:?} cost={}",
        sol_strict
            .units
            .iter()
            .map(|u| &p.decl_to_name[u])
            .collect::<Vec<_>>(),
        sol_strict.cost,
        sol_relaxed
            .units
            .iter()
            .map(|u| &p.decl_to_name[u])
            .collect::<Vec<_>>(),
        sol_relaxed.cost,
    );
}

#[test]
fn print_hsikan_and_gomb_abb_selection() {
    // Diagnostic: which specific units does ABB pick on the HSIKAN
    // and Gömb architecture-search P-graphs? The audit confirmed
    // these selections are canonical-feasible; this test exposes
    // them by name so they can be cited.
    for (name, src) in [
        ("HSIKAN/sweep_msg", HSIKAN_SWEEP),
        ("Gomb/sweep_msg", GOMB_SWEEP),
    ] {
        let p = parse_and_lower(src);
        let msg = maximal_structure(&p);
        let sol = abb_solve(&p, &msg).expect("ABB must succeed");
        let unit_names: Vec<String> = sol
            .units
            .iter()
            .map(|u| p.decl_to_name[u].clone())
            .collect();
        let msg_names: Vec<String> = msg
            .units
            .iter()
            .map(|u| p.decl_to_name[u].clone())
            .collect();
        println!(
            "[{name}] MSG units = {:?} ({}); ABB picks = {:?} (cost {})",
            msg_names,
            msg_names.len(),
            unit_names,
            sol.cost
        );
    }
}

fn outcome_brief_ext(r: &Result<(), Vec<ExtensionAxiomViolation>>) -> String {
    match r {
        Ok(()) => "PASS".to_string(),
        Err(v) => {
            let mut tags: Vec<&'static str> = Vec::new();
            for it in v {
                tags.push(match it {
                    ExtensionAxiomViolation::NonReachingMaterials { .. } => "E-NoExcess",
                    ExtensionAxiomViolation::UnitsWithDegreeZero { .. } => "E-WellFormed",
                    ExtensionAxiomViolation::ConsumedMaterialWithoutProducer { .. } => {
                        "E-ConsumedHasProducer"
                    }
                });
            }
            format!("FAIL [{}]", tags.join(","))
        }
    }
}
