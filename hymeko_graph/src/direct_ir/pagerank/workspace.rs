//! PageRank numeric state and its fused, ordered vertex sweep.
use super::{PageRankError, accumulator::Sum};
use hymeko::tensor::operator::OrderedAccumulator;
use hymeko::tensor::operator::RangeWorkspace;
use hymeko_numerics::GmresWorkspace;
use std::sync::Arc;

/// Caller-owned power-iteration buffers, reusable across models and node counts.
#[derive(Default)]
pub struct PageRankWorkspace {
    pub(super) scores: Vec<f64>,
    pub(super) scaled: Vec<f64>,
    pub(super) personalization: Vec<f64>,
    pub(super) incoming: Vec<Sum>,
    pub(super) dangling: f64,
    pub(super) ranges: RangeWorkspace<Sum>,
    /// Checkpoint for speculative ranges, or scalar output under a two-term bound.
    pub(super) auxiliary: Vec<f64>,
    pub(super) arc_values: Vec<f64>,
    pub(super) krylov: GmresWorkspace,
    pub(super) rhs: Vec<f64>,
    pub(super) pool: Option<Arc<rayon::ThreadPool>>,
}

impl PageRankWorkspace {
    /// Empty scratch; the first run sizes it.
    pub fn new() -> Self {
        Self::default()
    }
    /// Configure a private worker pool, without changing Rayon's global pool.
    /// One worker restores serial execution. Reapplying the same count is free.
    /// # Errors
    /// Rejects zero or an OS failure to create the requested worker pool.
    pub fn set_workers(&mut self, workers: usize) -> Result<(), PageRankError> {
        if workers == 0 {
            return Err(PageRankError::InvalidWorkers { workers });
        }
        if workers == self.worker_count() {
            return Ok(());
        }
        self.pool = if workers == 1 {
            None
        } else {
            Some(Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(workers)
                    .build()
                    .map_err(|_| PageRankError::WorkerPool { workers })?,
            ))
        };
        Ok(())
    }
    /// Configured compute-worker count; the default is one.
    pub fn worker_count(&self) -> usize {
        self.pool
            .as_ref()
            .map_or(1, |pool| pool.current_num_threads())
    }
    /// Owned numeric buffer capacities, including output but excluding preparation
    /// and OS/thread-pool bookkeeping or thread stacks.
    pub fn retained_bytes(&self) -> usize {
        (self.scores.capacity() + self.scaled.capacity() + self.personalization.capacity())
            * size_of::<f64>()
            + self.incoming.capacity() * size_of::<Sum>()
            + self.ranges.retained_bytes()
            + self.auxiliary.capacity() * size_of::<f64>()
            + self.arc_values.capacity() * size_of::<f64>()
            + self.krylov.retained_bytes()
            + self.rhs.capacity() * size_of::<f64>()
    }

    pub(super) fn initialize(
        &mut self,
        nodes: usize,
        distribution: Option<&[f64]>,
    ) -> Result<(), PageRankError> {
        if distribution.is_some_and(|p| p.len() != nodes) {
            return Err(PageRankError::InvalidPersonalization);
        }
        self.scores.resize(nodes, 0.0);
        self.scaled.resize(nodes, 0.0);
        self.personalization.resize(nodes, 0.0);
        self.incoming.resize(nodes, Sum::default());
        if nodes == 0 {
            return Ok(());
        }
        if let Some(values) = distribution {
            if values.iter().any(|v| !v.is_finite() || *v < 0.0) {
                return Err(PageRankError::InvalidPersonalization);
            }
            let maximum = values.iter().copied().fold(0.0, f64::max);
            if maximum == 0.0 {
                return Err(PageRankError::InvalidPersonalization);
            }
            for (output, &value) in self.personalization.iter_mut().zip(values) {
                *output = value / maximum;
            }
            let sum = Sum::of(self.personalization.iter().copied());
            for output in &mut self.personalization {
                *output /= sum;
            }
        } else {
            self.personalization.fill(1.0 / nodes as f64);
        }
        self.scores.copy_from_slice(&self.personalization);
        Ok(())
    }

    /// Stage the initial input and clear incoming state together. Repeated runs,
    /// including recovery from any previous error, cannot inherit stale sums.
    pub(super) fn prepare_input(&mut self, outgoing: &[f64], weighted: bool) {
        self.prepare_vector(None, outgoing, weighted);
    }

    /// Stage either the current probability scores or a signed Krylov vector
    /// through the same degree/dangling normalization.
    pub(super) fn prepare_vector(
        &mut self,
        input: Option<&[f64]>,
        outgoing: &[f64],
        weighted: bool,
    ) {
        let normalization = InputNormalization { weighted };
        let mut dangling = Sum::default();
        let scores = input.unwrap_or(&self.scores);
        debug_assert_eq!(scores.len(), outgoing.len());
        for (((&score, &degree), scaled), incoming) in scores
            .iter()
            .zip(outgoing)
            .zip(&mut self.scaled)
            .zip(&mut self.incoming)
        {
            *scaled = normalization.scale(score, degree, &mut dangling);
            *incoming = Sum::default();
        }
        self.dangling = dangling.total();
    }

    pub(super) fn finish_update(
        &mut self,
        teleport: f64,
        outgoing: &[f64],
        weighted: bool,
    ) -> Result<f64, PageRankError> {
        let normalization = InputNormalization { weighted };
        let mut change = Sum::default();
        let mut mass = Sum::default();
        let mut dangling = Sum::default();
        debug_assert_eq!(self.scores.len(), outgoing.len());
        // All incidence contractions finish before any next-iteration input is
        // written: this remains the same synchronous (Jacobi) PageRank update.
        for ((((score, incoming), &personalization), scaled), &degree) in self
            .scores
            .iter_mut()
            .zip(&mut self.incoming)
            .zip(&self.personalization)
            .zip(&mut self.scaled)
            .zip(outgoing)
        {
            let next = incoming.take_total() + teleport * personalization;
            if !next.is_finite() || next < 0.0 {
                return Err(PageRankError::NumericalMass);
            }
            change.add((next - *score).abs());
            mass.add(next);
            *score = next;
            *scaled = normalization.scale(next, degree, &mut dangling);
        }
        // Preserve the existing numerical gate and ordered scalar reductions.
        if (mass.total() - 1.0).abs() > 1e-8 {
            return Err(PageRankError::NumericalMass);
        }
        self.dangling = dangling.total();
        Ok(change.total())
    }
}

/// The same input normalization is used at initialization and at each completed
/// step. It never reorders contributions to the dangling-mass accumulator.
struct InputNormalization {
    weighted: bool,
}
impl InputNormalization {
    #[inline]
    fn scale(&self, score: f64, degree: f64, dangling: &mut Sum) -> f64 {
        if degree == 0.0 {
            dangling.add(score);
            0.0
        } else if self.weighted {
            score
        } else {
            score / degree
        }
    }
}

#[cfg(test)]
mod tests;
