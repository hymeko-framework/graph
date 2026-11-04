//! V&V for the direct-Ir AND/OR algebra (mission §3/§8): PGraphView adjacency parity vs the schema, the
//! BooleanExact-closure == `close_producible` proof, AND/OR truth semantics, cycles, duplicate incidence,
//! neutral rejection, empty graphs, out-of-range rejection, convergence/non-convergence, determinism, and
//! the hypergraph-native ABB, whose unit firing decides every reachability cut as the schema closure does.
//!
//! Sources use the `<isa>` archetype classification (via `meta_pgraph.hymeko`) so `lower_resolved` yields
//! both the canonical `Ir` (for `PGraphView`/`IncidenceIndex`) and the `LoweredPGraph` (for
//! `close_producible`) from one compile.

use std::collections::BTreeSet;
use std::sync::Arc;

use hymeko::common::ids::DeclId;
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::Ir;
use hymeko::module_store::module_store::{CompiledProgram, ModuleStore};
use hymeko::module_store::source_provider::MemProvider;
use hymeko::util::real_parser::RealParser;

use hymeko_pgraph::andor::{
    AndOrNorm, AndOrWorkspace, BooleanExact, Godel, Lukasiewicz, NormError, Product,
    PropagationConfig, Reliability, UnitReliability, propagate,
};
use hymeko_pgraph::lowering::LoweredPGraph;
use hymeko_pgraph::meta_resolve::lower_resolved;
use hymeko_pgraph::msg::{close_producible, maximal_structure_with_regime};
use hymeko_pgraph::regime::Canonical;
use hymeko_pgraph::view::{PGraphView, ViewError};
use hymeko_pgraph::{Abb, AdditiveCost, BoundedAbb, ObjectiveSolution};

const META: &str = include_str!("../data/meta_pgraph.hymeko");

const HEADER: &str = "R {\n    @\"meta_pgraph.hymeko\";\n    using pgraph.raw as raw;\n    using pgraph.product as product;\n    using pgraph.intermediate as inter;\n    using pgraph.process as process;\n}\n";

/// Compile a `<isa>`-style body into an `Ir` (+ store for the interner). The store must be kept alive
/// alongside the returned program.
fn compile_isa(body: &str) -> (Arc<CompiledProgram>, ModuleStore<MemProvider, RealParser>) {
    let src = format!("{HEADER}body {{\n{body}\n}}\n");
    let provider = MemProvider::default()
        .with_file("m.hymeko", src.as_str())
        .with_file("meta_pgraph.hymeko", META);
    let mut ms = ModuleStore::new(provider, RealParser);
    let prog = ms
        .compile(std::path::Path::new("m.hymeko"))
        .unwrap_or_else(|e| panic!("source must compile: {e:?}\n---\n{src}"));
    (prog, ms)
}

fn lower_of(prog: &CompiledProgram, ms: &ModuleStore<MemProvider, RealParser>) -> LoweredPGraph {
    lower_resolved(&prog.ir, &ms.it).unwrap_or_else(|e| panic!("lower: {e:?}"))
}

fn producible(ir: &Ir, lowered: &LoweredPGraph) -> BTreeSet<DeclId> {
    let index = build_incidence_index(ir).unwrap();
    let view = PGraphView::from_lowered(&index, lowered);
    let mut ws = AndOrWorkspace::new();
    let cfg = PropagationConfig::boolean_exact(lowered.units.len(), lowered.materials.len());
    propagate(&view, &BooleanExact, &UnitReliability, &mut ws, cfg)
        .unwrap()
        .producible()
        .clone()
}

