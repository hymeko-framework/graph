//! Export small, complete instances of the measured families for paper figures.
//! Topology generation remains in the existing Fixture -> Core IrConstruction path.
//! This example performs no timing or profiling and adds no graph representation.

#[path = "../tests/pagerank_support/mod.rs"]
mod fixtures;

use fixtures::{Fixture, Membership};
use hymeko::common::ids::NodeId;
use hymeko::generators::{GraphGenerator, HypergraphGenerator};
use hymeko::ir::incidence::build_incidence_index;
use hymeko_graph::direct_ir::{HyperTraversal, TraversalWorkspace, connected_components};
use hymeko_graph::{PageRank, PageRankConfig, PageRankModel, PageRankWorkspace};
use std::io::{BufWriter, Write};

struct GalleryCase {
    id: String,
    fixture: Fixture,
}

impl GalleryCase {
    fn cases() -> Vec<Self> {
        let seed = 0x51EED;
        [
            ("chain65k", 12, 11, 2, Membership::Chain),
            ("hub32_1m", 20, 12, 4, Membership::Hub),
            (
                "components8_1m",
                40,
                16,
                3,
                Membership::Communities {
                    groups: 8,
                    active: 32,
                },
            ),
            (
                "steiner255",
                9,
                12,
                3,
                Membership::Core(HypergraphGenerator::SteinerTriple { n: 9 }),
            ),
            (
                "complete32_3",
                5,
                10,
                3,
                Membership::Core(HypergraphGenerator::CompleteUniform { n: 5, r: 3 }),
            ),
            (
                "sunflower1024",
                16,
                5,
                4,
                Membership::Core(HypergraphGenerator::Sunflower {
                    petals: 5,
                    core: 1,
                    petal: 3,
                }),
            ),
            (
                "mycielski7",
                11,
                20,
                2,
                Membership::Mycielski { iterations: 1 },
            ),
            ("rank2_1m", 16, 32, 2, Membership::Affine),
        ]
        .into_iter()
        .map(|(id, nodes, arcs, arity, membership)| Self {
            id: id.to_owned(),
            fixture: Fixture {
                nodes,
                arcs,
                arity,
                seed,
                relabel_seed: None,
                membership,
            },
        })
        .collect()
    }

    fn write(&self, out: &mut impl Write) -> Result<(), Box<dyn std::error::Error>> {
        let ir = self.fixture.generate()?;
        let index = build_incidence_index(&ir)?;
        index.validate()?;
        let components = connected_components(&index, &mut TraversalWorkspace::new())?;
        let incidences: usize = ir.arcs.iter().map(|arc| arc.refs.len()).sum();
        assert_eq!(ir.nodes.len(), self.fixture.nodes);
        assert_eq!(ir.arcs.len(), self.fixture.arcs);
        assert_eq!(incidences, self.fixture.incidence_count());
        let rank = PageRank::build(&ir, PageRankModel::default())?;
        let mut workspace = PageRankWorkspace::new();
        let config = PageRankConfig::default();
        let result = rank.run(config, &mut workspace, None)?;
        assert!(result.progress.converged);
        assert!((result.scores.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        let traversal = HyperTraversal::new(&index);
        let mut search = TraversalWorkspace::new();
        let mut reachable: Vec<_> = traversal
            .reachable_nodes(&mut search, NodeId::new(0))?
            .nodes()
            .map(|node| node.raw())
            .collect();
        reachable.sort_unstable();
        let hops = traversal
            .berge_shortest_path(&mut search, NodeId::new(0), NodeId::new(ir.nodes.len() - 1))?
            .map_or_else(
                || "null".to_owned(),
                |path| path.hyperedge_len().to_string(),
            );
        write!(
            out,
            "{{\"family\":\"{}\",\"nodes\":{},\"arcs\":{},\"incidences\":{},\"seed\":{},\"components\":{},\"labels\":{:?},\"pagerank\":{:?},\"damping\":{},\"reachable\":{reachable:?},\"shortest_hops\":{hops},\"memberships\":[",
            self.id,
            ir.nodes.len(),
            ir.arcs.len(),
            incidences,
            self.fixture.seed,
            components.count(),
            components.labels(),
            result.scores,
            config.damping
        )?;
        for (i, arc) in ir.arcs.iter().enumerate() {
            if i != 0 {
                write!(out, ",")?;
            }
            let members: Vec<_> = arc
                .refs
                .iter()
                .map(|reference| ir.as_node(reference.target()).expect("node target").raw())
                .collect();
            write!(out, "{members:?}")?;
        }
        write!(out, "]}}")?;
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .ok_or("usage: render_fixture_gallery output.json")?;
    let names: Vec<_> = args
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    let (scope, cases) = if names.is_empty() {
        (
            "small complete instances, not the timed graph sizes",
            GalleryCase::cases(),
        )
    } else {
        (
            "named benchmark fixtures at their declared full sizes",
            names
                .into_iter()
                .map(|id| {
                    let fixture = Fixture::named(&id);
                    GalleryCase { id, fixture }
                })
                .collect(),
        )
    };
    let mut out = BufWriter::new(std::fs::File::create(path)?);
    write!(
        out,
        "{{\"representation\":\"canonical Core Ir\",\"scope\":\"{scope}\",\"cases\":["
    )?;
    for (i, case) in cases.iter().enumerate() {
        if i != 0 {
            write!(out, ",")?;
        }
        case.write(&mut out)?;
    }
    writeln!(out, "]}}")?;
    out.flush()?;
    Ok(())
}
