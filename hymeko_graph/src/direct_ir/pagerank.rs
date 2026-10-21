//! PageRank over the canonical IR, without a pairwise graph expansion.
//!
//! The default walk chooses an incident arc uniformly (counting incidence
//! multiplicity), then a member slot uniformly. Returning to the same vertex is
//! permitted. This is the incidence walk of Zhou, Huang and Schoelkopf (NIPS
//! 2006, section 4), with PageRank teleportation and dangling-node correction.
//! It is not the no-self-return walk on the unweighted clique expansion.
//!
//! With source masses `a(v,e)` and destination masses `b(v,e)`, the transition is
//! `P(u,v) = sum_e a(u,e)/sum_f a(u,f) * b(v,e)/sum_w b(w,e)`.
//! Arcs with zero destination mass are excluded from the source denominator.
//! Duplicate slots add mass; signs and ports affect routing only when selected.
//! A directed pairwise arc with one source and one destination recovers ordinary
//! directed PageRank. All `Ir.nodes` are ranked, including isolated nodes.
//!
//! Ordered execution gathers/scatters incidences in O(V + E + I). A certified
//! component plan can use range prefix/difference contraction in O(V + E + R),
//! where R counts contiguous ranges; canonical updates check convergence.
//! Core owns the shared numeric incidence projection and row operations. Target
//! bindings and routing are prepared once; the iteration never decodes IR records.
//! The factorized operator is fused per arc, without a materialized transition matrix.
//!
//! References: [incidence random walk, section 4](https://papers.nips.cc/paper/2006/file/dff8e9c2ac33381546d96deea9922999-Paper.pdf)
//! and [PageRank](https://snap.stanford.edu/class/cs224w-readings/Brin98Anatomy.pdf).

mod accumulator;
mod parallel;
mod plan;
mod workspace;
use accumulator::Sum;
pub use plan::PageRankPlanStats;
use plan::{ExecutionPlan, SelectedRows};
pub use workspace::PageRankWorkspace;

use hymeko::common::ids::{DeclId, HyperArcId, NodeId};
use hymeko::ir::incidence::{IncidenceDomain, IncidenceProjection, IncidenceProjectionError};
use hymeko::ir::incidence::{IncidenceError, IncidenceIndex};
use hymeko::ir::ir::{Ir, PortDirection, SignedRefR, ValueR};
use hymeko::tensor::operator::OrderedAccumulator;
use hymeko::tensor::operator::{RowContraction, RowTransform};
use std::sync::OnceLock;

/// Which reference attributes determine source and destination membership.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RankFlow {
    /// Every incidence participates on both sides; signs and ports do not gate it.
    #[default]
    Undirected,
    /// Minus incidences send to plus incidences. Neutral slots do not participate.
    MinusToPlus,
    /// In ports send to Out ports. Unspecified ports do not participate.
    InToOut,
}

impl RankFlow {
    fn roles(self, reference: &SignedRefR) -> (bool, bool) {
        match self {
            Self::Undirected => (true, true),
            Self::MinusToPlus => (reference.sign() < 0, reference.sign() > 0),
            Self::InToOut => (
                reference.direction() == PortDirection::In,
                reference.direction() == PortDirection::Out,
            ),
        }
    }
}

/// Explicit probability-weight interpretation; never infer a scalar from a vector.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RankWeights {
    /// Each incidence has unit mass, regardless of its optional weight payload.
    #[default]
    Unit,
    /// Use one numeric component on both sides. Absent weights mean one; a
    /// present vector must contain a finite nonnegative number at this index.
    Component(usize),
}

/// Preparation-time semantics. Changing these requires a new prepared PageRank.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageRankModel {
    /// Independent source/destination role selection.
    pub flow: RankFlow,
    /// Incidence probability mass interpretation.
    pub weights: RankWeights,
}

/// Coordinate candidates for the cost-based serial range planner. Unprofitable
/// layouts are not executed; results always use canonical NodeId order.
/// Parallel pull execution retains canonical per-node reduction order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RankOrdering {
    /// Keep profitable canonical ranges, otherwise try first-encounter ordering.
    #[default]
    Auto,
    /// Consider only the original coordinate ranges.
    Canonical,
    /// Consider only the first-encounter coordinate layout.
    FirstEncounter,
}

