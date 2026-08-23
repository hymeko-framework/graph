//! Stage P-mo (2026-05-19) — multi-objective ABB integration tests.
//!
//! Verifies that the weighted-sum cost path produces sensible
//! structural decisions on the `methanol_synthesis.hymeko` worked
//! example, and that the scalar fallback is byte-identical with
//! the pre-2026-05-19 behaviour when `cost_weights` is `None`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use hymeko::common::ids::DeclId;
use hymeko_pgraph::abb::{AbbOptions, solve_with_options};
use hymeko_pgraph::lower;
use hymeko_pgraph::msg::maximal_structure;
use parser::parse_description;

fn fixture_path(name: &str) -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest)
        .parent()
        .unwrap()
        .join("data/pgraph")
        .join(name)
}

fn load_methanol() -> hymeko_pgraph::lowering::LoweredPGraph {
    let path = fixture_path("methanol_synthesis.hymeko");
    let src = std::fs::read_to_string(&path).expect("methanol_synthesis.hymeko must be present");
    let desc = parse_description(&src).expect("parse must succeed");
    lower(&desc).expect("lowering must succeed")
}

fn decl(p: &hymeko_pgraph::lowering::LoweredPGraph, name: &str) -> DeclId {
    *p.name_to_decl
        .get(name)
        .unwrap_or_else(|| panic!("missing decl {name}"))
}

fn names(p: &hymeko_pgraph::lowering::LoweredPGraph, units: &BTreeSet<DeclId>) -> BTreeSet<String> {
    units
        .iter()
        .filter_map(|d| p.decl_to_name.get(d).cloned())
        .collect()
}

// ─── Lowering ────────────────────────────────────────────────────────

#[test]
fn lowering_collects_cost_dimensions_alphabetised() {
    let p = load_methanol();
    // The .hymeko declares capex, co2, h2o, opex; alphabetised order
    // is [capex, co2, h2o, opex].
    assert_eq!(
        p.cost_dimensions,
        vec![
            "capex".to_string(),
            "co2".to_string(),
            "h2o".to_string(),
            "opex".to_string(),
        ]
    );
}

#[test]
fn lowering_populates_per_unit_cost_vectors() {
    let p = load_methanol();
    // SMR's tagged costs: capex 600, opex 210, co2 310, h2o 12.
    // In alphabetised order: [capex 600, co2 310, h2o 12, opex 210].
    let smr = decl(&p, "SMR");
    let v = p
        .cost_vectors
        .get(&smr)
        .expect("SMR must have a multi-cost vector");
    assert_eq!(v, &vec![600.0, 310.0, 12.0, 210.0]);
}

#[test]
fn lowering_scalar_costs_kept_byte_identical_to_pre_p_mo() {
    let p = load_methanol();
    // The scalar cost = the edge's value (e.g. SMR's edge value is 600).
    // The multi-objective extension does NOT overwrite this.
    let smr = decl(&p, "SMR");
    let scalar = p.costs.get(&smr).copied().unwrap();
    assert!((scalar - 600.0).abs() < 1e-9);
    let elec = decl(&p, "Electrolyzer");
    assert!((p.costs.get(&elec).copied().unwrap() - 1400.0).abs() < 1e-9);
}

// ─── ABB --- single-criterion fallback ───────────────────────────────

