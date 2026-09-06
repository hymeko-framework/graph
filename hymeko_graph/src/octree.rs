//! Octree subdivision read from the canonical G-SPHF IR.
//!
//! Adapts the parent-plus-children representation recovered from the 2026-08-08
//! mobile-nav backup (SHA-256 1db636bdcadb409b7026dccd75224b6dae624cd9fc32c46316a3b41d7e8a439e).
//! Core declarations own identity and subdivision; these arrays are a validated
//! numerical projection. Unlike the navigation policy, no mixed volume is dropped.
use crate::sphf::{Role, SignedHypergraph};
use hymeko::{
    common::ids::DeclId,
    ir::ir::{Ir, ValueR},
    resolution::interner::Interner,
};
use std::collections::{HashMap, HashSet};

/// One authored axis-aligned cubic cell; identity belongs to Core.
#[derive(Clone, Debug)]
pub struct Cell {
    /// Canonical declaration.
    pub decl: DeclId,
    /// Inclusive lower bound in metres.
    pub lo: [f64; 3],
    /// Exclusive upper bound in metres.
    pub hi: [f64; 3],
    /// Subdivision depth.
    pub depth: usize,
    /// Indexes into the projected cell array, not persistent identities.
    pub children: Vec<usize>,
}
impl Cell {
    /// Geometric volume.
    pub fn volume(&self) -> f64 {
        (0..3).map(|k| self.hi[k] - self.lo[k]).product()
    }
    /// World-space centre.
    pub fn center(&self) -> [f64; 3] {
        std::array::from_fn(|k| (self.lo[k] + self.hi[k]) * 0.5)
    }
}