/// Transition semantics and an independent preparation policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageRankOptions {
    /// Transition semantics.
    pub model: PageRankModel,
    /// Candidate policy; only eligible unit/undirected range plans use it.
    pub ordering: RankOrdering,
}
impl From<PageRankModel> for PageRankOptions {
    fn from(model: PageRankModel) -> Self {
        Self {
            model,
            ordering: RankOrdering::Auto,
        }
    }
}
impl PageRankModel {
    /// Select preparation policy without changing the mathematical model.
    pub fn with_ordering(self, ordering: RankOrdering) -> PageRankOptions {
        PageRankOptions {
            model: self,
            ordering,
        }
    }
}

/// Bounded power-iteration controls, independent of graph preparation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageRankConfig {
    /// Probability of following a graph transition: finite, 0 <= damping < 1.
    pub damping: f64,
    /// Requested absolute L1 error bound; finite and strictly positive.
    pub tolerance: f64,
    /// Maximum updates; strictly positive. Exhaustion is reported in the result.
    pub max_iterations: usize,
}

impl Default for PageRankConfig {
    fn default() -> Self {
        Self {
            damping: 0.85,
            tolerance: 1e-10,
            max_iterations: 200,
        }
    }
}

impl PageRankConfig {
    /// Validate controls without executing a solver.
    /// # Errors
    /// Returns `InvalidConfig` for unsupported damping, tolerance or iteration cap.
    pub fn validate(self) -> Result<(), PageRankError> {
        if !self.damping.is_finite()
            || !(0.0..1.0).contains(&self.damping)
            || !self.tolerance.is_finite()
            || self.tolerance <= 0.0
            || self.max_iterations == 0
        {
            return Err(PageRankError::InvalidConfig);
        }
        Ok(())
    }
}

/// Checked preparation or execution failure, carrying only bounded data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageRankError {
    /// The caller requested zero execution workers.
    InvalidWorkers {
        /// Rejected worker count.
        workers: usize,
    },
    /// The OS could not create the caller's private worker pool.
    WorkerPool {
        /// Requested worker count whose pool could not be created.
        workers: usize,
    },
    /// The existing reverse-index coordinate domain could not be constructed.
    ReverseIndex(IncidenceError),
    /// A supplied shared projection does not use canonical NodeId coordinates.
    InvalidProjectionDomain,
    /// Damping, tolerance, or iteration limit is outside its documented domain.
    InvalidConfig,
    /// A node record does not map back to its own declaration and NodeId.
    InvalidNode(NodeId),
    /// A reference does not resolve to a coherent node record.
    InvalidTarget {
        /// Arc containing the reference.
        arc: HyperArcId,
        /// Reference position within that arc.
        slot: usize,
        /// Target declaration.
        target: DeclId,
    },
    /// Missing/nonnumeric/negative/nonfinite selected component, or foreign payload ID.
    InvalidWeight {
        /// Arc containing the reference.
        arc: HyperArcId,
        /// Reference position within that arc.
        slot: usize,
    },
    /// Personalization must have one finite nonnegative value per node and positive mass.
    InvalidPersonalization,
    /// An intermediate sum overflowed or the probability invariant was lost.
    NumericalMass,
}

impl std::fmt::Display for PageRankError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PageRank: {self:?}")
    }
}
impl std::error::Error for PageRankError {}

