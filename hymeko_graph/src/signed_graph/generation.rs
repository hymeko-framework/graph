use super::{GraphGenerator, SignedGraph, SyntheticGenerator};
use std::convert::Infallible;

/// Seeded signed simple-graph fixture generator, in lexicographic edge order.
/// Uses the established 32-bit LCG and per-mille probability thresholds.
/// Generation costs O(n²) trials and O(E) retained storage; no hard-size bound.
#[derive(Clone, Copy, Debug)]
pub struct SignedGraphGenerator {
    nodes: u32,
    edge_permille: u32,
    positive_permille: u32,
    seed: u32,
}

impl SignedGraphGenerator {
    /// Set graph size, probabilities in per mille and an explicit seed.
    /// # Panics
    /// Panics if either probability exceeds 1000. Zero nodes are valid.
    pub fn new(nodes: u32, edge_permille: u32, positive_permille: u32, seed: u32) -> Self {
        assert!(
            edge_permille <= 1000 && positive_permille <= 1000,
            "probabilities must be in 0..=1000 per mille"
        );
        Self {
            nodes,
            edge_permille,
            positive_permille,
            seed,
        }
    }
}

impl GraphGenerator for SignedGraphGenerator {
    type Output = SignedGraph;
    type Error = Infallible;

    fn generate(&self) -> Result<SignedGraph, Infallible> {
        let mut state = self.seed;
        let mut next = || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            state
        };
        let mut graph = SignedGraph {
            n_nodes: self.nodes,
            edges: Vec::new(),
            signs: Vec::new(),
        };
        for u in 0..self.nodes {
            for v in (u + 1)..self.nodes {
                if next() % 1_000_000 < self.edge_permille * 1000 {
                    graph.edges.push((u, v));
                    graph
                        .signs
                        .push(if next() % 1_000_000 < self.positive_permille * 1000 {
                            1
                        } else {
                            -1
                        });
                }
            }
        }
        Ok(graph)
    }
}

impl SyntheticGenerator for SignedGraphGenerator {
    type Seed = u32;
    fn seed(&self) -> u32 {
        self.seed
    }
    fn reseed(&mut self, seed: u32) {
        self.seed = seed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_graphs_match_pre_refactor_known_answer() {
        // Captured from the old Rust function before removal: 108 configurations.
        let mut hash = 0xcbf29ce484222325u64;
        for n in [0, 1, 6, 17] {
            for edge in [0, 500, 1000] {
                for positive in [0, 700, 1000] {
                    for seed in [0, 42, u32::MAX] {
                        let g = SignedGraphGenerator::new(n, edge, positive, seed)
                            .generate()
                            .unwrap();
                        for b in g
                            .n_nodes
                            .to_le_bytes()
                            .into_iter()
                            .chain((g.edges.len() as u32).to_le_bytes())
                            .chain(g.edges.iter().flat_map(|&(u, v)| {
                                u.to_le_bytes().into_iter().chain(v.to_le_bytes())
                            }))
                            .chain(g.signs.iter().map(|&s| s as u8))
                        {
                            hash = (hash ^ u64::from(b)).wrapping_mul(0x100000001b3);
                        }
                    }
                }
            }
        }
        assert_eq!(hash, 0x96a213c36abbd77a);
    }

    #[test]
    fn seed_control_replays_without_consuming_generator_state() {
        let mut generator = SignedGraphGenerator::new(12, 500, 700, 42);
        let before = generator.generate().unwrap();
        assert_eq!(generator.seed(), 42);
        generator.reseed(71);
        assert_eq!(generator.seed(), 71);
        assert_ne!(before.edges, generator.generate().unwrap().edges);
        generator.reseed(42);
        let after = generator.generate().unwrap();
        assert_eq!((before.edges, before.signs), (after.edges, after.signs));
    }

    #[test]
    #[should_panic(expected = "probabilities must")]
    fn invalid_probability_is_rejected_at_construction() {
        SignedGraphGenerator::new(10, 1001, 500, 0);
    }
}
