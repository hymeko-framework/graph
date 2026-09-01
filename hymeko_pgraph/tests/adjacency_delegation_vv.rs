//! P1.3 V&V: `PGraphView` (borrowed, Ir-based) is the equivalent of the schema's single directed
//! adjacency — reachability + degree parity — so the axiom-layer duplication removal is sound and the
//! delegation target is validated. Anchors the §1.5 fault sweep (mutating `view.rs` fails a named test
//! here). Adjacency-set parity (inputs/outputs/producers/consumers) is already proven in `andor_vv`.

use std::collections::BTreeSet;
use std::sync::Arc;

use hymeko::common::ids::DeclId;
use hymeko::ir::incidence::build_incidence_index;
use hymeko::module_store::module_store::{CompiledProgram, ModuleStore};
use hymeko::module_store::source_provider::MemProvider;
use hymeko::util::real_parser::RealParser;

use hymeko_pgraph::lowering::LoweredPGraph;
use hymeko_pgraph::meta_resolve::lower_resolved;
use hymeko_pgraph::schema::PGraphSchema;
use hymeko_pgraph::view::PGraphView;

const META: &str = include_str!("../data/meta_pgraph.hymeko");
const HEADER: &str = "R {\n    @\"meta_pgraph.hymeko\";\n    using pgraph.raw as raw;\n    using pgraph.product as product;\n    using pgraph.intermediate as inter;\n    using pgraph.process as process;\n}\n";

fn compile(body: &str) -> (Arc<CompiledProgram>, ModuleStore<MemProvider, RealParser>) {
    let src = format!("{HEADER}body {{\n{body}\n}}\n");
    let provider = MemProvider::default()
        .with_file("m.hymeko", src.as_str())
        .with_file("meta_pgraph.hymeko", META);
    let mut ms = ModuleStore::new(provider, RealParser);
    let prog = ms
        .compile(std::path::Path::new("m.hymeko"))
        .unwrap_or_else(|e| panic!("compile: {e:?}"));
    (prog, ms)
}

/// Directed reachability over the schema's single adjacency (BFS over `successors`) — the reference the
/// removed `axioms/axiom_extensions::reaches_any` computed, now expressed via the public accessor.
fn schema_reaches(schema: &PGraphSchema, from: DeclId, targets: &BTreeSet<DeclId>) -> bool {
    let mut seen = BTreeSet::new();
    let mut stack = vec![from];
    seen.insert(from);
    while let Some(v) = stack.pop() {
        for &n in schema.successors(v) {
            if targets.contains(&n) {
                return true;
            }
            if seen.insert(n) {
                stack.push(n);
            }
        }
    }
    false
}

const BODIES: &[&str] = &[
    "    A: + <isa> raw {}\n    B: + <isa> raw {}\n    C: + <isa> product {}\n    @U { (+C, -A, -B); }",
    "    A: + <isa> raw {}\n    B: + <isa> raw {}\n    M: + <isa> product {}\n    @U1 { (+M, -A); }\n    @U2 { (+M, -B); }",
    "    A: + <isa> raw {}\n    B: + <isa> inter {}\n    C: + <isa> inter {}\n    D: + <isa> product {}\n    @U1 { (+B, -A); }\n    @U2 { (+C, -B); }\n    @U3 { (+D, -C); }",
    "    A: + <isa> raw {}\n    C: + <isa> product {}\n    @U { (+C, -A, -A); }",
];

fn each<'a>(
    prog: &'a CompiledProgram,
    ms: &'a ModuleStore<MemProvider, RealParser>,
) -> LoweredPGraph {
    lower_resolved(&prog.ir, &ms.it).unwrap()
}

#[test]
fn pgraphview_forward_reachable_matches_schema_directed_reachability() {
    for body in BODIES {
        let (prog, ms) = compile(body);
        let lowered = each(&prog, &ms);
        let index = build_incidence_index(&prog.ir).unwrap();
        let view = PGraphView::from_lowered(&index, &lowered);

        // For every operating unit, "reaches a required product" agrees between the Ir-based view and the
        // schema's single adjacency — the exact A4 predicate.
        for &o in &lowered.units {
            let via_view = view
                .forward_reachable(o)
                .iter()
                .filter(|&&d| d != o)
                .any(|d| lowered.products.contains(d));
            let via_schema = schema_reaches(&lowered.schema, o, &lowered.products);
            assert_eq!(
                via_view,
                via_schema,
                "reachability disagrees for unit {}",
                o.raw()
            );
        }
        // And from every material.
        for &m in &lowered.materials {
            let via_view = view
                .forward_reachable(m)
                .iter()
                .filter(|&&d| d != m)
                .any(|d| lowered.products.contains(d));
            let via_schema = schema_reaches(&lowered.schema, m, &lowered.products);
            assert_eq!(
                via_view,
                via_schema,
                "reachability disagrees for material {}",
                m.raw()
            );
        }
    }
}

#[test]
fn pgraphview_degree_matches_schema_degree_with_multiplicity() {
    for body in BODIES {
        let (prog, ms) = compile(body);
        let lowered = each(&prog, &ms);
        let index = build_incidence_index(&prog.ir).unwrap();
        let view = PGraphView::from_lowered(&index, &lowered);
        for &u in &lowered.units {
            assert_eq!(
                view.in_degree(u),
                lowered.schema.in_degree(u),
                "in_degree unit {}",
                u.raw()
            );
            assert_eq!(
                view.out_degree(u),
                lowered.schema.out_degree(u),
                "out_degree unit {}",
                u.raw()
            );
        }
    }
}

#[test]
fn duplicate_input_preserved_as_multiplicity() {
    // The duplicate-`-A` graph: the deduped input set is {A} but the degree (multiplicity) is 2.
    let (prog, ms) =
        compile("    A: + <isa> raw {}\n    C: + <isa> product {}\n    @U { (+C, -A, -A); }");
    let lowered = each(&prog, &ms);
    let index = build_incidence_index(&prog.ir).unwrap();
    let view = PGraphView::from_lowered(&index, &lowered);
    let u = *lowered.units.iter().next().unwrap();
    assert_eq!(view.inputs_set(u).len(), 1);
    assert_eq!(
        view.in_degree(u),
        2,
        "duplicate incidence multiplicity preserved"
    );
}