/// The load-bearing parity: adjacency == schema, BooleanExact == close_producible, deterministic.
fn parity_check(ir: &Ir, lowered: &LoweredPGraph) {
    let index = build_incidence_index(ir).expect("index");
    let view = PGraphView::from_lowered(&index, lowered);
    view.validate().expect("view valid");

    for &u in &lowered.units {
        assert_eq!(
            view.inputs_set(u),
            *lowered.inputs(u),
            "inputs differ for unit {}",
            u.raw()
        );
        assert_eq!(
            view.outputs_set(u),
            *lowered.outputs(u),
            "outputs differ for unit {}",
            u.raw()
        );
    }
    for &m in &lowered.materials {
        assert_eq!(
            view.producers_set(m),
            *lowered.producers(m),
            "producers differ for {}",
            m.raw()
        );
        assert_eq!(
            view.consumers_set(m),
            *lowered.consumers(m),
            "consumers differ for {}",
            m.raw()
        );
    }

    let mut ws = AndOrWorkspace::new();
    let cfg = PropagationConfig::boolean_exact(lowered.units.len(), lowered.materials.len());
    let avail = propagate(&view, &BooleanExact, &UnitReliability, &mut ws, cfg).expect("propagate");
    let expected = close_producible(lowered, &lowered.units, &lowered.raws);
    assert_eq!(
        avail.producible(),
        &expected,
        "BooleanExact producible != close_producible"
    );

    let avail2 =
        propagate(&view, &BooleanExact, &UnitReliability, &mut ws, cfg).expect("propagate2");
    assert_eq!(avail.producible(), avail2.producible(), "non-deterministic");
}

const MULTI_AND: &str = "    A: + <isa> raw {}\n    B: + <isa> raw {}\n    C: + <isa> product {}\n    @U { (+C, -A, -B); }";
const MULTI_OR: &str = "    A: + <isa> raw {}\n    B: + <isa> raw {}\n    M: + <isa> product {}\n    @U1 { (+M, -A); }\n    @U2 { (+M, -B); }";
const CYCLE_NO_RAW: &str = "    A: + <isa> inter {}\n    B: + <isa> product {}\n    @U1 { (+B, -A); }\n    @U2 { (+A, -B); }";
const DUP_INPUT: &str =
    "    A: + <isa> raw {}\n    C: + <isa> product {}\n    @U { (+C, -A, -A); }";
const CHAIN: &str = "    A: + <isa> raw {}\n    B: + <isa> inter {}\n    C: + <isa> inter {}\n    D: + <isa> product {}\n    @U1 { (+B, -A); }\n    @U2 { (+C, -B); }\n    @U3 { (+D, -C); }";
const REVERSE_CHAIN: &str = "    A: + <isa> raw {}
    B: + <isa> inter {}
    C: + <isa> inter {}
    D: + <isa> product {}
    @U3 { (+D, -C); }
    @U2 { (+C, -B); }
    @U1 { (+B, -A); }";

#[test]
fn boolean_exact_equals_close_producible_on_multiple_graphs() {
    for body in [
        MULTI_AND,
        MULTI_OR,
        CYCLE_NO_RAW,
        DUP_INPUT,
        CHAIN,
        REVERSE_CHAIN,
    ] {
        let (prog, ms) = compile_isa(body);
        let lowered = lower_of(&prog, &ms);
        parity_check(&prog.ir, &lowered);
    }
}

#[test]
fn abb_unit_firing_decides_every_reachability_cut_as_the_schema_closure() {
    let trace = |s: &ObjectiveSolution| {
        (s.units.clone(), s.value, s.explored, s.pruned_by_bound, s.pruned_by_reachability, s.complete)
    };
    let mut cuts = 0;
    for body in [MULTI_AND, MULTI_OR, CYCLE_NO_RAW, DUP_INPUT, CHAIN, REVERSE_CHAIN] {
        let (prog, ms) = compile_isa(body);
        let lowered = lower_of(&prog, &ms);
        let index = build_incidence_index(&prog.ir).expect("index");
        let view = PGraphView::from_lowered(&index, &lowered);
        let msg = maximal_structure_with_regime(&lowered, &Canonical);
        let cost = AdditiveCost::new(&lowered, None);
        let abb = BoundedAbb::new(Abb::new(&Canonical, 0), &cost);
        let firing = abb.solve_on(&view, &lowered, &msg).expect("propagate");
        // Equal traces mean the same cut at every node of the same traversal.
        assert_eq!(firing.as_ref().map(trace), abb.solve(&lowered, &msg).as_ref().map(trace), "{body}");
        cuts += firing.map_or(0, |s| s.pruned_by_reachability);
    }
    assert!(cuts > 0);
}

