//! SBSH over a REAL compiled IR — `.hymeko` source -> RealParser -> ModuleStore -> canonical `Ir`.
//!
//! Not a hand-built fixture struct: the point of this crate is that it overlays the canonical IR,
//! so the tests compile actual HyMeKo source and overlay that.

use std::collections::BTreeMap;
use std::path::Path;

use hymeko::common::ids::DeclId;
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::Ir;
use hymeko::module_store::module_store::ModuleStore;
use hymeko::module_store::source_provider::MemProvider;
use hymeko::util::real_parser::RealParser;
use hymeko_graph::direct_ir::PositionProvider;
use hymeko_sbsh::{OrthConfig, SbshScene, SceneError};

/// Positions supplied by name, the way a scene binding would supply them from a pose table.
struct ByName {
    by_decl: BTreeMap<DeclId, Vec<f64>>,
}

impl PositionProvider for ByName {
    fn position(&self, node: DeclId) -> Option<&[f64]> {
        self.by_decl.get(&node).map(|v| v.as_slice())
    }
}

struct World {
    store: ModuleStore<MemProvider, RealParser>,
    prog: std::sync::Arc<hymeko::module_store::module_store::CompiledProgram>,
}

fn compile(src: &str) -> World {
    let provider = MemProvider::default().with_file("/m.hymeko", src);
    let mut store = ModuleStore::new(provider, RealParser);
    let prog = store
        .compile(Path::new("/m.hymeko"))
        .expect("fixture must compile");
    World { store, prog }
}

impl World {
    fn ir(&self) -> &Ir {
        &self.prog.ir
    }
    /// The `DeclId` of a leaf name.
    fn decl(&self, name: &str) -> DeclId {
        let sym = self
            .store
            .it
            .get_id(name)
            .unwrap_or_else(|| panic!("no symbol {name}"));
        let ir = self.ir();
        (0..ir.decl_nodes.len())
            .find(|&i| ir.decl_nodes[i].name == sym)
            .map(DeclId::new)
            .unwrap_or_else(|| panic!("no decl named {name}"))
    }
}

/// Six objects on a line, joined by two binary arcs and one ternary arc.
const SRC: &str = r#"
vv {}
g { a {} b {} c {} d {} e {} f {}
    @e1 { (+ a, + b); }
    @e2 { (+ b, + c); }
    @e3 { (+ d, + e, ~ f); }
}
"#;

fn scene_of(w: &World, coords: &[(&str, [f64; 2])], max_per_cell: usize) -> SbshScene {
    let by_decl = coords
        .iter()
        .map(|(n, p)| (w.decl(n), p.to_vec()))
        .collect();
    let provider = ByName { by_decl };
    let index = build_incidence_index(w.ir()).expect("index builds");
    SbshScene::build(
        &index,
        &provider,
        max_per_cell,
        &OrthConfig {
            max_depth: 6,
            drop_empty: true,
        },
    )
    .expect("scene builds")
}

#[test]
fn the_partition_covers_every_positioned_declaration_exactly_once() {
    let w = compile(SRC);
    let coords = [
        ("a", [0.0, 0.0]),
        ("b", [1.0, 0.0]),
        ("c", [2.0, 0.0]),
        ("d", [8.0, 8.0]),
        ("e", [9.0, 8.0]),
        ("f", [9.0, 9.0]),
    ];
    let scene = scene_of(&w, &coords, 2);

    assert_eq!(scene.dim(), 2);
    assert_eq!(scene.decls().len(), 6);
    scene
        .tree()
        .verify_laminar()
        .expect("the spatial family must be laminar");

    let mut seen: BTreeMap<DeclId, usize> = BTreeMap::new();
    for block in scene.patches() {
        for d in block {
            *seen.entry(d).or_default() += 1;
        }
    }
    assert_eq!(
        seen.len(),
        6,
        "every positioned declaration is in the partition"
    );
    assert!(seen.values().all(|&k| k == 1), "patches must be disjoint");
}

