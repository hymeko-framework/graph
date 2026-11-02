//! P3 — Steiner / Mycielski / random workloads on the direct canonical-Ir navigation path.
//!
//! Correctness is anchored on **formulas** (independent of the emitted edge list) and on the frozen
//! oracles in `informed_common` / `direct_ir_common` (a linear-scan Dijkstra and an O(V·E) reachability
//! scan share no code with the engine). The workloads emit canonical `hymeko_core::ir::Ir` and are
//! navigated through the sparse `IncidenceIndex` — no pairwise projection, no second model. The final
//! measurement test is ignored by default and requires explicit opt-in.

use std::collections::{BTreeSet, HashMap, HashSet};

use hymeko::common::ids::NodeId;
use hymeko::ir::incidence::build_incidence_index;
use hymeko_graph::direct_ir::{
    HyperTraversal, InformedSearchWorkspace, TraversalWorkspace, Undirected, UnitCost,
    ZeroHeuristic, astar_berge_path, dijkstra_berge_path,
};

#[path = "direct_ir_common/mod.rs"]
mod dc;
#[path = "informed_common/mod.rs"]
mod ic;

use ic::OracleCost;
#[path = "pagerank_support/mod.rs"]
mod construction;

use hymeko::generators::{GraphGenerator, HypergraphDesign, HypergraphGenerator, mycielskian};
use hymeko::ir::incidence::IncidenceIndex;
use hymeko::ir::ir::IrBuilder;
use hymeko_graph::direct_ir::{
    batch_reachable_counts, connected_components, dual_reachability, par_batch_reachable_counts,
};
use hymeko_graph::matroid::{
    GraphicMatroidWorkspace, MatroidRef, levi_components, levi_independent, levi_spanning_basis,
};

/// Test client only: one generated design feeds the existing canonical builder,
/// then all algorithms see the same IR. Expected answers use the frozen oracles.
struct GeneratedAlgorithms {
    name: &'static str,
    design: HypergraphDesign,
}
impl GeneratedAlgorithms {
    fn check(&self) {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap();
        let mut original = None;
        for seed in [None, Some(17), Some(29)] {
            let permutation = construction::VertexPermutation::new(self.design.n_vertices, seed);
            let members: Vec<Vec<usize>> = self
                .design
                .edges
                .iter()
                .map(|row| row.iter().map(|&v| permutation.target(v)).collect())
                .collect();
            let ir = construction::Case::unsigned(
                self.design.n_vertices,
                &members.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            )
            .build()
            .unwrap();
            assert_eq!(ir.nodes.len(), self.design.n_vertices);
            assert_eq!(ir.edges.len(), self.design.edges.len());
            assert_eq!(ir.arcs.len(), self.design.edges.len());
            for (arc, expected) in ir.arcs.iter().zip(&members) {
                let actual: Vec<_> = arc
                    .refs
                    .iter()
                    .map(|r| ir.as_node(r.target()).unwrap().raw())
                    .collect();
                assert_eq!(
                    &actual, expected,
                    "ordered membership must survive construction"
                );
            }
            let index = build_incidence_index(&ir).unwrap();
            let starts: Vec<_> = (0..ir.nodes.len())
                .map(|v| NodeId::new(permutation.target(v)))
                .collect();
            let mut traversal = TraversalWorkspace::new();
            let labeling = connected_components(&index, &mut traversal).unwrap();
            let expected: Vec<_> = starts
                .iter()
                .map(|&s| dc::oracle_reachable(&ir, s))
                .collect();
            let partitions: BTreeSet<_> = expected.iter().cloned().collect();
            assert_eq!(
                labeling.count() as usize,
                partitions.len(),
                "{} {seed:?}",
                self.name
            );
            let counts: Vec<_> = expected.iter().map(BTreeSet::len).collect();
            assert_eq!(batch_reachable_counts(&index, &starts).unwrap(), counts);
            assert_eq!(
                pool.install(|| par_batch_reachable_counts(&index, &starts))
                    .unwrap(),
                counts
            );
            for (i, &start) in starts.iter().enumerate() {
                let reached: BTreeSet<_> = HyperTraversal::new(&index)
                    .reachable_nodes(&mut traversal, start)
                    .unwrap()
                    .nodes()
                    .map(|n| n.raw())
                    .collect();
                assert_eq!(reached, expected[i]);
                for &goal in &starts {
                    assert_eq!(
                        labeling.same_component(start, goal),
                        Some(expected[i].contains(&goal.raw()))
                    );
                }
            }
            self.check_dual_and_basis(&index, partitions.len());
            let distances = Self::check_paths(&index, &starts);
            if let Some(ref baseline) = original {
                assert_eq!(&(counts, distances), baseline, "{} {seed:?}", self.name);
            } else {
                original = Some((counts, distances));
            }
            println!(
                "ALGORITHM_CASE\tname={}\tseed={seed:?}\tvertices={}\tarcs={}\tincidences={}\tcomponents={}\tordered_pairs={}",
                self.name,
                ir.nodes.len(),
                ir.arcs.len(),
                members.iter().map(Vec::len).sum::<usize>(),
                partitions.len(),
                starts.len() * starts.len()
            );
        }
    }

