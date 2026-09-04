//! Structurally typed, benchmark-only connectivity representations.

use std::mem::size_of;

use hymeko::ir::incidence::{IncidenceIndex, build_incidence_index};
use hymeko::ir::ir::Ir;
use hymeko_graph::{GraphicMatroidWorkspace, MatroidRef, levi_components};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepresentationKind {
    CanonicalIndex,
    ImplicitLevi,
    SparseStar,
    Clique2Section,
}

impl RepresentationKind {
    pub const ALL: [Self; 4] = [
        Self::CanonicalIndex,
        Self::ImplicitLevi,
        Self::SparseStar,
        Self::Clique2Section,
    ];

    pub const fn key(self) -> &'static str {
        match self {
            Self::CanonicalIndex => "canonical_index",
            Self::ImplicitLevi => "implicit_levi",
            Self::SparseStar => "sparse_star",
            Self::Clique2Section => "clique_2section",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == value)
    }
}

pub(crate) struct ControlDsu {
    parent: Vec<u32>,
    rank: Vec<u32>,
    seen_generation: Vec<u32>,
    generation: u32,
}

impl ControlDsu {
    fn new(n: usize) -> Self {
        assert!(u32::try_from(n).is_ok(), "control coordinate overflow");
        Self {
            parent: (0..n as u32).collect(),
            rank: vec![0; n],
            seen_generation: vec![0; n],
            generation: 0,
        }
    }

    fn reset(&mut self) {
        for (i, parent) in self.parent.iter_mut().enumerate() {
            *parent = i as u32;
        }
        self.rank.fill(0);
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.seen_generation.fill(0);
            self.generation = 1;
        }
    }

    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let parent = self.parent[x as usize];
            let grandparent = self.parent[parent as usize];
            self.parent[x as usize] = grandparent;
            x = grandparent;
        }
        x
    }

    fn union(&mut self, a: u32, b: u32) {
        let (mut a, mut b) = (self.find(a), self.find(b));
        if a == b {
            return;
        }
        if self.rank[a as usize] < self.rank[b as usize] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b as usize] = a;
        if self.rank[a as usize] == self.rank[b as usize] {
            self.rank[a as usize] += 1;
        }
    }

    fn count_distinct_roots(&mut self, coordinates: usize) -> usize {
        let mut count = 0;
        for coordinate in 0..coordinates as u32 {
            let root = self.find(coordinate) as usize;
            if self.seen_generation[root] != self.generation {
                self.seen_generation[root] = self.generation;
                count += 1;
            }
        }
        count
    }

    fn retained_bytes(&self) -> usize {
        (self.parent.capacity() + self.rank.capacity() + self.seen_generation.capacity())
            * size_of::<u32>()
    }
}

pub(crate) struct SparseStar {
    row: Vec<u32>,
    column: Vec<u32>,
    node_count: usize,
}

impl SparseStar {
    fn build(ir: &Ir) -> Self {
        let node_count = ir.nodes.len();
        let vertex_count = node_count + ir.arcs.len();
        assert!(
            u32::try_from(vertex_count).is_ok(),
            "star coordinate overflow"
        );
        let mut degree = vec![0u32; vertex_count + 1];
        for (arc, record) in ir.arcs.iter().enumerate() {
            for incidence in &record.refs {
                let node = ir
                    .as_node(incidence.target())
                    .expect("benchmark workload contains only node incidences")
                    .raw();
                degree[node] += 1;
                degree[node_count + arc] += 1;
            }
        }
        let mut row = vec![0u32; vertex_count + 1];
        for vertex in 0..vertex_count {
            row[vertex + 1] = row[vertex] + degree[vertex];
        }
        let mut cursor = row.clone();
        let mut column = vec![0u32; row[vertex_count] as usize];
        for (arc, record) in ir.arcs.iter().enumerate() {
            for incidence in &record.refs {
                let node = ir
                    .as_node(incidence.target())
                    .expect("node incidence")
                    .raw();
                let arc_vertex = node_count + arc;
                column[cursor[node] as usize] = arc_vertex as u32;
                cursor[node] += 1;
                column[cursor[arc_vertex] as usize] = node as u32;
                cursor[arc_vertex] += 1;
            }
        }
        Self {
            row,
            column,
            node_count,
        }
    }

    fn retained_bytes(&self) -> usize {
        (self.row.capacity() + self.column.capacity()) * size_of::<u32>()
    }
}

pub(crate) struct Clique2Section {
    edges: Vec<(u32, u32)>,
    node_count: usize,
}

impl Clique2Section {
    fn build(ir: &Ir) -> Self {
        assert!(
            u32::try_from(ir.nodes.len()).is_ok(),
            "clique coordinate overflow"
        );
        let mut edges = Vec::new();
        for record in &ir.arcs {
            let members: Vec<u32> = record
                .refs
                .iter()
                .map(|incidence| {
                    ir.as_node(incidence.target())
                        .expect("benchmark workload contains only node incidences")
                        .raw() as u32
                })
                .collect();
            for left in 0..members.len() {
                for right in left + 1..members.len() {
                    edges.push((members[left], members[right]));
                }
            }
        }
        Self {
            edges,
            node_count: ir.nodes.len(),
        }
    }

