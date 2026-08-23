//! End-to-end: parse a P-graph written in HyMeKo grammar, lower it,
//! then run MSG / SSG / ABB.

use std::collections::BTreeSet;

use hymeko_pgraph::{AbbOptions, SsgOptions, abb_solve, lower, maximal_structure, ssg_enumerate};
use parser::parse_description;

const HDA_SRC: &str = include_str!("../../data/pgraph/hda.hymeko");

#[test]
fn parses_hda_pgraph() {
    let d = parse_description(HDA_SRC).expect("parser must accept HDA file");
    assert_eq!(d.name, "HDA");
    // The lowering walks the `context { ... }` wrapper one level.
    let p = lower(&d).expect("lowering must succeed");
    // 5 materials.
    assert_eq!(p.materials.len(), 5);
    // 4 operating units.
    assert_eq!(p.units.len(), 4);
    // Raws: Toluene, H2.
    let raw_names: BTreeSet<&str> = p.raws.iter().map(|d| p.decl_to_name[d].as_str()).collect();
    assert_eq!(raw_names, BTreeSet::from(["Toluene", "H2"]));
    // Required products: Benzene.
    let prod_names: BTreeSet<&str> = p
        .products
        .iter()
        .map(|d| p.decl_to_name[d].as_str())
        .collect();
    assert_eq!(prod_names, BTreeSet::from(["Benzene"]));
}

#[test]
fn unit_signatures_lower_correctly() {
    let d = parse_description(HDA_SRC).unwrap();
    let p = lower(&d).unwrap();

    let mixer = p.name_to_decl["Mixer"];
    let reactor = p.name_to_decl["Reactor"];
    let direct = p.name_to_decl["DirectSynth"];
    let disposal = p.name_to_decl["Disposal"];
    let toluene = p.name_to_decl["Toluene"];
    let h2 = p.name_to_decl["H2"];
    let mix = p.name_to_decl["Mix"];
    let benzene = p.name_to_decl["Benzene"];
    let methane = p.name_to_decl["Methane"];

    assert_eq!(*p.inputs(mixer), BTreeSet::from([toluene, h2]));
    assert_eq!(*p.outputs(mixer), BTreeSet::from([mix]));

    assert_eq!(*p.inputs(reactor), BTreeSet::from([mix]));
    assert_eq!(*p.outputs(reactor), BTreeSet::from([benzene, methane]));

    assert_eq!(*p.inputs(direct), BTreeSet::from([toluene, h2]));
    assert_eq!(*p.outputs(direct), BTreeSet::from([benzene]));

    assert_eq!(*p.inputs(disposal), BTreeSet::from([methane]));
    assert!(p.outputs(disposal).is_empty());

    assert_eq!(p.costs[&mixer], 100.0);
    assert_eq!(p.costs[&reactor], 250.0);
    assert_eq!(p.costs[&direct], 800.0);
    assert_eq!(p.costs[&disposal], 50.0);
}

#[test]
fn msg_canonical_drops_disposal_sink_for_hda() {
    // Canonical MSG (book Ch.4 composition phase) keeps only units that
    // are backward-reachable from a product. Disposal consumes Methane
    // and produces nothing, so it reaches no product (axiom S4) and is
    // excluded — even though it is forward-feasible. (Pre-2026-05-27 the
    // buggy default kept it.)
    let d = parse_description(HDA_SRC).unwrap();
    let p = lower(&d).unwrap();
    let m = maximal_structure(&p);
    let names: BTreeSet<&str> = m.units.iter().map(|d| p.decl_to_name[d].as_str()).collect();
    assert_eq!(
        names,
        BTreeSet::from(["Mixer", "Reactor", "DirectSynth"]),
        "canonical maximal structure excludes the Disposal sink"
    );
    assert!(m.materials.contains(&p.name_to_decl["Benzene"]));
}

#[test]
fn msg_drops_a_forward_unreachable_unit() {
    // A unit whose input is *not* producible by any chain from the
    // raw set must be dropped by the forward pass.
    let bad = r#"
        Bad{}
        context {
            R <material, raw>;
            P <material, product>;
            X <material>;       // never produced, never raw
            @U_good <unit> {
                (-R, +P);
            }
            @U_bad <unit> {
                (-X, +P);       // input X is unreachable
            }
        }
    "#;
    let d = parse_description(bad).unwrap();
    let p = lower(&d).unwrap();
    let m = maximal_structure(&p);
    assert!(m.units.contains(&p.name_to_decl["U_good"]));
    assert!(
        !m.units.contains(&p.name_to_decl["U_bad"]),
        "U_bad consumes X which no chain from raws produces"
    );
}