    fn check_paths(index: &IncidenceIndex<'_>, starts: &[NodeId]) -> Vec<Option<usize>> {
        let traversal = HyperTraversal::new(index);
        let mut bfs = TraversalWorkspace::new();
        let mut informed = InformedSearchWorkspace::new();
        let mut distances = Vec::new();
        for &start in starts {
            for &goal in starts {
                let reference =
                    ic::oracle_dijkstra_cost(index.ir(), index, start, goal, OracleCost::Unit);
                let path = traversal
                    .berge_shortest_path(&mut bfs, start, goal)
                    .unwrap();
                assert_eq!(path.as_ref().map(|p| p.steps.len() as f64), reference);
                for use_astar in [false, true] {
                    let path = if use_astar {
                        astar_berge_path(
                            &traversal,
                            start,
                            goal,
                            &UnitCost,
                            &ZeroHeuristic,
                            &Undirected,
                            &mut informed,
                        )
                    } else {
                        dijkstra_berge_path(
                            &traversal,
                            start,
                            goal,
                            &UnitCost,
                            &Undirected,
                            &mut informed,
                        )
                    }
                    .unwrap();
                    assert_eq!(path.as_ref().map(|p| p.total_cost), reference);
                }
                distances.push(path.map(|p| p.steps.len()));
            }
        }
        distances
    }

    fn check_dual_and_basis(&self, index: &IncidenceIndex<'_>, components: usize) {
        use hymeko::common::ids::HyperArcId;
        let ir = index.ir();
        let mut traversal = TraversalWorkspace::new();
        for (start, arc) in ir.arcs.iter().enumerate() {
            let expected = if let Some(first) = arc.refs.first() {
                let nodes = dc::oracle_reachable(ir, ir.as_node(first.target()).unwrap());
                ir.arcs
                    .iter()
                    .enumerate()
                    .filter_map(|(i, arc)| {
                        arc.refs
                            .iter()
                            .any(|r| nodes.contains(&ir.as_node(r.target()).unwrap().raw()))
                            .then_some(i)
                    })
                    .collect::<BTreeSet<_>>()
            } else {
                BTreeSet::from([start])
            };
            let actual = dual_reachability(index, &mut traversal, HyperArcId::new(start)).unwrap();
            assert_eq!(
                actual
                    .arcs()
                    .iter()
                    .map(|a| a.raw())
                    .collect::<BTreeSet<_>>(),
                expected
            );
        }
        let expected_components =
            components + ir.arcs.iter().filter(|arc| arc.refs.is_empty()).count();
        let mut scratch = GraphicMatroidWorkspace::new();
        let facade = MatroidRef::new(index);
        assert_eq!(
            levi_components(facade, &mut scratch).unwrap(),
            expected_components
        );
        let mut basis = Vec::new();
        let expected_rank = ir.nodes.len() + ir.arcs.len() - expected_components;
        assert_eq!(
            levi_spanning_basis(facade, &mut scratch, &mut basis).unwrap(),
            expected_rank
        );
        assert_eq!(basis.len(), expected_rank);
        assert!(levi_independent(facade, &basis, &mut scratch).unwrap());
    }
}

