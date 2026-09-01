//! s-connected components: components of the s-line graph, where two hyperarcs are s-adjacent iff
//! they share at least `s` distinct nodes.
//!
//! No s-line graph is materialised as an adjacency structure: a union-find over arcs is fed
//! directly from the incidence. For `s == 1` (share any node) each node simply unions its incident
//! arcs -- `O(incidences · α)`. For `s >= 2` the shared-node count per arc pair is accumulated by
//! visiting, at each node, the distinct arcs incident to it and bumping each unordered pair; this is
//! the inherent `O(Σ_node distinct_deg(node)²)` cost of the s-line and is hub-sensitive -- documented,
//! not hidden.

use std::collections::HashMap;

use hymeko::common::ids::{DeclId, HyperArcId};
use hymeko::ir::incidence::IncidenceIndex;

/// A labeling of every hyperarc into its s-connected component, for a fixed `s`.
#[derive(Debug, Clone)]
pub struct SComponentLabeling {
    label_of: Vec<u32>,
    count: u32,
    s: usize,
}

impl SComponentLabeling {
    /// The `s` this labeling was computed for.
    #[must_use]
    pub fn s(&self) -> usize {
        self.s
    }

    /// Number of s-connected components.
    #[must_use]
    pub fn count(&self) -> u32 {
        self.count
    }

    /// The component id of `arc`, or `None` if the id is out of range.
    #[must_use]
    pub fn component_of(&self, arc: HyperArcId) -> Option<u32> {
        self.label_of.get(arc.raw()).copied()
    }

    /// Whether two arcs are s-connected (`None` if either id is out of range).
    #[must_use]
    pub fn same_component(&self, a: HyperArcId, b: HyperArcId) -> Option<bool> {
        Some(self.component_of(a)? == self.component_of(b)?)
    }

    /// The per-arc label slice, indexed by `HyperArcId`.
    #[must_use]
    pub fn labels(&self) -> &[u32] {
        &self.label_of
    }
}

/// Disjoint-set union with path halving and union by rank.
struct Dsu {
    parent: Vec<u32>,
    rank: Vec<u8>,
}

impl Dsu {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n as u32).collect(),
            rank: vec![0; n],
        }
    }

    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let gp = self.parent[self.parent[x as usize] as usize];
            self.parent[x as usize] = gp; // path halving
            x = gp;
        }
        x
    }

    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        match self.rank[ra as usize].cmp(&self.rank[rb as usize]) {
            std::cmp::Ordering::Less => self.parent[ra as usize] = rb,
            std::cmp::Ordering::Greater => self.parent[rb as usize] = ra,
            std::cmp::Ordering::Equal => {
                self.parent[rb as usize] = ra;
                self.rank[ra as usize] += 1;
            }
        }
    }
}

/// Partition the hyperarcs into s-connected components (adjacent iff they share `>= s` distinct
/// nodes). `s` is clamped to at least `1`.
#[must_use]
pub fn s_connected_components(index: &IncidenceIndex<'_>, s: usize) -> SComponentLabeling {
    let ir = index.ir();
    let n_arcs = ir.arcs.len();
    let s = s.max(1);
    let mut dsu = Dsu::new(n_arcs);

    if s == 1 {
        // Any shared node unions the incident arcs.
        for d in 0..ir.decl_nodes.len() {
            let arcs = index.incident_arcs(DeclId::new(d));
            if let Some((&first, rest)) = arcs.split_first() {
                for &a in rest {
                    dsu.union(first.raw() as u32, a.raw() as u32);
                }
            }
        }
    } else {
        // Count distinct shared nodes per unordered arc pair, then union pairs reaching s.
        let mut pair_count: HashMap<(u32, u32), u32> = HashMap::new();
        let mut at_node: Vec<u32> = Vec::new();
        for d in 0..ir.decl_nodes.len() {
            at_node.clear();
            at_node.extend(
                index
                    .incident_arcs(DeclId::new(d))
                    .iter()
                    .map(|a| a.raw() as u32),
            );
            at_node.sort_unstable();
            at_node.dedup(); // distinct arcs at this node
            for i in 0..at_node.len() {
                for j in (i + 1)..at_node.len() {
                    *pair_count.entry((at_node[i], at_node[j])).or_insert(0) += 1;
                }
            }
        }
        for (&(a, b), &c) in &pair_count {
            if c as usize >= s {
                dsu.union(a, b);
            }
        }
    }

    // Dense-label by root.
    let mut label_of = vec![0u32; n_arcs];
    let mut root_to_label: HashMap<u32, u32> = HashMap::new();
    let mut count = 0u32;
    for (a, slot) in label_of.iter_mut().enumerate() {
        let root = dsu.find(a as u32);
        let label = *root_to_label.entry(root).or_insert_with(|| {
            let l = count;
            count += 1;
            l
        });
        *slot = label;
    }

    SComponentLabeling { label_of, count, s }
}