#[test]
fn multi_input_and_requires_all_inputs() {
    let (prog, ms) = compile_isa(MULTI_AND);
    let lowered = lower_of(&prog, &ms);
    let prod = producible(&prog.ir, &lowered);
    // A, B (raws) and C (produced, both inputs available) are producible.
    assert_eq!(prod.len(), 3, "A,B,C producible: {prod:?}");
    assert_eq!(
        prod,
        close_producible(&lowered, &lowered.units, &lowered.raws)
    );
}

#[test]
fn multi_input_and_missing_one_input_blocks() {
    // Only A raw; B is an intermediate with no producer -> U cannot fire -> C not producible.
    let body = "    A: + <isa> raw {}\n    B: + <isa> inter {}\n    C: + <isa> product {}\n    @U { (+C, -A, -B); }";
    let (prog, ms) = compile_isa(body);
    let lowered = lower_of(&prog, &ms);
    let prod = producible(&prog.ir, &lowered);
    assert!(
        !prod.iter().any(|d| lowered.decl_to_name[d] == "C"),
        "C must NOT be producible: {prod:?}"
    );
    assert_eq!(
        prod,
        close_producible(&lowered, &lowered.units, &lowered.raws)
    );
}

#[test]
fn multi_or_either_producer_suffices() {
    let (prog, ms) = compile_isa(MULTI_OR);
    let lowered = lower_of(&prog, &ms);
    let prod = producible(&prog.ir, &lowered);
    assert!(
        prod.iter().any(|d| lowered.decl_to_name[d] == "M"),
        "M producible via either unit"
    );
    assert_eq!(
        prod,
        close_producible(&lowered, &lowered.units, &lowered.raws)
    );
}

#[test]
fn cycle_without_raw_terminates_and_produces_nothing() {
    let (prog, ms) = compile_isa(CYCLE_NO_RAW);
    let lowered = lower_of(&prog, &ms);
    let prod = producible(&prog.ir, &lowered); // MUST terminate (finite Boolean fixpoint)
    assert!(
        prod.is_empty(),
        "a raw-less cycle produces nothing: {prod:?}"
    );
    assert_eq!(
        prod,
        close_producible(&lowered, &lowered.units, &lowered.raws)
    );
}

#[test]
fn duplicate_input_incidence_is_semantic_and_boolean_idempotent() {
    let (prog, ms) = compile_isa(DUP_INPUT);
    let lowered = lower_of(&prog, &ms);
    let prod = producible(&prog.ir, &lowered);
    assert_eq!(
        prod,
        close_producible(&lowered, &lowered.units, &lowered.raws)
    );
    let index = build_incidence_index(&prog.ir).unwrap();
    let view = PGraphView::from_lowered(&index, &lowered);
    let u = *lowered.units.iter().next().unwrap();
    assert_eq!(
        view.in_degree(u),
        2,
        "duplicate input incidence is observable (multiplicity 2)"
    );
    assert_eq!(
        view.inputs_set(u).len(),
        1,
        "but the deduped input set is one material"
    );
}

#[test]
fn empty_graph_produces_only_raws() {
    let body = "    A: + <isa> raw {}\n    B: + <isa> inter {}";
    let (prog, ms) = compile_isa(body);
    let lowered = lower_of(&prog, &ms);
    let prod = producible(&prog.ir, &lowered);
    assert_eq!(
        prod,
        close_producible(&lowered, &lowered.units, &lowered.raws)
    );
    assert!(
        prod.iter().all(|d| lowered.raws.contains(d)),
        "only raws are available with no units"
    );
}

/// Reliability out of `[0,1]` drives availability past 1 under Product -> typed BadValue.
struct OverReliability;
impl Reliability for OverReliability {
    fn of(&self, _u: DeclId) -> f64 {
        2.0
    }
}

