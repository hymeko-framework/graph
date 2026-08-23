//! Tests for direct .pgip read/write (Stage P-io, 2026-05-19).
//!
//! Gated behind the `pgip` feature (on by default) — a `--no-default-features`
//! build has no `read_pgip`/`write_pgip`.
#![cfg(feature = "pgip")]
//!
//! Three properties to pin:
//!   1. `read_pgip` on a textbook chapter reproduces the topology
//!      we previously verified through the .hymeko intermediate.
//!   2. `write_pgip` + `read_pgip` round-trips a lowered graph
//!      (preserving units, materials, raws, products, costs, and
//!      input/output incidence — set-equal, name-equal).
//!   3. The written .pgip is byte-loadable by SQLite (sanity).

use std::collections::BTreeSet;
use std::path::PathBuf;

use hymeko_pgraph::abb::{AbbOptions, solve_with_options};
use hymeko_pgraph::lower;
use hymeko_pgraph::msg::maximal_structure;
use hymeko_pgraph::pgip_io::{read_pgip, write_pgip};
use parser::parse_description;

fn data_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("data/pgraph")
        .join(name)
}

// ─── Read path (option D) ────────────────────────────────────────────

#[test]
fn read_pgip_chapter6_canonical_optimum_is_9() {
    let path = data_path("Chapter6/example6_1.pgip");
    if !path.exists() {
        eprintln!("Chapter6 .pgip missing — skipping");
        return;
    }
    let graph = read_pgip(&path).expect("read_pgip must succeed on Chapter6");
    assert!(!graph.units.is_empty(), "Chapter6 must have units");
    assert_eq!(graph.units.len(), 7, "Chapter6 has 7 operating units");
    // Canonical MSG (7 units) + scalar ABB → {O2, O5, O7} at cost 9.0.
    // (Pre-2026-05-27 the buggy strict default returned {O1,O3,O6} at 18.)
    let m = maximal_structure(&graph);
    let abb = solve_with_options(&graph, &m, AbbOptions::default())
        .expect("Chapter6 ABB must find a feasible optimum");
    let names: BTreeSet<String> = abb
        .units
        .iter()
        .map(|d| graph.decl_to_name[d].clone())
        .collect();
    let expected: BTreeSet<String> = ["O2", "O5", "O7"].iter().map(|s| s.to_string()).collect();
    assert_eq!(
        names, expected,
        "Chapter6 canonical ABB units must be {{O2,O5,O7}}"
    );
    assert!(
        (abb.cost - 9.0).abs() < 1e-9,
        "Chapter6 cost = 9.0 expected, got {}",
        abb.cost
    );
}

#[test]
fn read_pgip_chapter3_structural_recovers_7_units_msg() {
    let path = data_path("Chapter3/example3_2.pgip");
    if !path.exists() {
        return;
    }
    let graph = read_pgip(&path).expect("read_pgip must succeed on Chapter3");
    assert_eq!(graph.units.len(), 7);
    // Canonical MSG keeps all 7 (book Example 3.2 maximal structure,
    // Fig. 3.6; 19 solution-structures). Pre-fix gave 3.
    let m = maximal_structure(&graph);
    assert_eq!(m.units.len(), 7, "Chapter3 canonical MSG = 7 units");
}

// ─── Round-trip (read .pgip → write .pgip → read back) ───────────────

