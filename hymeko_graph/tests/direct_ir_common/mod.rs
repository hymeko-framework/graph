//! Shared source-first harness for the `direct_ir` integration tests.
//!
//! `*.hymeko` text -> `RealParser` -> `ModuleStore::compile` -> canonical `Ir`. Every direct-Ir
//! test resolves nodes/edges by their (globally-unique, in these fixtures) leaf names and builds an
//! `IncidenceIndex` over `world.ir()`.
#![allow(dead_code)] // shared across test binaries; not every binary uses every helper.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use hymeko::common::ids::{DeclId, HyperArcId, NodeId};
use hymeko::ir::ir::Ir;
use hymeko::module_store::module_store::{CompiledProgram, ModuleStore};
use hymeko::module_store::source_provider::MemProvider;
use hymeko::util::real_parser::RealParser;

pub struct World {
    store: ModuleStore<MemProvider, RealParser>,
    prog: Arc<CompiledProgram>,
}

/// Compile HyMeKo source text into a `World` (canonical `Ir` + interner), through the real parser.
pub fn compile(src: &str) -> World {
    let provider = MemProvider::default().with_file("/m.hymeko", src);
    let mut store = ModuleStore::new(provider, RealParser);
    let prog = store
        .compile(Path::new("/m.hymeko"))
        .unwrap_or_else(|e| panic!("fixture must compile: {e:?}"));
    World { store, prog }
}

impl World {
    pub fn ir(&self) -> &Ir {
        &self.prog.ir
    }

    /// Resolve a node by its leaf name.
    pub fn node(&self, name: &str) -> NodeId {
        let sym = self
            .store
            .it
            .get_id(name)
            .unwrap_or_else(|| panic!("no interned symbol {name}"));
        let ir = self.ir();
        (0..ir.nodes.len())
            .find(|&i| ir.decl_nodes[ir.nodes[i].decl.raw()].name == sym)
            .map(NodeId::new)
            .unwrap_or_else(|| panic!("no node named {name}"))
    }

    /// The first arc of an edge, by the edge's leaf name.
    pub fn edge_arc(&self, name: &str) -> HyperArcId {
        let sym = self
            .store
            .it
            .get_id(name)
            .unwrap_or_else(|| panic!("no interned symbol {name}"));
        let ir = self.ir();
        ir.edges
            .iter()
            .find(|e| ir.decl_nodes[e.decl.raw()].name == sym)
            .and_then(|e| e.arcs.first().copied())
            .unwrap_or_else(|| panic!("no edge named {name} with an arc"))
    }
}

/// Independent reachability oracle: a fixpoint over **every arc** (`O(V·E)`), touching only
/// `ir.arcs[*].refs`, never the `IncidenceIndex`. A hyperarc connects all its members.
pub fn oracle_reachable(ir: &Ir, start: NodeId) -> BTreeSet<usize> {
    let start_decl = ir.nodes[start.raw()].decl;
    let mut seen: BTreeSet<usize> = BTreeSet::new();
    seen.insert(start_decl.raw());
    let mut changed = true;
    while changed {
        changed = false;
        for arc in &ir.arcs {
            let members: Vec<DeclId> = arc.refs.iter().map(|r| r.target()).collect();
            if members.iter().any(|m| seen.contains(&m.raw())) {
                for m in members {
                    if seen.insert(m.raw()) {
                        changed = true;
                    }
                }
            }
        }
    }
    seen.iter()
        .filter_map(|&d| ir.as_node(DeclId::new(d)))
        .map(|n| n.raw())
        .collect()
}