#[test]
fn msg_drops_a_backward_useless_unit() {
    // A unit whose output is neither product nor consumed by any
    // other unit must be dropped by the backward pass.
    let bad = r#"
        Bad{}
        context {
            R <material, raw>;
            P <material, product>;
            Junk <material>;
            @U_good <unit> {
                (-R, +P);
            }
            @U_useless <unit> {
                (-R, +Junk);    // Junk has no consumer
            }
        }
    "#;
    let d = parse_description(bad).unwrap();
    let p = lower(&d).unwrap();
    let m = maximal_structure(&p);
    assert!(m.units.contains(&p.name_to_decl["U_good"]));
    assert!(
        !m.units.contains(&p.name_to_decl["U_useless"]),
        "U_useless produces a dead-end material — MSG drops it"
    );
}

#[test]
fn ssg_finds_known_feasible_structures() {
    let d = parse_description(HDA_SRC).unwrap();
    let p = lower(&d).unwrap();
    let m = maximal_structure(&p);

    // Canonical SSG (no no-excess rule): excess Methane is vented, and
    // the Disposal sink is not even in the maximal structure (it reaches
    // no product). Solution-structures are enumerated over
    // {Mixer, Reactor, DirectSynth}.
    let solutions = ssg_enumerate(&p, &m);

    let mixer = p.name_to_decl["Mixer"];
    let reactor = p.name_to_decl["Reactor"];
    let direct = p.name_to_decl["DirectSynth"];

    let by_units = |us: &[_]| {
        let s: BTreeSet<_> = us.iter().copied().collect();
        solutions.iter().any(|sol| sol.units == s)
    };

    // One-shot direct synthesis — feasible.
    assert!(by_units(&[direct]), "DirectSynth alone must be feasible");
    // Two-stage route — feasible (Methane vented, canonical semantics).
    assert!(
        by_units(&[mixer, reactor]),
        "Mixer + Reactor is feasible under canonical (relaxed) semantics"
    );
    // Both routes together — feasible.
    assert!(by_units(&[mixer, reactor, direct]));
    // Mixer alone cannot produce Benzene.
    assert!(!by_units(&[mixer]));
}

#[test]
fn ssg_relaxed_includes_excess_byproduct() {
    let d = parse_description(HDA_SRC).unwrap();
    let p = lower(&d).unwrap();
    let m = maximal_structure(&p);

    let opts = SsgOptions {
        strict_no_excess: false,
        require_at_least_one_unit: true,
    };
    let solutions = hymeko_pgraph::ssg::enumerate_with_options(&p, &m, opts);

    let mixer = p.name_to_decl["Mixer"];
    let reactor = p.name_to_decl["Reactor"];

    let pair = BTreeSet::from([mixer, reactor]);
    assert!(
        solutions.iter().any(|s| s.units == pair),
        "relaxed SSG must accept Mixer+Reactor with excess Methane"
    );
}

#[test]
fn abb_finds_minimum_cost_route() {
    let d = parse_description(HDA_SRC).unwrap();
    let p = lower(&d).unwrap();
    let m = maximal_structure(&p);

    let mixer = p.name_to_decl["Mixer"];
    let reactor = p.name_to_decl["Reactor"];
    let direct = p.name_to_decl["DirectSynth"];

    // Canonical (default) optimum: {Mixer, Reactor} = 100 + 250 = 350,
    // venting the Methane byproduct. {DirectSynth} = 800 is dearer.
    let sol = abb_solve(&p, &m).expect("ABB must find a solution");
    assert_eq!(
        sol.units,
        BTreeSet::from([mixer, reactor]),
        "canonical optimum is the Mixer+Reactor route"
    );
    assert!(
        (sol.cost - 350.0).abs() < 1e-9,
        "minimum cost is 350, got {}",
        sol.cost
    );
    assert!(sol.explored > 0);

    // Non-canonical strict no-waste opt-in: Methane has no consumer in the
    // canonical maximal structure (the Disposal sink reaches no product and
    // is excluded), so {Mixer,Reactor} is strict-infeasible and the strict
    // optimum is the one-shot DirectSynth route (no byproduct) at 800.
    let strict = hymeko_pgraph::abb::solve_with_options(
        &p,
        &m,
        AbbOptions {
            strict_no_excess: true,
            ..AbbOptions::default()
        },
    )
    .expect("strict ABB must find the no-waste route");
    assert_eq!(strict.units, BTreeSet::from([direct]));
    assert!((strict.cost - 800.0).abs() < 1e-9);
}

#[test]
fn abb_returns_none_when_infeasible() {
    // Same file but with the products demand redirected to a material
    // that no unit can produce.  Lowering doesn't enforce reachability
    // — that's MSG / SSG / ABB's job.
    let bad = r#"
        Bad{}
        context {
            X <material, raw>;
            Z <material, product>;
        }
    "#;
    let d = parse_description(bad).expect("parses");
    let p = lower(&d).expect("lowers");
    let m = maximal_structure(&p);
    let sol = abb_solve(&p, &m);
    assert!(
        sol.is_none(),
        "no unit produces Z, so ABB must report infeasible"
    );
}
