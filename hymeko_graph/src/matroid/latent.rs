//! Learned proposal codebook beside exact canonical binary-column certificates.
//!
//! The existing BatchGas owns learning. Its prototype graph is statistical
//! state, not a projection/replacement of the authored hypergraph. Three bounded
//! support descriptors guide basis proposals; exact binary elimination admits
//! every result. This is not a learned circuit-equivalence or optimality oracle.
use super::{
    BinaryBasis, BinaryMatroidWorkspace, MatroidError, MatroidRef, binary_column,
    binary_weighted_basis,
};
use crate::{
    SignedGraph,
    neural_gas::{BatchGas, BatchGasConfig},
};
use hymeko::common::ids::HyperArcId;
use std::collections::BTreeSet;

/// Trained codebook and exact bases proposed from its centres.
#[derive(Debug)]
pub struct BinaryLatentProposals {
    /// Additive-weight optimum, retained as the deterministic control.
    pub baseline: BinaryBasis,
    /// Distinct candidate bases; their weights are proposal weights only.
    pub candidates: Vec<BinaryBasis>,
    /// Learned prototype centres in the three-dimensional descriptor space.
    pub prototypes: Vec<[f64; 3]>,
    /// Exact number of bounded learner updates performed.
    pub epochs: usize,
    /// Descriptor rows remain paired with the original canonical arc IDs.
    pub descriptors: Vec<(HyperArcId, [f64; 3])>,
}

/// Train the existing prototype engine and certify every decoded basis.
///
/// Descriptors are support density and mean/max incidence degree, invariant to
/// renumbering of nodes. There are no labels or diagram coordinates in learning.
/// `epochs` is explicitly bounded to 1..=64; training is not a benchmark.
pub fn binary_latent_proposals(
    m: MatroidRef<'_, '_>,
    ground: &[(HyperArcId, f64)],
    ws: &mut BinaryMatroidWorkspace,
    epochs: usize,
) -> Result<BinaryLatentProposals, MatroidError> {
    if !(1..=64).contains(&epochs) || !(2..=4096).contains(&ground.len()) {
        return Err(MatroidError::InvalidLatentRequest);
    }
    let baseline = binary_weighted_basis(m, ground, ws)?;
    let ir = m.incidence().ir();
    let n = ir.nodes.len().max(1) as f64;
    let a = ir.arcs.len().max(1) as f64;
    let mut descriptors = Vec::with_capacity(ground.len());
    for &(arc, _) in ground {
        let column = binary_column(m, arc)?;
        let degrees: Vec<_> = column
            .iter()
            .map(|node| m.incidence().incident_arcs(ir.nodes[node.raw()].decl).len() as f64)
            .collect();
        let size = column.len().max(1) as f64;
        descriptors.push((
            arc,
            [
                column.len() as f64 / n,
                degrees.iter().sum::<f64>() / (size * a),
                degrees.into_iter().fold(0.0, f64::max) / a,
            ],
        ));
    }
    let points: Vec<_> = descriptors.iter().map(|(_, x)| *x).collect();
    let seed = SignedGraph {
        n_nodes: 2,
        edges: vec![(0, 1)],
        signs: vec![1],
    };
    let config = BatchGasConfig {
        max_nodes: 8,
        max_distance_evaluations: 32768,
        ..Default::default()
    };
    let mut learner = BatchGas::new(&[points[0], points[points.len() - 1]], &seed, config)
        .map_err(|_| MatroidError::InvalidLatentRequest)?;
    for _ in 0..epochs {
        learner
            .step(&points)
            .map_err(|_| MatroidError::InvalidLatentRequest)?;
    }
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for centre in learner.positions() {
        let weights: Vec<_> = descriptors
            .iter()
            .map(|(arc, x)| {
                (
                    *arc,
                    x.iter()
                        .zip(centre)
                        .map(|(v, c)| (v - c) * (v - c))
                        .sum::<f64>(),
                )
            })
            .collect();
        let candidate = binary_weighted_basis(m, &weights, ws)?;
        let mut identity = candidate.elements.clone();
        identity.sort();
        if candidate.elements.len() != baseline.elements.len() {
            return Err(MatroidError::InvalidLatentRequest);
        }
        if seen.insert(identity) {
            candidates.push(candidate);
        }
    }
    Ok(BinaryLatentProposals {
        baseline,
        candidates,
        prototypes: learner.positions().to_vec(),
        epochs,
        descriptors,
    })
}