#[test]
fn core_generators_feed_one_ir_across_algorithms_and_vertex_numberings() {
    let fano = HypergraphGenerator::SteinerTriple { n: 7 }
        .generate()
        .unwrap();
    let (n, edges) = ic::base_c5();
    let cycle = HypergraphDesign {
        n_vertices: n,
        edges: edges.into_iter().map(|(a, b)| vec![a, b]).collect(),
    };
    let requests = [
        (
            "hypercube3",
            HypergraphGenerator::Hypercube { dimension: 3 },
        ),
        ("fano", HypergraphGenerator::SteinerTriple { n: 7 }),
        ("bose9", HypergraphGenerator::SteinerTriple { n: 9 }),
        ("steiner13", HypergraphGenerator::SteinerTriple { n: 13 }),
        ("bose15", HypergraphGenerator::SteinerTriple { n: 15 }),
        (
            "sunflower",
            HypergraphGenerator::Sunflower {
                petals: 4,
                core: 2,
                petal: 3,
            },
        ),
        (
            "disjoint",
            HypergraphGenerator::Sunflower {
                petals: 3,
                core: 0,
                petal: 2,
            },
        ),
        (
            "complete3",
            HypergraphGenerator::CompleteUniform { n: 6, r: 3 },
        ),
    ];
    for (name, request) in requests {
        GeneratedAlgorithms {
            name,
            design: request.generate().unwrap(),
        }
        .check();
    }
    for (name, design) in [
        ("mycielski_graph", mycielskian(&mycielskian(&cycle))),
        ("mycielski_hypergraph", mycielskian(&fano)),
        (
            "empty",
            HypergraphDesign {
                n_vertices: 0,
                edges: vec![],
            },
        ),
        (
            "isolated",
            HypergraphDesign {
                n_vertices: 4,
                edges: vec![],
            },
        ),
        (
            "boundary",
            HypergraphDesign {
                n_vertices: 6,
                edges: vec![vec![], vec![0], vec![1, 1], vec![1, 2], vec![3, 4]],
            },
        ),
    ] {
        GeneratedAlgorithms { name, design }.check();
    }
}

// ---- Steiner formula verification (§4.1, §6) -------------------------------------------------------

#[test]
fn sts_counts_match_formula_independent_of_edge_list() {
    for m in 3u32..=7 {
        let (v, blocks, incidences, degree) = ic::sts_expected_counts(m);
        let ir = ic::steiner_sts_ir(m);
        assert_eq!(ir.decl_nodes.len(), v, "STS({m}) vertex count");
        assert_eq!(ir.arcs.len(), blocks, "STS({m}) block count v(v-1)/6");
        let inc: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
        assert_eq!(inc, incidences, "STS({m}) incidence count v(v-1)/2");
        // every block is a triple
        assert!(
            ir.arcs.iter().all(|a| a.refs.len() == 3),
            "STS({m}) every block size 3"
        );
        // every unordered vertex pair occurs in exactly one block
        let mut pair_count: HashMap<(usize, usize), usize> = HashMap::new();
        let mut deg = vec![0usize; v];
        for a in &ir.arcs {
            let ns: Vec<usize> = a.refs.iter().map(|r| r.target().raw()).collect();
            for &n in &ns {
                deg[n] += 1;
            }
            for i in 0..ns.len() {
                for j in (i + 1)..ns.len() {
                    let p = (ns[i].min(ns[j]), ns[i].max(ns[j]));
                    *pair_count.entry(p).or_default() += 1;
                }
            }
        }
        assert_eq!(
            pair_count.len(),
            v * (v - 1) / 2,
            "STS({m}) all pairs covered"
        );
        assert!(
            pair_count.values().all(|&c| c == 1),
            "STS({m}) each pair in exactly one block"
        );
        assert!(
            deg.iter().all(|&d| d == degree),
            "STS({m}) uniform degree (v-1)/2"
        );
    }
}

