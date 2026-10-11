//! One synchronous accumulation/update, followed by stable compaction and growth.
use super::{BatchGas, GasError, StepReceipt, validate_points};
use crate::{SignedGraph, traversal::Csr};
use std::collections::BTreeSet;

pub(super) struct Batch {
    winner_delta: Vec<[f64; 3]>,
    neighbour_delta: Vec<[f64; 3]>,
    winner_count: Vec<usize>,
    neighbour_count: Vec<usize>,
    errors: Vec<f64>,
    edges: BTreeSet<(u32, u32)>,
}

impl Batch {
    pub(super) fn collect(model: &BatchGas, points: &[[f64; 3]]) -> Self {
        let n = model.positions.len();
        let mut batch = Self {
            winner_delta: vec![[0.0; 3]; n],
            neighbour_delta: vec![[0.0; 3]; n],
            winner_count: vec![0; n],
            neighbour_count: vec![0; n],
            errors: model.errors.clone(),
            edges: BTreeSet::new(),
        };
        let adjacency = Csr::from_graph(&model.graph);
        for point in points {
            let (first, second, distance) = nearest_two(&model.positions, point);
            batch.errors[first] += model.config.winner_rate * distance;
            batch.winner_count[first] += 1;
            for (axis, coordinate) in point.iter().enumerate() {
                batch.winner_delta[first][axis] += coordinate - model.positions[first][axis];
            }
            for &node in adjacency.neighbours(first as u32) {
                let node = node as usize;
                batch.neighbour_count[node] += 1;
                for (axis, coordinate) in point.iter().enumerate() {
                    batch.neighbour_delta[node][axis] += coordinate - model.positions[node][axis];
                }
            }
            batch
                .edges
                .insert((first.min(second) as u32, first.max(second) as u32));
        }
        batch
    }

    pub(super) fn finish(
        self,
        model: &BatchGas,
        evaluated_pairs: usize,
    ) -> Result<(BatchGas, StepReceipt), GasError> {
        let config = model.config;
        let n = model.positions.len();
        let mut retained = vec![false; n];
        for &(a, b) in &self.edges {
            retained[a as usize] = true;
            retained[b as usize] = true;
        }
        let mut old_to_new = vec![None; n];
        let mut positions = Vec::new();
        let mut errors = Vec::new();
        for (i, keep) in retained.into_iter().enumerate() {
            if !keep {
                continue;
            }
            old_to_new[i] = Some(positions.len() as u32);
            positions.push(std::array::from_fn(|axis| {
                model.positions[i][axis]
                    + config.winner_rate * self.winner_delta[i][axis]
                        / (self.winner_count[i] as f64 + config.epsilon)
                    + config.neighbour_rate * self.neighbour_delta[i][axis]
                        / (self.neighbour_count[i] as f64 + config.epsilon)
            }));
            errors.push(self.errors[i] * config.error_discount);
        }
        if errors.iter().any(|e| !e.is_finite() || *e < 0.0)
            || validate_points(&positions, config.max_nodes).is_err()
        {
            return Err(GasError::NumericalFailure);
        }
        let mut edges = Vec::with_capacity(self.edges.len());
        for (a, b) in self.edges {
            let pair = old_to_new[a as usize]
                .zip(old_to_new[b as usize])
                .ok_or(GasError::NumericalFailure)?;
            edges.push(pair);
        }
        let removed = n - positions.len();
        let graph = SignedGraph {
            n_nodes: positions.len() as u32,
            signs: vec![1; edges.len()],
            edges,
        };
        let mut next = BatchGas {
            positions,
            graph,
            errors,
            config,
        };
        let insertion_parents = next.grow()?;
        let receipt = StepReceipt {
            evaluated_pairs,
            nodes_before: n,
            nodes_after: next.positions.len(),
            edges_after: next.graph.edges.len(),
            removed,
            inserted: insertion_parents.len(),
            old_to_new,
            insertion_parents,
        };
        Ok((next, receipt))
    }
}

impl BatchGas {
    fn grow(&mut self) -> Result<Vec<[u32; 2]>, GasError> {
        let mut sorted = self.errors.clone();
        sorted.sort_by(f64::total_cmp);
        let offset = self.config.growth_quantile * (sorted.len() - 1) as f64;
        let lo = offset.floor() as usize;
        let hi = offset.ceil() as usize;
        let threshold = sorted[lo] + (offset - lo as f64) * (sorted[hi] - sorted[lo]);
        let count = self
            .errors
            .iter()
            .filter(|e| **e > threshold)
            .count()
            .min(self.config.max_nodes - self.positions.len());
        let mut parents = Vec::with_capacity(count);
        for _ in 0..count {
            let adjacency = Csr::from_graph(&self.graph);
            let first = highest_error(0..self.errors.len() as u32, &self.errors)
                .ok_or(GasError::NumericalFailure)?;
            let second = highest_error(adjacency.neighbours(first).iter().copied(), &self.errors)
                .ok_or(GasError::NumericalFailure)?;
            let new = self.positions.len() as u32;
            let a = first as usize;
            let b = second as usize;
            self.positions.push(std::array::from_fn(|axis| {
                0.5 * (self.positions[a][axis] + self.positions[b][axis])
            }));
            self.errors[a] *= self.config.growth_discount;
            self.errors[b] *= self.config.growth_discount;
            self.errors
                .push(0.5 * self.errors[a] + 0.5 * self.errors[b]);
            let removed = (first.min(second), first.max(second));
            self.graph.edges.retain(|pair| *pair != removed);
            self.graph.edges.extend([(first, new), (second, new)]);
            self.graph.edges.sort_unstable();
            self.graph.n_nodes += 1;
            self.graph.signs.resize(self.graph.edges.len(), 1);
            parents.push([first, second]);
        }
        Ok(parents)
    }
}

fn nearest_two(positions: &[[f64; 3]], point: &[f64; 3]) -> (usize, usize, f64) {
    let mut first = (f64::INFINITY, usize::MAX);
    let mut second = first;
    for (index, position) in positions.iter().enumerate() {
        let squared = position
            .iter()
            .zip(point)
            .map(|(a, b)| (a - b) * (a - b))
            .sum::<f64>();
        let candidate = (squared, index);
        if candidate < first {
            second = first;
            first = candidate;
        } else if candidate < second {
            second = candidate;
        }
    }
    (first.1, second.1, first.0.sqrt())
}

fn highest_error(nodes: impl Iterator<Item = u32>, errors: &[f64]) -> Option<u32> {
    nodes.max_by(|a, b| {
        errors[*a as usize]
            .total_cmp(&errors[*b as usize])
            .then_with(|| b.cmp(a))
    })
}