/// A validated, volume-covering, 2:1-balanced octree over existing IR declarations.
#[derive(Clone, Debug)]
pub struct Octree {
    /// All subdivision nodes, including internal organizers.
    pub cells: Vec<Cell>,
    /// Unique root index.
    pub root: usize,
    /// Only these cells carry independent physical state.
    pub leaves: Vec<usize>,
}
impl Octree {
    /// Read `lo`/`hi` cell bounds and `kind "subdivision"` signed hyperedges.
    /// A subdivision has one negative parent and exactly eight positive children.
    pub fn from_ir(ir: &Ir, it: &Interner, scope: DeclId) -> Result<Self, String> {
        let graph = SignedHypergraph::within(ir, Some(scope));
        let mut cells = Vec::new();
        for &decl in &graph.vertices {
            if field(ir, it, decl, "lo").is_none() {
                continue;
            }
            let lo = vector(ir, it, decl, "lo")?;
            let hi = vector(ir, it, decl, "hi")?;
            let side = hi[0] - lo[0];
            if !side.is_finite()
                || side <= 0.0
                || (0..3).any(|k| {
                    !lo[k].is_finite()
                        || !hi[k].is_finite()
                        || ((hi[k] - lo[k]) - side).abs() > side * 1e-12
                })
            {
                return Err("octree requires positive finite cubic bounds".into());
            }
            if !side.powi(3).is_finite() || side.powi(3) <= 0.0 {
                return Err("cell volume is not representable".into());
            }
            cells.push(Cell {
                decl,
                lo,
                hi,
                depth: 0,
                children: Vec::new(),
            });
            if cells.len() > 4096 {
                return Err("octree supports at most 4096 nodes".into());
            }
        }
        if cells.is_empty() {
            return Err("octree contains no cells".into());
        }
        let index: HashMap<_, _> = cells.iter().enumerate().map(|(i, c)| (c.decl, i)).collect();
        let mut parent = vec![None; cells.len()];
        for edge in &graph.edges {
            if !matches!(field(ir,it,edge.decl,"kind"), Some(ValueR::Str(s)) if it.resolve(*s)=="subdivision")
            {
                continue;
            }
            let sources: Vec<_> = edge
                .incidences
                .iter()
                .filter(|i| i.role == Role::Source)
                .collect();
            if edge.incidences.len() != 9
                || sources.len() != 1
                || edge
                    .incidences
                    .iter()
                    .any(|i| i.role == Role::Neutral || i.weight != 1.0)
            {
                return Err(
                    "subdivision requires one parent and eight children with unit incidence".into(),
                );
            }
            let p = *index
                .get(&graph.vertices[sources[0].vertex])
                .ok_or("subdivision parent is not a cell")?;
            if !cells[p].children.is_empty() {
                return Err("duplicate subdivision".into());
            }
            for incidence in edge.incidences.iter().filter(|i| i.role == Role::Target) {
                let child = *index
                    .get(&graph.vertices[incidence.vertex])
                    .ok_or("subdivision child is not a cell")?;
                if parent[child].replace(p).is_some() || child == p {
                    return Err("cell has repeated or cyclic parent".into());
                }
                cells[p].children.push(child);
            }
            Self::validate_children(&cells, p)?;
        }
        let roots: Vec<_> = parent
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.is_none().then_some(i))
            .collect();
        if roots.len() != 1 {
            return Err("octree requires exactly one root".into());
        }
        let root = roots[0];
        let mut stack = vec![(root, 0)];
        let mut seen = HashSet::new();
        let mut leaves = Vec::new();
        while let Some((i, depth)) = stack.pop() {
            if !seen.insert(i) || depth > 16 {
                return Err("cyclic or excessively deep subdivision".into());
            }
            cells[i].depth = depth;
            if cells[i].children.is_empty() {
                leaves.push(i);
            }
            for &child in cells[i].children.iter().rev() {
                stack.push((child, depth + 1));
            }
        }
        if seen.len() != cells.len() || leaves.len() > 1024 {
            return Err("disconnected or oversized octree".into());
        }
        let out = Self {
            cells,
            root,
            leaves,
        };
        for (i, j, _, _) in out.faces() {
            if out.cells[out.leaves[i]]
                .depth
                .abs_diff(out.cells[out.leaves[j]].depth)
                > 1
            {
                return Err("octree must be 2:1 balanced across faces".into());
            }
        }
        Ok(out)
    }
    fn validate_children(cells: &[Cell], p: usize) -> Result<(), String> {
        let parent = &cells[p];
        let mid = parent.center();
        let side = parent.hi[0] - parent.lo[0];
        let mut octants = HashSet::new();
        for &child in &parent.children {
            let c = &cells[child];
            let mut code = 0;
            for k in 0..3 {
                let upper = (c.lo[k] - mid[k]).abs() <= side * 1e-12;
                let (lo, hi) = if upper {
                    (mid[k], parent.hi[k])
                } else {
                    (parent.lo[k], mid[k])
                };
                if (c.lo[k] - lo).abs() > side * 1e-12 || (c.hi[k] - hi).abs() > side * 1e-12 {
                    return Err("child does not occupy a parent octant".into());
                }
                if upper {
                    code |= 1 << k;
                }
            }
            if !octants.insert(code) {
                return Err("overlapping octants".into());
            }
        }
        Ok(())
    }
    /// Shared subfaces: leaf indexes, area and normal centre distance.
    /// One coarse face naturally yields four independently conserved subfaces.
    /// Bounded pair scan for the initial <=1024-leaf demonstrator.
    pub fn faces(&self) -> Vec<(usize, usize, f64, f64)> {
        let mut out = Vec::new();
        let root = &self.cells[self.root];
        let eps = (root.hi[0] - root.lo[0]) * 1e-12;
        for (i, &a) in self.leaves.iter().enumerate() {
            for (j, &b) in self.leaves.iter().enumerate().skip(i + 1) {
                let (a, b) = (&self.cells[a], &self.cells[b]);
                for axis in 0..3 {
                    if (a.hi[axis] - b.lo[axis]).abs() > eps
                        && (b.hi[axis] - a.lo[axis]).abs() > eps
                    {
                        continue;
                    }
                    let mut area = 1.0;
                    for k in (0..3).filter(|&k| k != axis) {
                        area *= (a.hi[k].min(b.hi[k]) - a.lo[k].max(b.lo[k])).max(0.0);
                    }
                    if area > 0.0 {
                        out.push((
                            i,
                            j,
                            area,
                            (a.hi[axis] - a.lo[axis] + b.hi[axis] - b.lo[axis]) * 0.5,
                        ));
                    }
                }
            }
        }
        out
    }
}