// ---- Mycielski formula + graph-property verification (§4.2, §6) ------------------------------------

#[test]
fn mycielski_recurrence_and_properties() {
    let (bn, be) = ic::base_c5();
    for iters in 0u32..=3 {
        let (exp_n, exp_e) = ic::mycielski_expected_counts(bn, be.len(), iters);
        let ir = ic::mycielski_ir(bn, &be, iters);
        let ordered_membership = ir
            .arcs
            .iter()
            .flat_map(|arc| arc.refs.iter())
            .flat_map(|reference| (reference.target().raw() as u64).to_le_bytes())
            .fold(0xcbf29ce484222325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            });
        // Frozen from the pre-delegation adapter on 2026-10-20. Checks ordered
        // endpoints, not only counts/isomorphism, independently of Core generation.
        assert_eq!(
            ordered_membership,
            [
                8829341927553824037,
                12972309819242767082,
                14714448923187843448,
                16428135619258162422,
            ][iters as usize]
        );
        assert_eq!(
            ir.decl_nodes.len(),
            exp_n,
            "Mycielski^{iters} n = 2n+1 recurrence"
        );
        assert_eq!(
            ir.arcs.len(),
            exp_e,
            "Mycielski^{iters} e = 3e+n recurrence"
        );
        let mut edges: HashSet<(usize, usize)> = HashSet::new();
        let mut adj: HashMap<usize, HashSet<usize>> = HashMap::new();
        for a in &ir.arcs {
            assert_eq!(a.refs.len(), 2, "Mycielski is rank-2");
            let (x, y) = (a.refs[0].target().raw(), a.refs[1].target().raw());
            assert_ne!(x, y, "no self-loop");
            let e = (x.min(y), x.max(y));
            assert!(edges.insert(e), "no duplicate edge {e:?}");
            adj.entry(x).or_default().insert(y);
            adj.entry(y).or_default().insert(x);
        }
        // triangle-free preserved (C5 is triangle-free; Mycielskian preserves it)
        for (&u, nbrs) in &adj {
            for &w in nbrs {
                if w <= u {
                    continue;
                }
                let common = adj
                    .get(&w)
                    .map(|s| s.intersection(nbrs).count())
                    .unwrap_or(0);
                assert_eq!(common, 0, "Mycielski^{iters} triangle-free (edge {u}-{w})");
            }
        }
    }
}

// ---- seeded random determinism (§4.4) -------------------------------------------------------------

#[test]
fn random_uniform_rank_k_is_deterministic() {
    let a = ic::uniform_rank_k_ir(0xC0FFEE, 500, 800, 3);
    let b = ic::uniform_rank_k_ir(0xC0FFEE, 500, 800, 3);
    assert_eq!(a.arcs.len(), b.arcs.len());
    let sig = |ir: &hymeko::ir::ir::Ir| -> Vec<Vec<usize>> {
        ir.arcs
            .iter()
            .map(|x| x.refs.iter().map(|r| r.target().raw()).collect())
            .collect()
    };
    assert_eq!(
        sig(&a),
        sig(&b),
        "same seed -> byte-identical arc membership"
    );
    let inc: usize = a.arcs.iter().map(|x| x.refs.len()).sum();
    assert_eq!(inc, 800 * 3, "incidences = m_edges * k");
}

// ---- oracle-verified navigation on the generated workloads (§6) -----------------------------------