#[test]
fn an_unpositioned_declaration_is_excluded_rather_than_placed_at_the_origin() {
    // `f` is deliberately unpositioned. Silently defaulting it to the origin would put it in a
    // patch it has no business being in, and would corrupt both the partition and the width.
    let w = compile(SRC);
    let coords = [
        ("a", [0.0, 0.0]),
        ("b", [1.0, 0.0]),
        ("c", [2.0, 0.0]),
        ("d", [3.0, 0.0]),
        ("e", [4.0, 0.0]),
    ];
    let scene = scene_of(&w, &coords, 2);

    assert_eq!(scene.decls().len(), 5);
    assert!(
        scene.point_of(w.decl("f")).is_none(),
        "unpositioned decl must have no point"
    );
    assert!(
        scene.patch_of(w.decl("f")).is_none(),
        "unpositioned decl is in no patch"
    );
    assert!(scene.point_of(w.decl("a")).is_some());
}

#[test]
fn a_scene_with_no_positions_is_an_error_not_an_empty_tree() {
    let w = compile(SRC);
    let provider = ByName {
        by_decl: BTreeMap::new(),
    };
    let index = build_incidence_index(w.ir()).expect("index");
    let err = SbshScene::build(&index, &provider, 2, &OrthConfig::default()).unwrap_err();
    assert_eq!(err, SceneError::NoPositionedDeclarations);
}

#[test]
fn ragged_position_dimensions_are_refused() {
    let w = compile(SRC);
    let mut by_decl = BTreeMap::new();
    by_decl.insert(w.decl("a"), vec![0.0, 0.0]);
    by_decl.insert(w.decl("b"), vec![1.0, 0.0, 3.0]); // 3-D among 2-D
    let provider = ByName { by_decl };
    let index = build_incidence_index(w.ir()).expect("index");
    let err = SbshScene::build(&index, &provider, 2, &OrthConfig::default()).unwrap_err();
    assert_eq!(
        err,
        SceneError::RaggedDimension {
            expected: 2,
            found: 3
        }
    );
}

#[test]
fn the_primal_graph_cliques_each_hyperarc_and_the_width_follows() {
    let w = compile(SRC);
    let coords = [
        ("a", [0.0, 0.0]),
        ("b", [1.0, 0.0]),
        ("c", [2.0, 0.0]),
        ("d", [8.0, 8.0]),
        ("e", [9.0, 8.0]),
        ("f", [9.0, 9.0]),
    ];
    let scene = scene_of(&w, &coords, 2);
    let index = build_incidence_index(w.ir()).expect("index");

    let adj = scene.primal_adjacency(&index);
    // e1 and e2 are binary (1 edge each); e3 is ternary and induces a 3-clique. Summing the
    // adjacency's degrees counts every undirected edge twice: 2 * (1 + 1 + 3) = 10.
    let half: usize = adj.iter().map(|n| n.len()).sum();
    assert_eq!(
        half, 10,
        "2 binary arcs + one 3-clique = 5 undirected edges, got adj {adj:?}"
    );

    // a-b-c is a chain (width 1); d,e,f is a triangle (width 2); the scene is their disjoint
    // union, so the certificate is the max, 2.
    assert_eq!(scene.width(&index), 2);
}

#[test]
fn a_ternary_arc_is_not_flattened_to_a_single_pair() {
    // The `~` incidence in e3 participates like any other: signs never gate structure.
    let w = compile(SRC);
    let coords = [("d", [0.0, 0.0]), ("e", [1.0, 0.0]), ("f", [0.0, 1.0])];
    let scene = scene_of(&w, &coords, 4);
    let index = build_incidence_index(w.ir()).expect("index");
    let adj = scene.primal_adjacency(&index);
    let half: usize = adj.iter().map(|n| n.len()).sum();
    assert_eq!(
        half, 6,
        "a 3-member arc must induce a 3-clique (6 half-edges), got {adj:?}"
    );
    assert_eq!(scene.width(&index), 2, "a triangle certifies at width 2");
}

#[test]
fn a_region_query_returns_the_declarations_in_the_box() {
    let w = compile(SRC);
    let coords = [
        ("a", [0.0, 0.0]),
        ("b", [1.0, 0.0]),
        ("c", [2.0, 0.0]),
        ("d", [8.0, 8.0]),
        ("e", [9.0, 8.0]),
        ("f", [9.0, 9.0]),
    ];
    let scene = scene_of(&w, &coords, 2);
    let near = scene.objects_in_region(&[-0.5, -0.5], &[2.5, 0.5]);
    let want = [w.decl("a"), w.decl("b"), w.decl("c")];
    let mut got = near.clone();
    got.sort();
    let mut expect = want.to_vec();
    expect.sort();
    assert_eq!(
        got, expect,
        "the near cluster, and nothing from the far one"
    );
}