impl From<IncidenceProjectionError> for PageRankError {
    fn from(error: IncidenceProjectionError) -> Self {
        match error {
            IncidenceProjectionError::InvalidNode(node) => Self::InvalidNode(node),
            IncidenceProjectionError::InvalidTarget { arc, slot, target } => {
                Self::InvalidTarget { arc, slot, target }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Rates {
    source: f64,
    destination: f64,
}

/// Semantic policies prepared before iteration. Unit undirected incidence needs
/// no coefficient array; directed unit incidence uses two bits per slot.
enum PreparedRates {
    Unit,
    Directed(Vec<u8>),
    Weighted(Vec<Rates>),
}
impl PreparedRates {
    fn new(model: PageRankModel, count: usize) -> Self {
        match (model.weights, model.flow) {
            (RankWeights::Unit, RankFlow::Undirected) => Self::Unit,
            (RankWeights::Unit, _) => Self::Directed(Vec::with_capacity(count)),
            (RankWeights::Component(_), _) => Self::Weighted(Vec::with_capacity(count)),
        }
    }
    fn push(&mut self, rate: Rates, source: bool, destination: bool) {
        match self {
            Self::Unit => {}
            Self::Directed(roles) => roles.push(u8::from(source) | (u8::from(destination) << 1)),
            Self::Weighted(rates) => rates.push(rate),
        }
    }
    fn source(&self, position: usize) -> f64 {
        match self {
            Self::Unit => 1.0,
            Self::Directed(roles) => f64::from(roles[position] & 1),
            Self::Weighted(rates) => rates[position].source,
        }
    }
    fn weighted(&self) -> bool {
        matches!(self, Self::Weighted(_))
    }
    fn retained_bytes(&self) -> usize {
        match self {
            Self::Unit => 0,
            Self::Directed(roles) => roles.capacity(),
            Self::Weighted(rates) => rates.capacity() * size_of::<Rates>(),
        }
    }
}

/// Validated incidence walk over Core's shared immutable numeric projection.
/// Storage is O(V+E+I); unit routing omits numeric weight arrays. Projection
/// clones share their coordinate buffers, while normalization belongs to this model.
pub struct PageRank<'ir> {
    projection: IncidenceProjection<'ir>,
    outgoing: Vec<f64>,
    destinations: Vec<f64>,
    rates: PreparedRates,
    execution: Option<ExecutionPlan<'ir>>,
    incoming_terms: Option<usize>,
    reverse: OnceLock<Result<IncidenceIndex<'ir>, IncidenceError>>,
}

impl<'ir> PageRank<'ir> {
    /// Validate all node targets and prepare the selected incidence walk.
    ///
    /// # Errors
    /// Rejects inconsistent node bindings, non-node targets, invalid selected
    /// weights, and overflowing normalization sums. Empty arcs are harmless.
    pub fn build(ir: &'ir Ir, options: impl Into<PageRankOptions>) -> Result<Self, PageRankError> {
        Self::from_projection(
            IncidenceProjection::build(ir, IncidenceDomain::Nodes)?,
            options,
        )
    }

    /// Reuse a Core node projection, including one lazily cached by an existing
    /// IncidenceView/PreparedIr. Coefficients remain specific to this model.
    /// # Errors
    /// Rejects a non-node domain and the same weight/normalization errors as build.
    pub fn from_projection(
        projection: IncidenceProjection<'ir>,
        options: impl Into<PageRankOptions>,
    ) -> Result<Self, PageRankError> {
        let options = options.into();
        let model = options.model;
        if projection.domain() != IncidenceDomain::Nodes {
            return Err(PageRankError::InvalidProjectionDomain);
        }
        let ir = projection.ir();
        let rates = PreparedRates::new(model, projection.incidence_count());
        let mut prepared = Self {
            projection,
            outgoing: vec![0.0; ir.nodes.len()],
            destinations: Vec::with_capacity(ir.arcs.len()),
            rates,
            execution: None,
            incoming_terms: None,
            reverse: OnceLock::new(),
        };
        // Unit undirected masses are cardinalities. Up to 2^53 incidences,
        // every partial integer count is exactly representable in f64. Reuse
        // the validated projection; neither IR payload reads nor a compensated
        // scratch vector are needed. Larger counts retain general preparation.
        if matches!(prepared.rates, PreparedRates::Unit)
            && prepared.projection.incidence_count() as u128 <= 1u128 << f64::MANTISSA_DIGITS
        {
            for row in prepared.projection.rows() {
                prepared.destinations.push(row.len() as f64);
                row.scatter(1.0, &mut prepared.outgoing, |_| 1.0);
            }
            prepared.incoming_terms =
                Some(prepared.outgoing.iter().copied().fold(0.0, f64::max) as usize);
            prepared.execution =
                ExecutionPlan::build(&prepared.projection, &prepared.outgoing, options.ordering);
            return Ok(prepared);
        }
        for (raw, arc) in ir.arcs.iter().enumerate() {
            let mut destination = Sum::default();
            for (slot, reference) in arc.refs.iter().enumerate() {
                let id = HyperArcId::new(raw);
                let weight = Self::weight(ir, model.weights, reference, id, slot)?;
                let (source, sink) = model.flow.roles(reference);
                let rate = Rates {
                    source: if source { weight } else { 0.0 },
                    destination: if sink { weight } else { 0.0 },
                };
                destination.add(rate.destination);
                prepared.rates.push(rate, source, sink);
            }
            let total = destination.total();
            if !total.is_finite() {
                return Err(PageRankError::NumericalMass);
            }
            prepared.destinations.push(total);
        }
        prepared.normalize()?;
        Ok(prepared)
    }

    fn weight(
        ir: &Ir,
        weights: RankWeights,
        reference: &SignedRefR,
        arc: HyperArcId,
        slot: usize,
    ) -> Result<f64, PageRankError> {
        let RankWeights::Component(component) = weights else {
            return Ok(1.0);
        };
        let invalid = PageRankError::InvalidWeight { arc, slot };
        let atom = ir.reference_atom(reference).map_err(|_| invalid)?;
        let weight = match atom.weights {
            None => 1.0,
            Some(values) => match values.get(component) {
                Some(ValueR::Num(value)) => *value,
                _ => return Err(invalid),
            },
        };
        if !weight.is_finite() || weight < 0.0 {
            return Err(invalid);
        }
        Ok(weight)
    }

    fn normalize(&mut self) -> Result<(), PageRankError> {
        let mut outgoing = vec![Sum::default(); self.outgoing.len()];
        for (raw, row) in self.projection.rows().enumerate() {
            if self.destinations[raw] > 0.0 {
                row.scatter(1.0, &mut outgoing, |position| self.rates.source(position));
            }
        }
        for (sum, output) in outgoing.into_iter().zip(&mut self.outgoing) {
            *output = sum.total();
            if !output.is_finite() {
                return Err(PageRankError::NumericalMass);
            }
        }
        // Normalize finite component weights before iteration: no division of a
        // rank by a subnormal raw weight, and no product of two huge raw weights.
        if let PreparedRates::Weighted(rates) = &mut self.rates {
            for (raw, row) in self.projection.rows().enumerate() {
                let destination = self.destinations[raw];
                for (slot, node) in row.iter_targets().enumerate() {
                    let source = self.outgoing[node];
                    let rate = &mut rates[row.offset() + slot];
                    rate.source = if destination > 0.0 && source > 0.0 {
                        rate.source / source
                    } else {
                        0.0
                    };
                    rate.destination = if destination > 0.0 {
                        rate.destination / destination
                    } else {
                        0.0
                    };
                }
            }
        }
        Ok(())
    }

    /// Number of canonical node records ranked by this model.
    pub fn node_count(&self) -> usize {
        self.outgoing.len()
    }
    /// Number of incidence slots, including duplicates and inactive roles.
    pub fn incidence_count(&self) -> usize {
        self.projection.incidence_count()
    }
    /// Numeric buffers reachable from this model, including its shared projection
    /// but excluding the borrowed IR. Shared projections are not allocated per clone.
    pub fn retained_bytes(&self) -> usize {
        (self.outgoing.capacity() + self.destinations.capacity()) * size_of::<f64>()
            + self.rates.retained_bytes()
            + self.projection.retained_bytes()
            + self
                .execution
                .as_ref()
                .map_or(0, ExecutionPlan::retained_bytes)
            + self
                .reverse
                .get()
                .and_then(|result| result.as_ref().ok())
                .map_or(0, IncidenceIndex::retained_bytes)
    }
    /// Present when structural analysis selected a lower-work range plan.
    pub fn plan_stats(&self) -> Option<PageRankPlanStats> {
        self.execution.as_ref().map(|p| p.stats)
    }
    /// Exact maximum number of incoming unit terms when that structural proof
    /// is available. Weighted and directed preparation currently returns None.
    pub fn incoming_term_bound(&self) -> Option<usize> {
        self.incoming_terms
    }

    /// Solve from the normalized personalization (uniform when absent).
    /// Dangling mass uses that same distribution. A warm workspace allocates
    /// nothing. Scores are indexed by canonical NodeId, never DeclId.
    ///
    /// # Errors
    /// Invalid controls/personalization or a nonfinite/probability-invalid update.
    /// Reaching the iteration cap is a successful result with `converged=false`.
    pub fn run<'w>(
        &self,
        config: PageRankConfig,
        workspace: &'w mut PageRankWorkspace,
        personalization: Option<&[f64]>,
    ) -> Result<PageRankResult<'w>, PageRankError> {
        config.validate()?;
        workspace.initialize(self.node_count(), personalization)?;
        let pool = workspace.pool.clone();
        if let Some(pool) = pool {
            let reverse = (self.node_count() != 0 && config.damping != 0.0)
                .then(|| self.reverse_index())
                .transpose()?;
            pool.install(|| self.iterate(config, workspace, reverse))
        } else {
            self.iterate(config, workspace, None)
        }
    }

    fn iterate<'w>(
        &self,
        config: PageRankConfig,
        workspace: &'w mut PageRankWorkspace,
        reverse: Option<&IncidenceIndex<'ir>>,
    ) -> Result<PageRankResult<'w>, PageRankError> {
        let mut progress = Progress::default();
        if self.node_count() == 0 || config.damping == 0.0 {
            progress.converged = true;
        } else {
            workspace.prepare_input(&self.outgoing, self.rates.weighted());
            if self.incoming_terms.is_some_and(|n| n <= 2) {
                workspace.auxiliary.resize(self.node_count(), 0.0);
                workspace.auxiliary.fill(0.0);
            }
            let mut certified = self.execution.is_none() || reverse.is_some();
            let mut previous_change = f64::INFINITY;
            for iteration in 1..=config.max_iterations {
                // A canonical compensated update certifies every successful
                // return, and also the error estimate at the iteration cap.
                let ordered = certified || iteration == config.max_iterations;
                let (change, ordered) = if let Some(reverse) = reverse {
                    (
                        self.update_parallel(config.damping, workspace, reverse)?,
                        true,
                    )
                } else if ordered {
                    (self.update(config.damping, workspace)?, true)
                } else {
                    self.update_ranges(config.damping, workspace)?
                };
                progress = Progress {
                    iterations: iteration,
                    change_l1: change,
                    error_bound_l1: config.damping * change / (1.0 - config.damping),
                    converged: false,
                };
                let candidate = progress.error_bound_l1 <= config.tolerance;
                progress.converged = ordered && candidate;
                if progress.converged {
                    break;
                }
                // Regrouping cannot trap the solve at its rounding-error floor.
                if candidate || change >= previous_change || ordered {
                    certified = true;
                }
                previous_change = change;
            }
        }
        Ok(PageRankResult {
            scores: &workspace.scores,
            progress,
        })
    }

    fn update(&self, damping: f64, ws: &mut PageRankWorkspace) -> Result<f64, PageRankError> {
        if self.incoming_terms.is_some_and(|n| n <= 2) {
            return self.update_bounded_scatter(damping, ws);
        }
        // Dispatch once per update, not once per incidence. Each instantiation
        // calls the same Core row operations and keeps the original FP order.
        match &self.rates {
            PreparedRates::Unit => self.distribute(damping, ws, |_| 1.0, |_| 1.0, false),
            PreparedRates::Directed(roles) => self.distribute(
                damping,
                ws,
                |i| f64::from(roles[i] & 1),
                |i| f64::from((roles[i] >> 1) & 1),
                false,
            ),
            PreparedRates::Weighted(rates) => self.distribute(
                damping,
                ws,
                |i| rates[i].source,
                |i| rates[i].destination,
                true,
            ),
        }
        self.finish_transition(damping, ws, self.rates.weighted())
    }

    /// Every execution strategy uses the same teleportation and vertex update.
    #[inline]
    fn finish_transition(
        &self,
        damping: f64,
        workspace: &mut PageRankWorkspace,
        weighted: bool,
    ) -> Result<f64, PageRankError> {
        let teleport = (1.0 - damping) + damping * workspace.dangling;
        workspace.finish_update(teleport, &self.outgoing, weighted)
    }

    fn update_bounded_scatter(
        &self,
        damping: f64,
        ws: &mut PageRankWorkspace,
    ) -> Result<f64, PageRankError> {
        self.projection.for_each_row(
            &mut RowContraction::new(
                &ws.scaled,
                &mut ws.auxiliary,
                |_| 1.0,
                |_| 1.0,
                RankTransfer {
                    damping,
                    normalized: false,
                    destinations: &self.destinations,
                },
            )
            .with_gather::<Sum>(),
        );
        // Each destination has at most two terms, including duplicate slots.
        // Their finite nonnegative sum has the same bits as Sum::sum_pair.
        for (incoming, value) in ws.incoming.iter_mut().zip(&mut ws.auxiliary) {
            *incoming = Sum::from_first(value.take_total());
        }
        self.finish_transition(damping, ws, false)
    }

    fn update_ranges(
        &self,
        damping: f64,
        ws: &mut PageRankWorkspace,
    ) -> Result<(f64, bool), PageRankError> {
        let Some(plan) = &self.execution else {
            return self.update(damping, ws).map(|v| (v, true));
        };
        ws.auxiliary.resize(ws.scores.len(), 0.0);
        ws.auxiliary.copy_from_slice(&ws.scores);
        match plan.layout.canonical_indices() {
            Some(order) => ws
                .ranges
                .prepare_mapped(&ws.scaled, &plan.partition, |slot| order[slot]),
            None => ws.ranges.prepare(&ws.scaled, &plan.partition),
        }
        plan.layout.for_each_row(&mut SelectedRows {
            visitor: ws.ranges.contraction(
                &plan.partition,
                RankTransfer {
                    damping,
                    normalized: false,
                    destinations: &self.destinations,
                },
            ),
            rows: &plan.range_rows,
            selected: true,
        });
        if plan.stats.range_rows < self.projection.arc_count() {
            self.projection.for_each_row(&mut SelectedRows {
                visitor: RowContraction::new(
                    &ws.scaled,
                    &mut ws.incoming,
                    |_| 1.0,
                    |_| 1.0,
                    RankTransfer {
                        damping,
                        normalized: false,
                        destinations: &self.destinations,
                    },
                ),
                rows: &plan.range_rows,
                selected: false,
            });
        }
        match plan.layout.canonical_indices() {
            Some(order) => ws
                .ranges
                .accumulate_mapped(&plan.partition, &mut ws.incoming, |slot| order[slot]),
            None => ws.ranges.accumulate_into(&plan.partition, &mut ws.incoming),
        }
        match self.finish_transition(damping, ws, false) {
            Ok(change) => Ok((change, false)),
            Err(PageRankError::NumericalMass) => {
                ws.scores.copy_from_slice(&ws.auxiliary);
                ws.prepare_input(&self.outgoing, false);
                self.update(damping, ws).map(|v| (v, true))
            }
            Err(error) => Err(error),
        }
    }

    fn distribute(
        &self,
        damping: f64,
        ws: &mut PageRankWorkspace,
        source: impl Fn(usize) -> f64,
        sink: impl Fn(usize) -> f64,
        normalized: bool,
    ) {
        self.projection.for_each_row(&mut RowContraction::new(
            &ws.scaled,
            &mut ws.incoming,
            source,
            sink,
            RankTransfer {
                damping,
                normalized,
                destinations: &self.destinations,
            },
        ));
    }

    #[inline]
    fn fraction(damping: f64, mass: f64, denominator: f64) -> f64 {
        let damped = damping * mass;
        if denominator == 2.0 {
            damped * 0.5
        } else {
            damped / denominator
        }
    }
}