fn reachable_decls(
    idx: &hymeko::ir::incidence::IncidenceIndex<'_>,
    start: NodeId,
) -> BTreeSet<usize> {
    let trav = HyperTraversal::new(idx);
    let mut ws = TraversalWorkspace::new();
    let set = trav.reachable_nodes(&mut ws, start).unwrap();
    set.decls().iter().map(|d| d.raw()).collect()
}

#[test]
fn steiner_navigation_matches_oracles() {
    let ir = ic::steiner_sts_ir(5); // STS(31): connected, every pair co-blocked
    let idx = build_incidence_index(&ir).unwrap();
    // reachability: STS is connected -> engine reaches all; equals the O(V·E) scan
    let eng = reachable_decls(&idx, NodeId::new(0));
    let orc = dc::oracle_reachable(&ir, NodeId::new(0));
    assert_eq!(eng, orc, "engine reachability == scan oracle");
    assert_eq!(eng.len(), ir.decl_nodes.len(), "STS(31) is connected");
    // shortest Berge path == oracle Dijkstra (unit cost) for several pairs
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    for (s, g) in [(0usize, 30usize), (1, 15), (7, 23), (0, 1)] {
        let (start, goal) = (NodeId::new(s), NodeId::new(g));
        let oracle = ic::oracle_dijkstra_cost(&ir, &idx, start, goal, OracleCost::Unit);
        let engine =
            dijkstra_berge_path(&trav, start, goal, &UnitCost, &Undirected, &mut ws).unwrap();
        match (oracle, engine) {
            (Some(oc), Some(p)) => assert!(
                (oc - p.total_cost).abs() < 1e-9,
                "cost {oc} vs {}",
                p.total_cost
            ),
            (None, None) => {}
            other => panic!("reachability disagreement {s}->{g}: {other:?}"),
        }
    }
}

#[test]
fn mycielski_astar_zero_equals_dijkstra_equals_oracle() {
    let (bn, be) = ic::base_c5();
    let ir = ic::mycielski_ir(bn, &be, 2); // 23 vertices
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    for (s, g) in [(0usize, 22usize), (0, 11), (5, 17), (10, 12)] {
        let (start, goal) = (NodeId::new(s), NodeId::new(g));
        let oracle = ic::oracle_dijkstra_cost(&ir, &idx, start, goal, OracleCost::Unit);
        let dij = dijkstra_berge_path(&trav, start, goal, &UnitCost, &Undirected, &mut ws).unwrap();
        let ast = astar_berge_path(
            &trav,
            start,
            goal,
            &UnitCost,
            &ZeroHeuristic,
            &Undirected,
            &mut ws,
        )
        .unwrap();
        let dc = dij.as_ref().map(|p| p.total_cost);
        let ac = ast.as_ref().map(|p| p.total_cost);
        assert_eq!(
            dc.map(|x| (x * 1e6) as i64),
            ac.map(|x| (x * 1e6) as i64),
            "A*(Zero) == Dijkstra"
        );
        match (oracle, dc) {
            (Some(oc), Some(x)) => assert!((oc - x).abs() < 1e-9),
            (None, None) => {}
            other => panic!("mycielski reachability disagreement {s}->{g}: {other:?}"),
        }
    }
}

// ---- metamorphic: unreachable target returns the typed no-path result (§6) -------------------------