fn graphs_equivalent(
    a: &hymeko_pgraph::lowering::LoweredPGraph,
    b: &hymeko_pgraph::lowering::LoweredPGraph,
) -> Result<(), String> {
    // Name-set equivalence (DeclIds may differ).
    let a_units: BTreeSet<&String> = a.units.iter().map(|d| &a.decl_to_name[d]).collect();
    let b_units: BTreeSet<&String> = b.units.iter().map(|d| &b.decl_to_name[d]).collect();
    if a_units != b_units {
        return Err(format!(
            "unit-name sets differ:\n  a={a_units:?}\n  b={b_units:?}"
        ));
    }
    let a_mats: BTreeSet<&String> = a.materials.iter().map(|d| &a.decl_to_name[d]).collect();
    let b_mats: BTreeSet<&String> = b.materials.iter().map(|d| &b.decl_to_name[d]).collect();
    if a_mats != b_mats {
        return Err(format!(
            "material-name sets differ:\n  a={a_mats:?}\n  b={b_mats:?}"
        ));
    }
    let a_raws: BTreeSet<&String> = a.raws.iter().map(|d| &a.decl_to_name[d]).collect();
    let b_raws: BTreeSet<&String> = b.raws.iter().map(|d| &b.decl_to_name[d]).collect();
    if a_raws != b_raws {
        return Err(format!("raw sets differ:\n  a={a_raws:?}\n  b={b_raws:?}"));
    }
    let a_prods: BTreeSet<&String> = a.products.iter().map(|d| &a.decl_to_name[d]).collect();
    let b_prods: BTreeSet<&String> = b.products.iter().map(|d| &b.decl_to_name[d]).collect();
    if a_prods != b_prods {
        return Err(format!(
            "product sets differ:\n  a={a_prods:?}\n  b={b_prods:?}"
        ));
    }
    // Per-unit name-keyed cost equality.
    for ua in &a.units {
        let name = &a.decl_to_name[ua];
        let ub = b
            .name_to_decl
            .get(name)
            .ok_or_else(|| format!("unit {name} present in a but missing in b"))?;
        let ca = a.costs.get(ua).copied().unwrap_or(1.0);
        let cb = b.costs.get(ub).copied().unwrap_or(1.0);
        if (ca - cb).abs() > 1e-9 {
            return Err(format!("unit {name} cost differs: a={ca} b={cb}"));
        }
        // In/out incidence by material-name (derived via the queries).
        let a_in: BTreeSet<&String> = a.inputs(*ua).iter().map(|d| &a.decl_to_name[d]).collect();
        let b_in: BTreeSet<&String> = b.inputs(*ub).iter().map(|d| &b.decl_to_name[d]).collect();
        if a_in != b_in {
            return Err(format!(
                "unit {name} inputs differ:\n  a={a_in:?}\n  b={b_in:?}"
            ));
        }
        let a_out: BTreeSet<&String> = a.outputs(*ua).iter().map(|d| &a.decl_to_name[d]).collect();
        let b_out: BTreeSet<&String> = b.outputs(*ub).iter().map(|d| &b.decl_to_name[d]).collect();
        if a_out != b_out {
            return Err(format!(
                "unit {name} outputs differ:\n  a={a_out:?}\n  b={b_out:?}"
            ));
        }
    }
    Ok(())
}

#[test]
fn roundtrip_chapter6_pgip_to_hymeko_lower_back() {
    let path = data_path("Chapter6/example6_1.pgip");
    if !path.exists() {
        return;
    }
    let g1 = read_pgip(&path).expect("read Chapter6");
    let tmp = std::env::temp_dir().join("pgip_io_chapter6_roundtrip.pgip");
    write_pgip(&g1, &tmp, None).expect("write back to .pgip");
    let g2 = read_pgip(&tmp).expect("read back from written .pgip");
    if let Err(e) = graphs_equivalent(&g1, &g2) {
        panic!("Chapter6 round-trip: {e}");
    }
}

#[test]
fn roundtrip_hda_hymeko_to_pgip_to_lower() {
    // Read HDA from .hymeko source, lower it, write as .pgip,
    // read .pgip back, compare.
    let path = data_path("hda.hymeko");
    let src = std::fs::read_to_string(&path).expect("hda.hymeko");
    let desc = parse_description(&src).expect("parse hda");
    let g1 = lower(&desc).expect("lower hda");
    let tmp = std::env::temp_dir().join("pgip_io_hda_roundtrip.pgip");
    write_pgip(&g1, &tmp, None).expect("write hda to .pgip");
    let g2 = read_pgip(&tmp).expect("read hda back from .pgip");
    if let Err(e) = graphs_equivalent(&g1, &g2) {
        panic!("HDA round-trip: {e}");
    }
}

#[test]
fn write_pgip_bakes_abb_result_into_run_history() {
    // Verify the .pgip we emit contains the ABB's computed optimum
    // in runHistory + unitsInStructure (so P-graph Studio displays
    // our solution alongside the topology).
    let src = std::fs::read_to_string(data_path("hda.hymeko")).unwrap();
    let desc = parse_description(&src).expect("parse");
    let g = lower(&desc).expect("lower");
    let m = maximal_structure(&g);
    let abb = solve_with_options(&g, &m, AbbOptions::default())
        .expect("HDA must have a feasible optimum");
    let tmp = std::env::temp_dir().join("pgip_io_hda_with_abb.pgip");
    write_pgip(&g, &tmp, Some(&abb)).expect("write with abb");

    // Re-open and query.
    let conn = rusqlite::Connection::open(&tmp).expect("re-open");
    let n_runs: i64 = conn
        .query_row("SELECT COUNT(*) FROM runHistory", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n_runs, 1, "runHistory must have exactly 1 row");
    let cost: f64 = conn
        .query_row("SELECT optimalCost FROM runHistory", [], |r| r.get(0))
        .unwrap();
    // Canonical optimum: {Mixer, Reactor} at 350 (Methane vented;
    // Disposal is excluded from the maximal structure).
    assert!(
        (cost - 350.0).abs() < 1e-9,
        "HDA optimal cost is 350, got {cost}"
    );
    let n_units: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM unitsInStructure WHERE structureId = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        n_units, 2,
        "HDA canonical optimum is 2 units (Mixer, Reactor)"
    );
}
