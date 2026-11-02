//! Input adapters only: canonical population is owned by Core IrConstruction.
#![allow(dead_code)] // Shared by numerical, allocation and opt-in evidence clients.

use hymeko::common::ids::{DeclId, SymId};
use hymeko::generators::{
    GraphGenerator, HypergraphDesign, HypergraphGenerator, SyntheticGenerator, binom, mycielskian,
};
use hymeko::ir::ir::{
    AnnoR, ArcPlacement, IncidenceSign, Ir, IrBuilder, IrCapacities, IrConstruction,
    ReferenceDraft, dispatch,
};
use hymeko_graph::rand_lcg::Lcg;

#[derive(Clone)]
pub struct Case {
    pub nodes: usize,
    pub arcs: Vec<Vec<ReferenceDraft>>,
}
impl Case {
    pub fn unsigned(nodes: usize, arcs: &[&[usize]]) -> Self {
        Self {
            nodes,
            arcs: arcs
                .iter()
                .map(|members| {
                    members
                        .iter()
                        .map(|&node| {
                            ReferenceDraft::plain(DeclId::new(node), IncidenceSign::Neutral)
                        })
                        .collect()
                })
                .collect(),
        }
    }
}
impl IrBuilder for Case {
    type Output = Ir;
    type Error = dispatch::DispatchError;
    fn capacities(&self) -> IrCapacities {
        IrCapacities {
            declarations: self.nodes + 2 * self.arcs.len(),
            nodes: self.nodes,
            edges: self.arcs.len(),
            arcs: self.arcs.len(),
        }
    }
    fn populate(&mut self, construction: &mut IrConstruction<'_>) -> Result<(), Self::Error> {
        for _ in 0..self.nodes {
            construction.append_node(SymId::new(0), AnnoR::default(), vec![])?;
        }
        for refs in self.arcs.drain(..) {
            let edge = construction.append_edge(SymId::new(0), AnnoR::default(), vec![])?;
            construction.append_arc(ArcPlacement::under_owner(edge), AnnoR::default(), refs)?;
        }
        Ok(())
    }
    fn finish(self, ir: Ir) -> Ir {
        ir
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Membership {
    Core(HypergraphGenerator),
    Mycielski { iterations: u32 },
    Affine,
    Disjoint,
    Mixed(&'static [usize]),
    Hub,
    Communities { groups: usize, active: usize },
    Chain,
}
impl Membership {
    pub fn label(self) -> &'static str {
        match self {
            Self::Core(_) => "core_combinatorial",
            Self::Mycielski { .. } => "core_mycielskian",
            Self::Affine => "affine",
            Self::Disjoint => "disjoint",
            Self::Mixed(_) => "mixed_affine",
            Self::Hub => "universal_hub_cyclic_leaves",
            Self::Communities { .. } => "overlapping_windows_in_disjoint_regions",
            Self::Chain => "chain",
        }
    }
}

/// Streaming input strategies over the same canonical builder. Affine rows use
/// distinct members for a power-of-two domain and odd stride. These are explicit
/// deterministic workloads, not a claim of real-world or uniform random graphs.
#[derive(Clone, Copy, Debug)]
pub struct Fixture {
    pub nodes: usize,
    pub arcs: usize,
    /// Fixed width, or the maximum permitted width for a mixed strategy.
    pub arity: usize,
    pub seed: u64,
    pub relabel_seed: Option<u64>,
    pub membership: Membership,
}

/// Test input coordinate bijection; population remains in the existing builder.
pub struct VertexPermutation {
    targets: Option<Vec<usize>>,
}
impl VertexPermutation {
    pub fn new(nodes: usize, seed: Option<u64>) -> Self {
        let targets = seed.map(|seed| {
            let mut targets: Vec<_> = (0..nodes).collect();
            let mut rng = Lcg::new(seed);
            for last in (1..nodes).rev() {
                let range = last as u64 + 1;
                let threshold = range.wrapping_neg() % range;
                let chosen = loop {
                    let value = rng.next_u64();
                    if value >= threshold {
                        break (value % range) as usize;
                    }
                };
                targets.swap(last, chosen);
            }
            targets
        });
        Self { targets }
    }
    pub fn target(&self, original: usize) -> usize {
        self.targets
            .as_ref()
            .map_or(original, |targets| targets[original])
    }
}

impl Fixture {
    pub fn incidence_count(&self) -> usize {
        self.validate();
        if let Membership::Mixed(widths) = self.membership {
            (self.arcs / widths.len()) * widths.iter().sum::<usize>()
                + widths[..self.arcs % widths.len()].iter().sum::<usize>()
        } else {
            self.arcs * self.arity
        }
    }
    pub fn named(name: &str) -> Self {
        let (name, relabel_seed) = match name.split_once('@') {
            Some((name, seed)) => (name, Some(seed.parse().expect("numeric permutation seed"))),
            None => (name, None),
        };
        let (nodes, arcs, arity, membership) = match name {
            "steiner255" => (
                255,
                10_795,
                3,
                Membership::Core(HypergraphGenerator::SteinerTriple { n: 255 }),
            ),
            "complete32_3" => (
                32,
                4_960,
                3,
                Membership::Core(HypergraphGenerator::CompleteUniform { n: 32, r: 3 }),
            ),
            "sunflower1024" => (
                32_769,
                1_024,
                33,
                Membership::Core(HypergraphGenerator::Sunflower {
                    petals: 1_024,
                    core: 1,
                    petal: 32,
                }),
            ),
            "mycielski7" => (767, 22_196, 2, Membership::Mycielski { iterations: 7 }),
            "rank2_1m" => (65_536, 500_000, 2, Membership::Affine),
            "rank32_1m" => (65_536, 31_250, 32, Membership::Affine),
            "rank1000_1m" => (65_536, 1_000, 1_000, Membership::Affine),
            "disjoint_1m" => (1_000_000, 1_000, 1_000, Membership::Disjoint),
            "mixed_1m" => (65_536, 31_250, 130, Membership::Mixed(&[2, 4, 8, 16, 130])),
            "hub32_1m" => (65_536, 31_250, 32, Membership::Hub),
            "components8_1m" => (
                65_536,
                31_250,
                32,
                Membership::Communities {
                    groups: 8,
                    active: 32_768,
                },
            ),
            "chain65k" => (65_536, 65_535, 2, Membership::Chain),
            "scale_small" => (8_192, 4_096, 32, Membership::Affine),
            "scale_large" => (262_144, 125_000, 32, Membership::Affine),
            _ => panic!("unknown ranking fixture"),
        };
        Self {
            nodes,
            arcs,
            arity,
            membership,
            seed: 0x51EED,
            relabel_seed,
        }
    }

    fn design(&self) -> Option<HypergraphDesign> {
        match self.membership {
            Membership::Core(generator) => Some(generator.generate().expect("valid Core fixture")),
            Membership::Mycielski { iterations } => {
                let mut design = HypergraphDesign {
                    n_vertices: 5,
                    edges: vec![vec![0, 1], vec![1, 2], vec![2, 3], vec![3, 4], vec![4, 0]],
                };
                for _ in 0..iterations {
                    design = mycielskian(&design);
                }
                Some(design)
            }
            _ => None,
        }
    }
    fn append_rows<R: IntoIterator<Item = usize>>(
        &self,
        construction: &mut IrConstruction<'_>,
        rows: impl IntoIterator<Item = R>,
        permutation: &VertexPermutation,
    ) -> Result<(), dispatch::DispatchError> {
        for row in rows {
            let edge = construction.append_edge(SymId::new(0), AnnoR::default(), vec![])?;
            let refs = row
                .into_iter()
                .map(|target| {
                    ReferenceDraft::plain(
                        DeclId::new(permutation.target(target)),
                        IncidenceSign::Neutral,
                    )
                })
                .collect();
            construction.append_arc(ArcPlacement::under_owner(edge), AnnoR::default(), refs)?;
        }
        Ok(())
    }

    fn validate(&self) {
        assert!(self.nodes > 0 && self.arity <= self.nodes);
        match self.membership {
            Membership::Core(generator) => {
                let expected = match generator {
                    HypergraphGenerator::SteinerTriple { n } => {
                        assert!(n >= 7 && matches!(n % 6, 1 | 3));
                        (n, n * (n - 1) / 6, 3)
                    }
                    HypergraphGenerator::Sunflower {
                        petals,
                        core,
                        petal,
                    } => {
                        assert!(petals > 0 && petal > 0);
                        (core + petals * petal, petals, core + petal)
                    }
                    HypergraphGenerator::CompleteUniform { n, r } => {
                        assert!((2..=n).contains(&r));
                        (
                            n,
                            usize::try_from(binom(n, r)).expect("fixture edge count"),
                            r,
                        )
                    }
                    HypergraphGenerator::Hypercube { dimension } => {
                        assert!(dimension > 0);
                        let nodes = 2usize
                            .checked_pow(u32::try_from(dimension).expect("fixture dimension"))
                            .expect("fixture vertex count");
                        let arcs = (nodes / 2)
                            .checked_mul(dimension)
                            .expect("fixture edge count");
                        (nodes, arcs, 2)
                    }
                };
                assert_eq!((self.nodes, self.arcs, self.arity), expected);
            }
            Membership::Mycielski { iterations } => {
                assert!(iterations <= 8, "bounded fixture generation");
                let (mut nodes, mut arcs) = (5, 5);
                for _ in 0..iterations {
                    (nodes, arcs) = (2 * nodes + 1, 3 * arcs + nodes);
                }
                assert_eq!((self.nodes, self.arcs, self.arity), (nodes, arcs, 2));
            }
            Membership::Affine => assert!(self.nodes.is_power_of_two()),
            Membership::Mixed(widths) => {
                assert!(self.nodes.is_power_of_two() && !widths.is_empty());
                assert!(
                    widths
                        .iter()
                        .all(|&width| (2..=self.arity).contains(&width))
                );
            }
            Membership::Disjoint => assert_eq!(self.arcs * self.arity, self.nodes),
            Membership::Hub => assert!(self.arity >= 2),
            Membership::Communities { groups, active } => {
                assert!(groups > 0 && active > 0 && active <= self.nodes);
                assert!(
                    active.is_multiple_of(groups) && (2..=active / groups).contains(&self.arity)
                );
            }
            Membership::Chain => {
                assert_eq!(self.arity, 2);
                assert_eq!(self.arcs + 1, self.nodes);
            }
        }
    }

    fn row(&self, ordinal: usize, rng: &mut Lcg) -> MemberRow {
        let mut row = MemberRow {
            width: self.arity,
            base: 0,
            domain: self.nodes,
            start: 0,
            stride: 1,
            hub: None,
        };
        match self.membership {
            Membership::Core(_) | Membership::Mycielski { .. } => {
                unreachable!("Core designs supply explicit rows")
            }
            Membership::Affine | Membership::Mixed(_) => {
                // Keep the original affine RNG sequence exactly unchanged.
                row.start = (rng.next_u64() >> 24) as usize % self.nodes;
                row.stride = ((rng.next_u64() >> 24) as usize % self.nodes) | 1;
                if let Membership::Mixed(widths) = self.membership {
                    row.width = widths[ordinal % widths.len()];
                }
            }
            Membership::Disjoint => row.start = ordinal * self.arity,
            Membership::Hub => {
                row.base = 1;
                row.domain = self.nodes - 1;
                row.start =
                    (self.seed as usize % row.domain + ordinal * (self.arity - 1)) % row.domain;
                row.hub = Some(0);
            }
            Membership::Communities { groups, active } => {
                row.domain = active / groups;
                row.base = (ordinal % groups) * row.domain;
                row.start = (self.seed as usize % row.domain
                    + (ordinal / groups) * (self.arity - 1))
                    % row.domain;
            }
            Membership::Chain => row.start = ordinal,
        }
        row
    }
}

/// One row of input coordinates only; IR population remains IrConstruction's.
struct MemberRow {
    width: usize,
    base: usize,
    domain: usize,
    start: usize,
    stride: usize,
    hub: Option<usize>,
}
impl MemberRow {
    fn targets(self) -> impl Iterator<Item = usize> {
        (0..self.width).map(move |slot| {
            if let Some(hub) = self.hub {
                if slot == 0 {
                    return hub;
                }
                self.base + (self.start + (slot - 1) * self.stride) % self.domain
            } else {
                self.base + (self.start + slot * self.stride) % self.domain
            }
        })
    }
}
impl IrBuilder for Fixture {
    type Output = Ir;
    type Error = dispatch::DispatchError;
    fn capacities(&self) -> IrCapacities {
        IrCapacities {
            declarations: self.nodes + 2 * self.arcs,
            nodes: self.nodes,
            edges: self.arcs,
            arcs: self.arcs,
        }
    }
    fn populate(&mut self, construction: &mut IrConstruction<'_>) -> Result<(), Self::Error> {
        self.validate();
        for _ in 0..self.nodes {
            construction.append_node(SymId::new(0), AnnoR::default(), vec![])?;
        }
        let mut rng = Lcg::new(self.seed);
        let permutation = VertexPermutation::new(self.nodes, self.relabel_seed);
        if let Some(design) = self.design() {
            self.append_rows(construction, design.edges, &permutation)?;
        } else {
            self.append_rows(
                construction,
                (0..self.arcs).map(|ordinal| self.row(ordinal, &mut rng).targets()),
                &permutation,
            )?;
        }
        Ok(())
    }
    fn finish(self, ir: Ir) -> Ir {
        ir
    }
}
impl GraphGenerator for Fixture {
    type Output = Ir;
    type Error = dispatch::DispatchError;
    fn generate(&self) -> Result<Ir, Self::Error> {
        (*self).build()
    }
}
impl SyntheticGenerator for Fixture {
    type Seed = u64;
    fn seed(&self) -> u64 {
        self.seed
    }
    fn reseed(&mut self, seed: u64) {
        self.seed = seed;
    }
}