    fn retained_bytes(&self) -> usize {
        self.edges.capacity() * size_of::<(u32, u32)>()
    }
}

pub struct RetainedBytes {
    pub representation: usize,
    pub workspace: Option<usize>,
}

pub enum PreparedRepresentation<'ir> {
    Canonical {
        index: IncidenceIndex<'ir>,
        workspace: ControlDsu,
    },
    Levi {
        index: IncidenceIndex<'ir>,
        workspace: GraphicMatroidWorkspace,
        zero_incidence_arcs: usize,
    },
    Star {
        representation: SparseStar,
        workspace: ControlDsu,
    },
    Clique {
        representation: Clique2Section,
        workspace: ControlDsu,
    },
}

impl<'ir> PreparedRepresentation<'ir> {
    /// Build the representation and execute one first query so reusable scratch is prepared.
    pub fn build(kind: RepresentationKind, ir: &'ir Ir) -> (Self, usize) {
        let mut prepared = match kind {
            RepresentationKind::CanonicalIndex => Self::Canonical {
                index: build_incidence_index(ir).expect("canonical index"),
                workspace: ControlDsu::new(ir.nodes.len()),
            },
            RepresentationKind::ImplicitLevi => Self::Levi {
                index: build_incidence_index(ir).expect("canonical index"),
                workspace: GraphicMatroidWorkspace::new(),
                zero_incidence_arcs: ir.arcs.iter().filter(|arc| arc.refs.is_empty()).count(),
            },
            RepresentationKind::SparseStar => {
                let representation = SparseStar::build(ir);
                let workspace = ControlDsu::new(representation.row.len() - 1);
                Self::Star {
                    representation,
                    workspace,
                }
            }
            RepresentationKind::Clique2Section => {
                let representation = Clique2Section::build(ir);
                let workspace = ControlDsu::new(representation.node_count);
                Self::Clique {
                    representation,
                    workspace,
                }
            }
        };
        let components = prepared.query();
        (prepared, components)
    }

    pub fn query(&mut self) -> usize {
        match self {
            Self::Canonical { index, workspace } => {
                let ir = index.ir();
                workspace.reset();
                for record in &ir.arcs {
                    let mut members = record.refs.iter().map(|incidence| {
                        ir.as_node(incidence.target())
                            .expect("benchmark workload contains only node incidences")
                            .raw() as u32
                    });
                    if let Some(anchor) = members.next() {
                        for member in members {
                            workspace.union(anchor, member);
                        }
                    }
                }
                workspace.count_distinct_roots(ir.nodes.len())
            }
            Self::Levi {
                index,
                workspace,
                zero_incidence_arcs,
            } => {
                levi_components(MatroidRef::new(index), workspace).expect("Levi components")
                    - *zero_incidence_arcs
            }
            Self::Star {
                representation,
                workspace,
            } => {
                workspace.reset();
                for vertex in 0..representation.row.len() - 1 {
                    for offset in representation.row[vertex]..representation.row[vertex + 1] {
                        workspace.union(vertex as u32, representation.column[offset as usize]);
                    }
                }
                workspace.count_distinct_roots(representation.node_count)
            }
            Self::Clique {
                representation,
                workspace,
            } => {
                workspace.reset();
                for &(left, right) in &representation.edges {
                    workspace.union(left, right);
                }
                workspace.count_distinct_roots(representation.node_count)
            }
        }
    }

    pub fn retained_bytes(&self) -> RetainedBytes {
        match self {
            Self::Canonical { index, workspace } => RetainedBytes {
                representation: index.retained_bytes(),
                workspace: Some(workspace.retained_bytes()),
            },
            Self::Levi {
                index, workspace, ..
            } => RetainedBytes {
                representation: index.retained_bytes(),
                workspace: Some(workspace.retained_bytes()),
            },
            Self::Star {
                representation,
                workspace,
            } => RetainedBytes {
                representation: representation.retained_bytes(),
                workspace: Some(workspace.retained_bytes()),
            },
            Self::Clique {
                representation,
                workspace,
            } => RetainedBytes {
                representation: representation.retained_bytes(),
                workspace: Some(workspace.retained_bytes()),
            },
        }
    }

    pub const fn semantic_loss(kind: RepresentationKind) -> &'static str {
        match kind {
            RepresentationKind::CanonicalIndex => "none",
            RepresentationKind::ImplicitLevi => "signs,weights,paths,incidence payload",
            RepresentationKind::SparseStar => "signs,weights,exact incidence slots",
            RepresentationKind::Clique2Section => {
                "hyperarc identity,cardinality,signs,weights,exact incidence slots"
            }
        }
    }
}