#[test]
fn disconnected_target_has_no_path() {
    // two disjoint STS(7) components -> cross-component pairs are unreachable (typed None)
    let (ir, off) = ic::two_disjoint_sts_ir(3);
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    let p = dijkstra_berge_path(
        &trav,
        NodeId::new(0),
        NodeId::new(off),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .unwrap();
    assert!(
        p.is_none(),
        "cross-component target must be unreachable (typed None)"
    );
    let q = dijkstra_berge_path(
        &trav,
        NodeId::new(off),
        NodeId::new(off + 1),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .unwrap();
    assert!(q.is_some(), "within-component target reachable");
}

// ---- measurement pass (§7): cold construction vs warm search (run with --nocapture) ---------------

#[test]
#[ignore = "performance benchmark; opt in with --ignored --exact"]
fn measure_navigation_matrix() {
    use std::time::Instant;
    println!("\n# P3 direct-Ir navigation measurement (medians; warm = post-index)");
    println!(
        "{:<18} {:>7} {:>8} {:>9} {:>12} {:>12} {:>12} {:>10}",
        "workload", "V", "E", "nnz", "idx_us", "reach_us", "dijkstra_us", "B/inc"
    );
    // Steiner STS
    for m in [3u32, 5, 7] {
        let ir = ic::steiner_sts_ir(m);
        bench_row(&format!("STS(2^{m}-1)"), &ir);
    }
    // Mycielski
    for it in [2u32, 4, 6] {
        let (bn, be) = ic::base_c5();
        let ir = ic::mycielski_ir(bn, &be, it);
        bench_row(&format!("Mycielski^{it}"), &ir);
    }
    // random uniform rank-3
    for (n, e) in [(1_000usize, 2_000usize), (30_000, 90_000)] {
        let ir = ic::uniform_rank_k_ir(0x51EED, n, e, 3);
        bench_row(&format!("rand3({n})"), &ir);
    }
    let _ = Instant::now();
}

fn bench_row(name: &str, ir: &hymeko::ir::ir::Ir) {
    let v = ir.decl_nodes.len();
    let e = ir.arcs.len();
    let nnz: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
    let (idx_us, _, _) = ic::median_us(9, || {
        let _ = build_incidence_index(ir).unwrap();
    });
    let idx = build_incidence_index(ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let (reach_us, _, _) = ic::median_us(9, || {
        let mut ws = TraversalWorkspace::new();
        let _ = trav.reachable_nodes(&mut ws, NodeId::new(0)).unwrap();
    });
    let (dij_us, _, _) = ic::median_us(9, || {
        let mut ws = InformedSearchWorkspace::new();
        let _ = dijkstra_berge_path(
            &trav,
            NodeId::new(0),
            NodeId::new(v - 1),
            &UnitCost,
            &Undirected,
            &mut ws,
        )
        .unwrap();
    });
    // retained index bytes / incidence — approximate via the reported footprint if available; else nnz*est
    let bytes_per_inc = (idx.retained_bytes() as f64) / (nnz.max(1) as f64);
    println!(
        "{name:<18} {v:>7} {e:>8} {nnz:>9} {idx_us:>12.2} {reach_us:>12.2} {dij_us:>12.2} {bytes_per_inc:>10.2}"
    );
}

// ---- P3.2: STS(1023) large-instance formula validation (§5/§9 mandatory) ---------------------------

#[test]
fn sts_1023_formula_validation() {
    // STS(2^10 - 1) = STS(1023): v=1023, blocks=v(v-1)/6=174251, incidences=v(v-1)/2=522753,
    // degree=(v-1)/2=511. Predicted retained: 522753 incidences * ~fat SignedRefR — well under 16 GiB.
    let (v, blocks, incidences, degree) = ic::sts_expected_counts(10);
    assert_eq!((v, blocks, incidences, degree), (1023, 174251, 522753, 511));
    let ir = ic::steiner_sts_ir(10);
    assert_eq!(ir.decl_nodes.len(), v);
    assert_eq!(ir.arcs.len(), blocks);
    let inc: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
    assert_eq!(inc, incidences);
    assert!(ir.arcs.iter().all(|a| a.refs.len() == 3));
    // uniform degree, without the O(V^2) pair map (too large to store here; covered on small m).
    let mut deg = vec![0usize; v];
    for a in &ir.arcs {
        for r in &a.refs {
            deg[r.target().raw()] += 1;
        }
    }
    assert!(
        deg.iter().all(|&d| d == degree),
        "STS(1023) uniform degree 511"
    );
    // navigation sanity: connected (reachable from node 0 == all V).
    let idx = build_incidence_index(&ir).unwrap();
    let eng = reachable_decls(&idx, NodeId::new(0));
    assert_eq!(eng.len(), v, "STS(1023) connected");
}