#[test]
fn abb_scalar_path_byte_identical_when_weights_none() {
    let p = load_methanol();
    let msg = maximal_structure(&p);

    // Scalar path: cost_weights = None.
    let sol = solve_with_options(
        &p,
        &msg,
        AbbOptions {
            strict_no_excess: false,
            max_explored: 1_000_000,
            cost_weights: None,
        },
    )
    .expect("scalar ABB must find a solution");

    // Canonical (no-excess-free) optimum picks the cheapest CO2 source
    // (CaptureFlue 800 < CaptureDAC 1900) and the cheapest H2 route
    // (SMR 600 < Electrolyzer 1400) plus the reactor chain:
    //   {CaptureFlue, SMR, MixerBlue, MeOHReactor, Distillation}
    //   = 800 + 600 + 90 + 1100 + 350 = 2940.
    // The waste_water / steam byproduct-consumer sinks (WaterTreatment,
    // SteamRecycle) reach no product and are excluded from the maximal
    // structure, so they are not in the optimum (excess is vented).
    let s = names(&p, &sol.units);
    assert!(
        s.contains("CaptureFlue"),
        "scalar should pick CaptureFlue, got {:?}",
        s
    );
    assert!(s.contains("SMR"), "scalar should pick SMR, got {:?}", s);
    assert!(
        s.contains("MixerBlue"),
        "scalar should pick MixerBlue (uses SMR's h2_blue)"
    );
    assert!(s.contains("MeOHReactor"));
    assert!(s.contains("Distillation"));
    assert!(
        (sol.cost - 2940.0).abs() < 1e-9,
        "canonical scalar optimum is 2940, got {}",
        sol.cost
    );
    assert!(
        !s.contains("WaterTreatment"),
        "sink not in canonical maximal structure"
    );
    assert!(
        !s.contains("SteamRecycle"),
        "sink not in canonical maximal structure"
    );
    assert!(!s.contains("CaptureDAC"));
    assert!(!s.contains("Electrolyzer"));
    assert!(!s.contains("MixerGreen"));
}

// ─── ABB --- multi-objective: pure CAPEX weight ──────────────────────

#[test]
fn abb_capex_only_weights_match_scalar_route() {
    let p = load_methanol();
    let msg = maximal_structure(&p);

    // Pure CAPEX weight should match the scalar path's structural
    // choice (both prefer SMR + MixerBlue + CaptureFlue), because
    // the scalar `costs` are themselves CAPEX-aligned in this
    // example.
    let weights = vec![1.0, 0.0, 0.0, 0.0]; // CAPEX, CO2, H2O, OPEX
    let sol = solve_with_options(
        &p,
        &msg,
        AbbOptions {
            strict_no_excess: false,
            max_explored: 1_000_000,
            cost_weights: Some(weights),
        },
    )
    .expect("CAPEX-only ABB must find a solution");

    let s = names(&p, &sol.units);
    assert!(
        s.contains("CaptureFlue"),
        "CAPEX-only should pick CaptureFlue"
    );
    assert!(
        s.contains("SMR"),
        "CAPEX-only should pick SMR (cheaper CAPEX than Electrolyzer)"
    );
}

// ─── ABB --- multi-objective: heavy CO2 weight ───────────────────────

#[test]
fn abb_co2_heavy_weights_switch_to_green_route() {
    let p = load_methanol();
    let msg = maximal_structure(&p);

    // Heavy CO2 weight should force the structural switch:
    //   CaptureFlue (CO2=45) vs CaptureDAC (CO2=12)
    //   SMR (CO2=310!)        vs Electrolyzer (CO2=18)
    // With weight 100 on the CO2 dim, SMR's 31000 CO2 penalty
    // dwarfs its 600 CAPEX advantage. Should switch to green.
    let weights = vec![0.01, 100.0, 0.01, 0.01]; // CAPEX, CO2, H2O, OPEX
    let sol = solve_with_options(
        &p,
        &msg,
        AbbOptions {
            strict_no_excess: false,
            max_explored: 1_000_000,
            cost_weights: Some(weights),
        },
    )
    .expect("CO2-heavy ABB must find a solution");

    let s = names(&p, &sol.units);
    assert!(
        s.contains("Electrolyzer"),
        "CO2-heavy should pick green H2 via Electrolyzer (not SMR), got {:?}",
        s
    );
    assert!(
        !s.contains("SMR"),
        "CO2-heavy should NOT pick SMR (carbon-emitting), got {:?}",
        s
    );
    // CaptureDAC has CO2=12 vs CaptureFlue's CO2=45 — with very heavy
    // CO2 weight DAC should win the upstream slot too.
    assert!(
        s.contains("CaptureDAC"),
        "CO2-heavy should prefer DAC over flue capture, got {:?}",
        s
    );
}

