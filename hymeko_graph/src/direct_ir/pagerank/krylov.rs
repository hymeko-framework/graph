//! Tensor's existing GMRES composed with the canonical PageRank transition.
use super::{
    PageRank, PageRankConfig, PageRankError, PageRankResult, PageRankWorkspace, Progress, Sum,
};
use hymeko::tensor::operator::OrderedAccumulator;
use hymeko_numerics::{GmresConfig, GmresTolerance, Operator};
use std::cell::RefCell;

/// Execution strategy over the same prepared incidence walk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageRankSolver {
    /// Existing structurally planned power iteration, also used by `run`.
    #[default]
    PowerIteration,
    /// Power iteration with canonical ordered contraction at every step. This
    /// controls for range regrouping when comparing iterative methods.
    OrderedPowerIteration,
    /// Serial, matrix-free restarted GMRES. Restart must be positive; its basis
    /// needs O(nodes * min(restart, iteration budget)) additional storage.
    Gmres {
        /// Maximum Arnoldi basis size before restarting.
        restart: usize,
    },
}

impl<'ir> PageRank<'ir> {
    /// Solve using an explicit strategy without rebuilding the IR or transition.
    ///
    /// GMRES uses Tensor's `(I - damping T)x = (1-damping)p` solver, where `T`
    /// acts on column scores (the transpose of the row-stochastic transition).
    /// Its signed basis vectors reuse power iteration's structural range plan
    /// when available, otherwise the ordered incidence contraction. Personalized
    /// dangling columns are included. No transition matrix is built.
    /// The candidate is normalized to nonnegative probability mass, then one
    /// canonical power update certifies the returned vector's global L1 bound.
    /// Tensor's L2 stopping flag alone never determines `converged` here.
    ///
    /// # Errors
    /// The same input errors as `run`, zero restart, or a nonserial workspace
    /// for GMRES. Exhaustion returns `converged=false`, not a false certificate.
    pub fn run_with_solver<'w>(
        &self,
        config: PageRankConfig,
        workspace: &'w mut PageRankWorkspace,
        personalization: Option<&[f64]>,
        solver: PageRankSolver,
    ) -> Result<PageRankResult<'w>, PageRankError> {
        let restart = match solver {
            PageRankSolver::PowerIteration => return self.run(config, workspace, personalization),
            PageRankSolver::OrderedPowerIteration => {
                return self.run_power(config, workspace, personalization, true);
            }
            PageRankSolver::Gmres { restart } => restart,
        };
        config.validate()?;
        if restart == 0 {
            return Err(PageRankError::InvalidConfig);
        }
        if workspace.worker_count() != 1 {
            return Err(PageRankError::InvalidWorkers {
                workers: workspace.worker_count(),
            });
        }
        workspace.initialize(self.node_count(), personalization)?;
        if self.node_count() == 0 || config.damping == 0.0 {
            return Ok(PageRankResult {
                scores: &workspace.scores,
                progress: Progress {
                    converged: true,
                    ..Default::default()
                },
            });
        }
        self.prepare_workspace(workspace);
        // Move the caller-owned buffers temporarily to keep the operator's
        // scratch borrow disjoint from the solver's mutable output and basis.
        let mut solution = std::mem::take(&mut workspace.scores);
        let mut krylov = std::mem::take(&mut workspace.krylov);
        let mut rhs = std::mem::take(&mut workspace.rhs);
        rhs.resize(self.node_count(), 0.0);
        for (b, p) in rhs.iter_mut().zip(&workspace.personalization) {
            *b = (1.0 - config.damping) * p;
        }
        // ||r||_1 <= sqrt(n)||r||_2. Leave rounding margin, then independently
        // certify with the canonical compensated update below.
        let tolerance =
            0.25 * config.tolerance * (1.0 - config.damping) / (self.node_count() as f64).sqrt();
        let transition = Transition {
            rank: self,
            workspace: RefCell::new(&mut *workspace),
        };
        let report = krylov.solve(
            &transition,
            config.damping,
            &rhs,
            &mut solution,
            GmresConfig {
                tolerance: GmresTolerance::Absolute(tolerance),
                restart: restart.min(self.node_count()),
                max_iterations: config.max_iterations - 1,
            },
        );
        drop(transition);
        workspace.scores = solution;
        workspace.krylov = krylov;
        workspace.rhs = rhs;
        if workspace.scores.iter().any(|x| !x.is_finite()) {
            return Err(PageRankError::NumericalMass);
        }
        for x in &mut workspace.scores {
            *x = x.max(0.0);
        }
        let mass = Sum::of(workspace.scores.iter().copied());
        if !mass.is_finite() || mass <= 0.0 {
            return Err(PageRankError::NumericalMass);
        }
        for score in &mut workspace.scores {
            *score /= mass;
        }
        workspace.prepare_input(&self.outgoing, self.rates.weighted());
        let change_l1 = self.update(config.damping, workspace)?;
        let error_bound_l1 = config.damping * change_l1 / (1.0 - config.damping);
        Ok(PageRankResult {
            scores: &workspace.scores,
            progress: Progress {
                iterations: report.iterations + 1,
                change_l1,
                error_bound_l1,
                converged: error_bound_l1 <= config.tolerance,
            },
        })
    }
}

