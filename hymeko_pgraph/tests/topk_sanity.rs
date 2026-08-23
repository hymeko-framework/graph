//! Fast sanity check (2026-05-27): top-k (n-best) solution-structure
//! enumeration under each regime, including the **combined** (Composite)
//! way. Run: `cargo test -p hymeko_pgraph --test topk_sanity -- --nocapture`.
//!
//! Surfaces the key property: `CostDominance` preserves the *optimum cost*
//! but collapses dominated distinct alternatives — so it is faithful for
//! arg-min, not for "k distinct n-best structures". `NoExcess` changes the
//! admissible set (no-waste). `Composite` stacks both.

use std::collections::BTreeSet;
use std::time::Instant;

use hymeko::common::ids::DeclId;
use hymeko_pgraph::lowering::LoweredPGraph;
use hymeko_pgraph::msg::maximal_structure_with_regime;
use hymeko_pgraph::regime::{Canonical, Composite, CostDominance, NoExcess, Regime};
use hymeko_pgraph::{lower, ssg_dm_enumerate};
use parser::parse_description;

/// Scalar cost of a structure = sum of per-unit costs (default 1.0).
fn cost_of(p: &LoweredPGraph, units: &BTreeSet<DeclId>) -> f64 {
    units.iter().map(|u| p.costs.get(u).copied().unwrap_or(1.0)).sum()
}

fn names(p: &LoweredPGraph, units: &BTreeSet<DeclId>) -> Vec<String> {
    let mut v: Vec<String> = units.iter().map(|d| p.decl_to_name[d].clone()).collect();
    v.sort();
    v
}

/// Regime-aware top-k: refine the maximal structure, enumerate structural
/// solution-structures, keep those the regime admits, rank by cost, take k.
/// Returns (total_admissible, elapsed_micros, top_k:[(cost,names)]).
fn topk(
    p: &LoweredPGraph,
    regime: &dyn Regime,
    k: usize,
) -> (usize, u128, Vec<(f64, Vec<String>)>) {
    let t0 = Instant::now();
    let msg = maximal_structure_with_regime(p, regime);
    let mut scored: Vec<(f64, BTreeSet<DeclId>)> = ssg_dm_enumerate(p, &msg)
        .into_iter()
        .filter(|s| regime.structure_admissible(p, &s.units))
        .map(|s| (cost_of(p, &s.units), s.units))
        .collect();
    scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let micros = t0.elapsed().as_micros();
    let total = scored.len();
    let top: Vec<(f64, Vec<String>)> =
        scored.into_iter().take(k).map(|(c, u)| (c, names(p, &u))).collect();
    (total, micros, top)
}

const SYNTH: &str = r#"
T{}
context {
    P   <material, product>;
    R   <material, raw>;
    Mid <material>;
    W   <material>;
    @a <unit> 1 { (-R, +P); }            // cheapest direct producer of P
    @b <unit> 5 { (-R, +P); }            // dearer interchangeable twin of `a`
    @c <unit> 2 { (-Mid, +P); }          // alt route to P (needs Mid)
    @d <unit> 1 { (-R, +Mid); }          // produces Mid for `c`
    @e <unit> 1 { (-R, +P, +W); }        // produces P + unconsumed waste W
}
"#;

#[test]
fn topk_under_each_regime_synthetic() {
    let p = lower(&parse_description(SYNTH).unwrap()).unwrap();
    let k = 3;
    let cd_ne = Composite::new(vec![&CostDominance as &dyn Regime, &NoExcess]);
    let regimes: [(&str, &dyn Regime); 4] = [
        ("canonical", &Canonical),
        ("no-excess", &NoExcess),
        ("cost-dominance", &CostDominance),
        ("cost-dominance+no-excess", &cd_ne),
    ];

    println!("\n=== top-{k} solution-structures (synthetic) ===");
    let mut canonical_best = f64::INFINITY;
    for (label, r) in regimes {
        let (total, micros, top) = topk(&p, r, k);
        println!("[{label}]  structures={total}  ({micros} us)");
        for (i, (c, u)) in top.iter().enumerate() {
            println!("    #{}: cost {:>4.1}  {:?}", i + 1, c, u);
        }
        let best = top.first().map(|(c, _)| *c).unwrap_or(f64::INFINITY);
        if label == "canonical" {
            canonical_best = best;
        } else {
            // Every regime must preserve the #1 optimum COST (a=1.0):
            // cost-dominance prunes only dominated units; no-excess only
            // drops waste structures (here `e`, which is not the optimum).
            assert!(
                (best - canonical_best).abs() < 1e-9,
                "{label}: #1 optimum cost drifted ({best} vs {canonical_best})"
            );
        }
    }

    // Concrete invariants of this rigged graph:
    // - canonical admits the waste producer `e`; cost-dominance prunes the
    //   dearer twin `b`; the composite drops BOTH -> fewest structures.
    let (n_can, _, _) = topk(&p, &Canonical, k);
    let (n_cd, _, _) = topk(&p, &CostDominance, k);
    let (n_comp, _, _) = topk(&p, &cd_ne, k);
    assert!(n_cd < n_can, "cost-dominance must enumerate fewer than canonical");
    assert!(n_comp <= n_cd, "composite must not enumerate more than cost-dominance alone");
    assert!(n_comp < n_can, "combined way prunes the most");
    println!("structures: canonical={n_can}  cost-dominance={n_cd}  composite={n_comp}");
}

#[test]
fn topk_real_example4_1() {
    // Book Example 4.1 (costed, 7-unit canonical maximal structure).
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("data/pgraph/Chapter4/example4_1.hymeko");
    if !path.exists() {
        return;
    }
    let p = lower(&parse_description(&std::fs::read_to_string(&path).unwrap()).unwrap()).unwrap();
    let (total, micros, top) = topk(&p, &Canonical, 3);
    println!("\n=== top-3 (Example 4.1, canonical) — {total} structures, {micros} us ===");
    for (i, (c, u)) in top.iter().enumerate() {
        println!("    #{}: cost {:>4.1}  {:?}", i + 1, c, u);
    }
    assert!(total > 0, "Example 4.1 must yield solution-structures");
}