// ─── ABB --- multi-objective: H2O-heavy weights swap back ────────────

#[test]
fn abb_h2o_heavy_weights_avoid_electrolyzer() {
    let p = load_methanol();
    let msg = maximal_structure(&p);

    // Heavy H2O weight punishes Electrolyzer (h2o=160) but not SMR
    // (h2o=12). Should fall back to the SMR route despite CO2.
    let weights = vec![0.01, 0.01, 100.0, 0.01]; // CAPEX, CO2, H2O, OPEX
    let sol = solve_with_options(
        &p,
        &msg,
        AbbOptions {
            strict_no_excess: false,
            max_explored: 1_000_000,
            cost_weights: Some(weights),
        },
    )
    .expect("H2O-heavy ABB must find a solution");

    let s = names(&p, &sol.units);
    assert!(
        s.contains("SMR"),
        "H2O-heavy should fall back to SMR (low water), got {:?}",
        s
    );
    assert!(
        !s.contains("Electrolyzer"),
        "H2O-heavy should avoid Electrolyzer (high water), got {:?}",
        s
    );
}

// ─── ABB --- different weight ratios pick structurally different opt. ─

#[test]
fn abb_different_weights_pick_different_optima() {
    let p = load_methanol();
    let msg = maximal_structure(&p);

    let opt_capex = solve_with_options(
        &p,
        &msg,
        AbbOptions {
            strict_no_excess: false,
            max_explored: 1_000_000,
            cost_weights: Some(vec![1.0, 0.0, 0.0, 0.0]),
        },
    )
    .expect("CAPEX-only ABB");

    let opt_co2 = solve_with_options(
        &p,
        &msg,
        AbbOptions {
            strict_no_excess: false,
            max_explored: 1_000_000,
            cost_weights: Some(vec![0.01, 100.0, 0.01, 0.01]),
        },
    )
    .expect("CO2-heavy ABB");

    // The optimal STRUCTURES must differ.
    assert_ne!(
        opt_capex.units, opt_co2.units,
        "weight changes must surface as structural changes"
    );
}

// ─── ABB --- missing-dimension unit defaults to zero contribution ─────

#[test]
fn abb_unit_missing_a_dimension_contributes_zero() {
    // Construct a tiny synthetic P-graph where one unit declares only
    // `cost <capex> N;` (no CO2 tag) — its CO2 weight contribution
    // must be zero, not a panic or a NaN.
    let src = r#"
tiny {}
context {
    a <material, raw>;
    b <material, product>;
    @U <unit> 50 {
        cost <capex> 50;
        (-a, +b);
    }
}
"#;
    let desc = parse_description(src).expect("parse");
    let p = lower(&desc).expect("lower");
    let msg = maximal_structure(&p);

    // CAPEX-only run.
    let s_capex = solve_with_options(
        &p,
        &msg,
        AbbOptions {
            strict_no_excess: false,
            max_explored: 1_000,
            cost_weights: Some(vec![1.0]),
        },
    )
    .expect("CAPEX-only");
    assert!((s_capex.cost - 50.0).abs() < 1e-9);

    // CO2-only run: U has no CO2 dim → effective_cost = 0.
    // Since cost_dimensions was alphabetised it's ["capex"]; weight
    // vector of length 2 has its second slot unused (pad with zero
    // via min-len defensive logic).
    let s_co2 = solve_with_options(
        &p,
        &msg,
        AbbOptions {
            strict_no_excess: false,
            max_explored: 1_000,
            cost_weights: Some(vec![0.0, 100.0]),
        },
    )
    .expect("CO2-only");
    assert!(
        s_co2.cost.abs() < 1e-9,
        "missing dim must contribute zero, got cost={}",
        s_co2.cost
    );
}