/// Borrowed adapter only: Graph owns the stochastic semantics; Tensor owns GMRES.
/// Interior mutability satisfies Tensor's immutable `Operator` interface while
/// retaining one caller-owned serial scratch workspace.
struct Transition<'a, 'ir> {
    rank: &'a PageRank<'ir>,
    workspace: RefCell<&'a mut PageRankWorkspace>,
}
impl Operator for Transition<'_, '_> {
    fn dim(&self) -> usize {
        self.rank.node_count()
    }
    fn apply(&self, input: &[f64], output: &mut [f64]) {
        let mut workspace = self.workspace.borrow_mut();
        workspace.prepare_vector(Some(input), &self.rank.outgoing, self.rank.rates.weighted());
        match &self.rank.execution {
            Some(plan) => self.rank.contract_ranges(1.0, &mut workspace, plan),
            None => self.rank.contract_ordered(1.0, &mut workspace),
        }
        let ws = &mut **workspace;
        for ((value, incoming), p) in output
            .iter_mut()
            .zip(&mut ws.incoming)
            .zip(&ws.personalization)
        {
            *value = incoming.take_total() + ws.dangling * p;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rand_lcg::Lcg;
    use hymeko::common::ids::SymId;
    use hymeko::ir::ir::{AnnoR, ArcPlacement, IncidenceSign, Ir, IrConstruction, ReferenceDraft};

    #[test]
    fn structural_transition_is_linear_on_signed_vectors_and_clears_reused_state() {
        // Sliding long hyperedges with duplicate slots, plus a separate pair
        // component and isolates. Construction always uses the Core owner.
        for offset in [0, 13] {
            let mut ir = Ir::default();
            let mut construction = IrConstruction::new(&mut ir);
            let nodes: Vec<_> = (0..128)
                .map(|_| {
                    construction
                        .append_node(SymId::new(0), AnnoR::default(), vec![])
                        .unwrap()
                })
                .collect();
            let permute = |i: usize| {
                if offset == 0 {
                    i
                } else {
                    (17 * i + offset) % 128
                }
            };
            for row in 0..97 {
                let members: Vec<_> = if row == 96 {
                    vec![70, 71]
                } else {
                    let start = row % 24;
                    (start..start + 32).chain(std::iter::once(start)).collect()
                };
                let edge = construction
                    .append_edge(SymId::new(0), AnnoR::default(), vec![])
                    .unwrap();
                let refs = members
                    .into_iter()
                    .map(|i| ReferenceDraft::plain(nodes[permute(i)], IncidenceSign::Neutral))
                    .collect();
                construction
                    .append_arc(ArcPlacement::under_owner(edge), AnnoR::default(), refs)
                    .unwrap();
            }
            let rank = PageRank::build(&ir, super::super::PageRankModel::default()).unwrap();
            let plan = rank.plan_stats().unwrap();
            assert!(plan.range_rows > 0 && plan.range_rows < ir.arcs.len());
            let p: Vec<_> = (1..=128).map(|n| n as f64).collect();
            let mut workspace = PageRankWorkspace::new();
            workspace.initialize(128, Some(&p)).unwrap();
            let transition = Transition {
                rank: &rank,
                workspace: RefCell::new(&mut workspace),
            };
            let mut rng = Lcg::new(17);
            for exponent in [-100, 0, 100] {
                let scale = 2.0_f64.powi(exponent);
                let input: Vec<_> = (0..128)
                    .map(|_| (rng.next_in_range(2001) as f64 - 1000.0) / 1000.0 * scale)
                    .collect();
                let mut output = vec![f64::NAN; 128];
                transition.apply(&input, &mut output);
                // Independent explicit source/destination sum on this tiny IR.
                let mut expected = vec![0.0; 128];
                for arc in ir.arcs.iter() {
                    for source in arc.refs.iter() {
                        let u = ir.as_node(source.target()).unwrap().raw();
                        for destination in arc.refs.iter() {
                            let v = ir.as_node(destination.target()).unwrap().raw();
                            expected[v] += input[u] / rank.outgoing[u] / arc.refs.len() as f64;
                        }
                    }
                }
                let ws = transition.workspace.borrow();
                let dangling: f64 = input
                    .iter()
                    .zip(&rank.outgoing)
                    .filter(|(_, degree)| **degree == 0.0)
                    .map(|(x, _)| x)
                    .sum();
                for (value, probability) in expected.iter_mut().zip(&ws.personalization) {
                    *value += dangling * probability;
                }
                assert!(
                    ws.ranges.retained_bytes() > 0,
                    "the structural kernel must execute"
                );
                drop(ws);
                let error: f64 = output
                    .iter()
                    .zip(&expected)
                    .map(|(a, b)| (a - b).abs())
                    .sum();
                assert!(
                    error <= 2e-13 * input.iter().map(|v| v.abs()).sum::<f64>(),
                    "{error}"
                );
                let negative: Vec<_> = input.iter().map(|v| -v).collect();
                let mut negated = vec![0.0; 128];
                transition.apply(&negative, &mut negated);
                for (a, b) in output.iter().zip(&negated) {
                    assert_eq!(*a, -*b);
                }
                let other: Vec<_> = (0..128)
                    .map(|_| (rng.next_in_range(2001) as f64 - 1000.0) / 1000.0 * scale)
                    .collect();
                let combined: Vec<_> = input.iter().zip(&other).map(|(a, b)| a + b).collect();
                let mut sum = vec![0.0; 128];
                transition.apply(&other, &mut negated);
                transition.apply(&combined, &mut sum);
                let additive_error: f64 = sum
                    .iter()
                    .zip(output.iter().zip(&negated))
                    .map(|(combined, (a, b))| (combined - (a + b)).abs())
                    .sum();
                let magnitude: f64 = input.iter().chain(&other).map(|v| v.abs()).sum();
                assert!(additive_error <= 2e-13 * magnitude);
                transition.apply(&vec![0.0; 128], &mut output);
                assert!(
                    output.iter().all(|v| *v == 0.0),
                    "no stale range or dangling state"
                );
            }
        }
    }
}