struct RankTransfer<'a> {
    damping: f64,
    normalized: bool,
    destinations: &'a [f64],
}
impl RowTransform for RankTransfer<'_> {
    #[inline(always)]
    fn enabled(&self, row: usize) -> bool {
        self.destinations[row] > 0.0
    }
    #[inline(always)]
    fn apply(&self, raw: usize, mass: f64) -> f64 {
        let destination = self.destinations[raw];
        let denominator = if self.normalized { 1.0 } else { destination };
        PageRank::fraction(self.damping, mass, denominator)
    }
}

/// Numerical stopping evidence; the error estimate assumes stochastic arithmetic
/// up to floating-point roundoff and is the L1 contraction bound for the returned iterate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Progress {
    /// Number of graph updates, excluding initialization.
    pub iterations: usize,
    /// L1 difference between the last two iterates (not the next-step residual).
    pub change_l1: f64,
    /// damping/(1-damping) times change_l1.
    pub error_bound_l1: f64,
    /// Whether the requested bound was attained, or the exact zero-iteration case applies.
    pub converged: bool,
}

/// Borrowed ranking and convergence evidence; valid until its workspace is reused.
#[derive(Clone, Copy, Debug)]
pub struct PageRankResult<'w> {
    /// Probability scores in canonical NodeId order.
    pub scores: &'w [f64],
    /// Stopping evidence, including explicit cap exhaustion.
    pub progress: Progress,
}
impl PageRankResult<'_> {
    /// Probability of one canonical node, or None for an out-of-range NodeId.
    pub fn score(&self, node: NodeId) -> Option<f64> {
        self.scores.get(node.raw()).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::{PageRank, Sum};
    use crate::rand_lcg::Lcg;
    use hymeko::ir::incidence::IncidenceAccumulator;

    struct PairCase([f64; 2]);
    impl PairCase {
        const VALUES: [f64; 14] = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            1e16,
            -1e16,
            f64::from_bits(1),
            -f64::from_bits(1),
            f64::MIN_POSITIVE,
            f64::MAX,
            -f64::MAX,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ];

        fn check(self) {
            let [first, second] = self.0;
            let mut expected = Sum::default();
            expected.add(first);
            expected.add(second);
            let actual = Sum::sum_pair(|| first, || second);
            if expected.total().is_nan() {
                assert!(actual.is_nan());
            } else {
                assert_eq!(actual.to_bits(), expected.total().to_bits(), "{:?}", self.0);
            }
        }
    }

    #[test]
    fn pair_reduction_matches_neumaier_on_extremes_and_random_ieee_patterns() {
        for first in PairCase::VALUES {
            for second in PairCase::VALUES {
                PairCase([first, second]).check();
            }
        }
        let mut random = Lcg::new(0x51_EED);
        for _ in 0..100_000 {
            let first = f64::from_bits(random.next_u64());
            let second = f64::from_bits(random.next_u64());
            PairCase([first, second]).check();
            PairCase([first, -first]).check();
            PairCase([first, first * f64::EPSILON * 0.5]).check();
        }
    }

    #[test]
    fn exact_halving_preserves_the_original_fraction_bits() {
        for mass in PairCase::VALUES {
            for damping in [0.0, 0.5, 0.85, 1.0 - f64::EPSILON] {
                for denominator in [1.0, 2.0, 3.0, 32.0, 1000.0] {
                    let expected = damping * mass / denominator;
                    let actual = PageRank::fraction(damping, mass, denominator);
                    if expected.is_nan() {
                        assert!(actual.is_nan());
                    } else {
                        assert_eq!(actual.to_bits(), expected.to_bits());
                    }
                }
            }
        }
    }

    #[test]
    fn first_term_initialization_preserves_the_neumaier_recurrence() {
        // The unchanged add recurrence is the reference for the new seed only.
        // Include cancellation, underflow, overflow, signed zeros and nonfinites.
        assert_eq!(Sum::sum([]).to_bits(), 0.0f64.to_bits());
        for first in PairCase::VALUES {
            for second in PairCase::VALUES {
                for third in PairCase::VALUES {
                    let mut expected = Sum::default();
                    let mut seeded = Sum::from_first(first);
                    expected.add(first);
                    for value in [second, third] {
                        expected.add(value);
                        seeded.add(value);
                    }
                    let actual = Sum::sum([first, second, third]);
                    if expected.total().is_nan() {
                        assert!(actual.is_nan() && seeded.total().is_nan());
                    } else {
                        assert_eq!(actual.to_bits(), expected.total().to_bits());
                        assert_eq!(seeded.total().to_bits(), expected.total().to_bits());
                    }
                }
            }
        }
    }

    #[test]
    fn chunk_boundaries_preserve_ordered_neumaier_bits() {
        for first in PairCase::VALUES {
            for second in PairCase::VALUES {
                let terms = [first, second, -first, 0.0, -0.0, second];
                let expected = Sum::sum(terms);
                for width in 1..=terms.len() {
                    let actual = Sum::sum_chunks(terms.chunks(width).map(|c| c.iter().copied()));
                    if expected.is_nan() {
                        assert!(actual.is_nan());
                    } else {
                        assert_eq!(actual.to_bits(), expected.to_bits());
                    }
                }
            }
        }
        assert_eq!(Sum::sum_chunks([[], [], []]).to_bits(), 0.0f64.to_bits());
    }
}