/// Borrow an authored attribute using the existing Core child iterator.
pub fn field<'a>(ir: &'a Ir, it: &Interner, decl: DeclId, name: &str) -> Option<&'a ValueR> {
    ir.decl_children(decl)
        .find(|&id| it.resolve(ir.decl_nodes[id.raw()].name) == name)
        .and_then(|id| ir.decl_nodes[id.raw()].anno.value.as_ref())
}
fn vector(ir: &Ir, it: &Interner, decl: DeclId, name: &str) -> Result<[f64; 3], String> {
    match field(ir, it, decl, name) {
        Some(ValueR::List(v)) if v.len() == 3 => {
            let mut out = [0.0; 3];
            for (i, x) in v.iter().enumerate() {
                if let ValueR::Num(n) = x {
                    out[i] = *n;
                } else {
                    return Err(format!("{name} must be numeric"));
                }
            }
            Ok(out)
        }
        _ => Err(format!("{name} must have three components")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hymeko::{
        module_store::{module_store::ModuleStore, source_provider::StdFsProvider},
        util::real_parser::RealParser,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    // Test-only source fixture authoring, using the real Core compiler.
    fn source(refine: bool) -> String {
        let mut s = String::from("fixture {}\ndomain { root { lo [0,0,0]; hi [2,2,2];\n");
        for i in 0..8 {
            let lo: [usize; 3] = std::array::from_fn(|k| (i >> (2 - k)) & 1);
            s.push_str(&format!("c{i} {{ lo {:?}; hi {:?};", lo, lo.map(|v| v + 1)));
            if refine && i == 0 {
                for j in 0..8 {
                    let lo: [f64; 3] = std::array::from_fn(|k| ((j >> (2 - k)) & 1) as f64 * 0.5);
                    s.push_str(&format!(
                        "f{j} {{ lo {:?}; hi {:?}; }}",
                        lo,
                        lo.map(|v| v + 0.5)
                    ));
                }
                s.push_str("@fine { kind \"subdivision\"; (-domain.root.c0");
                for j in 0..8 {
                    s.push_str(&format!(", +domain.root.c0.f{j}"));
                }
                s.push_str("); }");
            }
            s.push_str("}\n");
        }
        s.push_str("@split { kind \"subdivision\"; (-domain.root");
        for i in 0..8 {
            s.push_str(&format!(", +domain.root.c{i}"));
        }
        s.push_str("); } } }");
        s
    }
    fn read(text: &str) -> Result<Octree, String> {
        let path = std::env::temp_dir().join(format!(
            "hymeko-octree-{}-{}.hymeko",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, text).unwrap();
        let mut store = ModuleStore::new(StdFsProvider::new(), RealParser);
        let result = store.compile(&path).expect("test source must compile");
        std::fs::remove_file(path).unwrap();
        let scope = result
            .ir
            .decls()
            .find(|(_, n)| store.it.resolve(n.name) == "domain")
            .unwrap()
            .0;
        Octree::from_ir(&result.ir, &store.it, scope)
    }
    #[test]
    fn recovered_subdivision_covers_volume_and_resolves_coarse_fine_faces() {
        let tree = read(&source(true)).unwrap();
        assert_eq!(tree.leaves.len(), 15);
        assert_eq!(tree.cells.len(), 17);
        assert_eq!(
            tree.leaves
                .iter()
                .map(|&i| tree.cells[i].volume())
                .sum::<f64>(),
            8.0
        );
        let cross: Vec<_> = tree
            .faces()
            .into_iter()
            .filter(|&(i, j, _, _)| {
                tree.cells[tree.leaves[i]].depth != tree.cells[tree.leaves[j]].depth
            })
            .collect();
        assert_eq!(cross.len(), 12); // three faces, four quarter-faces each
        assert_eq!(cross.iter().map(|f| f.2).sum::<f64>(), 3.0);
        assert!(cross.iter().all(|f| f.3 == 0.75));
    }
    #[test]
    fn malformed_subdivision_is_rejected_instead_of_losing_volume() {
        let valid = source(false);
        assert_eq!(read(&valid).unwrap().leaves.len(), 8);
        for invalid in [
            valid.replace(", +domain.root.c7", ""),
            valid.replace("+domain.root.c7", "+domain.root.c6"),
            valid.replace("-domain.root,", "+domain.root,"),
            valid.replace("hi [2,2,2]", "hi [3,2,2]"),
            valid.replace("hi [1, 1, 1]", "hi [0, 1, 1]"),
        ] {
            assert!(read(&invalid).is_err(), "invalid subdivision was accepted");
        }
    }
}