#[test]
fn out_of_range_value_is_rejected() {
    let (prog, ms) = compile_isa(MULTI_AND);
    let lowered = lower_of(&prog, &ms);
    let index = build_incidence_index(&prog.ir).unwrap();
    let view = PGraphView::from_lowered(&index, &lowered);
    let mut ws = AndOrWorkspace::new();
    let r = propagate(
        &view,
        &Product,
        &OverReliability,
        &mut ws,
        PropagationConfig::fuzzy(1e-9, 10_000),
    );
    assert!(
        matches!(r, Err(NormError::BadValue { .. })),
        "out-of-range availability must be rejected"
    );
}

#[test]
fn non_convergent_when_iteration_budget_too_small() {
    let (prog, ms) = compile_isa(CHAIN);
    let lowered = lower_of(&prog, &ms);
    let index = build_incidence_index(&prog.ir).unwrap();
    let view = PGraphView::from_lowered(&index, &lowered);
    let mut ws = AndOrWorkspace::new();
    // The 3-stage chain needs several worklist steps; a budget of 1 forces NonConvergent.
    let r = propagate(
        &view,
        &Product,
        &UnitReliability,
        &mut ws,
        PropagationConfig::fuzzy(1e-12, 1),
    );
    assert!(matches!(r, Err(NormError::NonConvergent { .. })));
}

#[test]
fn neutral_unit_ref_is_rejected() {
    let body = "    A: + <isa> raw {}\n    B: + <isa> product {}\n    @U { (+B, ~A); }";
    let src = format!("{HEADER}body {{\n{body}\n}}\n");
    let provider = MemProvider::default()
        .with_file("m.hymeko", src.as_str())
        .with_file("meta_pgraph.hymeko", META);
    let mut ms = ModuleStore::new(provider, RealParser);
    let prog = match ms.compile(std::path::Path::new("m.hymeko")) {
        Ok(p) => p,
        Err(_) => return, // rejected at parse/resolve — acceptable
    };
    match lower_resolved(&prog.ir, &ms.it) {
        Err(_) => {} // rejected at lowering (NeutralRef) — expected
        Ok(lowered) => {
            let index = build_incidence_index(&prog.ir).unwrap();
            let view = PGraphView::from_lowered(&index, &lowered);
            assert!(matches!(
                view.validate(),
                Err(ViewError::NeutralUnitRef { .. })
            ));
        }
    }
}

#[test]
fn norm_identities_hold() {
    let b = BooleanExact;
    assert_eq!(b.t(0.4, b.t_identity()), 0.4);
    assert_eq!(b.s(0.4, b.s_identity()), 0.4);
}

#[test]
fn fuzzy_norms_are_ordering_only_same_producible_set() {
    // With full reliability, Godel/Product/Lukasiewicz give avail=1 exactly where BooleanExact does, so the
    // producible SET is norm-invariant (fuzzy modes are ordering-only; they never change the exact set).
    for body in [MULTI_AND, MULTI_OR, CHAIN, DUP_INPUT, CYCLE_NO_RAW] {
        let (prog, ms) = compile_isa(body);
        let lowered = lower_of(&prog, &ms);
        let index = build_incidence_index(&prog.ir).unwrap();
        let view = PGraphView::from_lowered(&index, &lowered);
        let mut ws = AndOrWorkspace::new();
        let exact = {
            let cfg =
                PropagationConfig::boolean_exact(lowered.units.len(), lowered.materials.len());
            propagate(&view, &BooleanExact, &UnitReliability, &mut ws, cfg)
                .unwrap()
                .producible()
                .clone()
        };
        let fcfg = PropagationConfig::fuzzy(1e-12, 1_000_000);
        let g = propagate(&view, &Godel, &UnitReliability, &mut ws, fcfg)
            .unwrap()
            .producible()
            .clone();
        let p = propagate(&view, &Product, &UnitReliability, &mut ws, fcfg)
            .unwrap()
            .producible()
            .clone();
        let l = propagate(&view, &Lukasiewicz, &UnitReliability, &mut ws, fcfg)
            .unwrap()
            .producible()
            .clone();
        assert_eq!(g, exact, "Godel changed the exact set for {body}");
        assert_eq!(p, exact, "Product changed the exact set for {body}");
        assert_eq!(l, exact, "Lukasiewicz changed the exact set for {body}");
    }
}
