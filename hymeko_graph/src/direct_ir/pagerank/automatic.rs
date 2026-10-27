//! Bounded method selection over the existing power and Tensor GMRES owners.
use super::{
    PageRank, PageRankConfig, PageRankError, PageRankResult, PageRankWorkspace, PowerState,
    Progress,
};

/// Controls for adaptive execution; these are work-model heuristics, not timing guarantees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageRankAutoConfig {
    /// Power updates between decisions, retained in the solution and total budget.
    pub probe_iterations: usize,
    /// Maximum Arnoldi steps in the adaptive Krylov phase and per restart.
    pub restart: usize,
    /// Cap on requested basis payload (nodes * directions * sizeof(f64)).
    /// Excludes other scratch, allocator overhead and previously retained capacity.
    /// Zero disables Krylov admission without allocating its buffers.
    pub max_basis_bytes: usize,
}
impl Default for PageRankAutoConfig {
    fn default() -> Self {
        Self {
            probe_iterations: 4,
            restart: 20,
            max_basis_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Reason for the adaptive decision; no case names or historical timings are consulted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageRankAutoReason {
    /// Empty domain or zero damping needs no iterative solve.
    Trivial,
    /// The existing parallel power implementation serves a nonserial workspace.
    ParallelPower,
    /// Power iteration passed canonical validation before a Krylov phase was selected.
    PowerConverged,
    /// Too little of the total budget remains for Krylov steps and validation.
    IterationBudget,
    /// Requested basis payload exceeds the cap or cannot be represented.
    BasisBudget,
    /// Predicted power work does not exceed one complete Krylov cycle.
    PowerEstimate,
    /// One Krylov cycle is cheaper under the nominal work estimate.
    KrylovEstimate,
}

/// Actual adaptive execution accounting, independent of elapsed time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageRankAutoSelection {
    /// Reason for the selected continuation.
    pub reason: PageRankAutoReason,
    /// Power updates before the final decision, included in `power_iterations`.
    pub probe_iterations: usize,
    /// Number of periodic selection checks, including a converged power return.
    pub selection_checks: usize,
    /// All executed power updates, including any post-Krylov completion.
    pub power_iterations: usize,
    /// Executed Arnoldi steps plus the Krylov candidate's canonical validation.
    pub gmres_iterations: usize,
    /// Predicted remaining power updates at selection; zero when not estimated.
    pub estimated_power_iterations: usize,
    /// Nominal remaining power work; not a measured runtime.
    pub estimated_power_work: u128,
    /// Nominal single-cycle Krylov work, including validation and vector work.
    pub estimated_gmres_work: u128,
    /// Requested basis bytes; None on overflow or when Krylov was not considered.
    pub basis_bytes: Option<usize>,
    /// Admitted directions, zero when no Krylov cycle is selected.
    pub restart: usize,
}
impl PageRankAutoSelection {
    fn new(reason: PageRankAutoReason) -> Self {
        Self {
            reason,
            probe_iterations: 0,
            selection_checks: 0,
            power_iterations: 0,
            gmres_iterations: 0,
            estimated_power_iterations: 0,
            estimated_power_work: 0,
            estimated_gmres_work: 0,
            basis_bytes: None,
            restart: 0,
        }
    }
}

/// Reuses the existing structural planner's incidence work, rather than scanning
/// or reconstructing another graph. Units approximate scalar arithmetic work.
struct WorkModel {
    nodes: usize,
    contraction: u128,
    canonical: u128,
}
impl WorkModel {
    fn from_rank(rank: &PageRank<'_>) -> Self {
        let direct = 2 * rank.incidence_count() as u128;
        // The certified <=2-term scatter uses ordinary additions. General
        // ordered and range contractions maintain compensated accumulators.
        let canonical = direct
            * if rank.incoming_term_bound().is_some_and(|n| n <= 2) {
                1
            } else {
                4
            };
        Self {
            nodes: rank.node_count(),
            contraction: rank.plan_stats().map_or(canonical, |p| 4 * p.selected_work),
            canonical,
        }
    }

    fn select(
        &self,
        policy: PageRankAutoConfig,
        config: PageRankConfig,
        state: &PowerState,
    ) -> PageRankAutoSelection {
        let mut selection = PageRankAutoSelection::new(PageRankAutoReason::IterationBudget);
        let remaining = config.max_iterations - state.progress.iterations;
        if remaining < 2 {
            return selection;
        }
        let directions = policy.restart.min(self.nodes).min(remaining - 1);
        selection.basis_bytes = self
            .nodes
            .checked_mul(directions)
            .and_then(|elements| elements.checked_mul(size_of::<f64>()));
        if selection
            .basis_bytes
            .is_none_or(|bytes| bytes > policy.max_basis_bytes)
        {
            selection.reason = PageRankAutoReason::BasisBudget;
            return selection;
        }
        // An observed contraction is a prediction, not a certified spectral bound.
        // Missing/stalled observations use the stochastic damping bound instead.
        let observed = state.progress.change_l1 / state.prior_change;
        let ratio = if state.prior_change.is_finite()
            && state.prior_change > 0.0
            && observed.is_finite()
            && observed >= 0.0
        {
            observed.min(config.damping)
        } else {
            config.damping
        };
        let target = config.tolerance * (1.0 - config.damping) / config.damping;
        let change = state.progress.change_l1;
        let predicted = if change <= target || ratio == 0.0 {
            1
        } else {
            ((target.ln() - change.ln()) / ratio.ln()).ceil().max(1.0) as usize
        }
        .min(remaining);
        selection.estimated_power_iterations = predicted;
        let n = self.nodes as u128;
        let m = directions as u128;
        // Compensated incidence reductions carry more scalar work than a plain
        // multiply/add. This fixed nominal weight is explicit and uncalibrated;
        // the estimate is never used as numerical convergence evidence.
        let contraction = self.contraction;
        // finish_update separately compensates change and mass, consumes the
        // incoming sum, forms teleport/delta and stages normalized next input.
        // Counting it like the simpler Krylov output sweep underprices power.
        let power_contraction = if state.certified {
            self.canonical
        } else {
            self.contraction
        };
        let power_step = power_contraction + 16 * n;
        selection.estimated_power_work = power_step.saturating_mul(predicted as u128);
        // One complete cycle: initial/final residuals, m Arnoldi applications,
        // MGS dots/axpys, basis updates and one canonical validation update.
        selection.estimated_gmres_work = (m + 2)
            .saturating_mul(contraction + 6 * n)
            .saturating_add(2 * n * m * (m + 1))
            .saturating_add(self.canonical + 16 * n);
        selection.reason = PageRankAutoReason::PowerEstimate;
        if selection.estimated_gmres_work < selection.estimated_power_work {
            selection.reason = PageRankAutoReason::KrylovEstimate;
            selection.restart = directions;
        }
        selection
    }
}

impl<'ir> PageRank<'ir> {
    pub(super) fn run_automatic<'w>(
        &self,
        config: PageRankConfig,
        workspace: &'w mut PageRankWorkspace,
        personalization: Option<&[f64]>,
        policy: PageRankAutoConfig,
    ) -> Result<PageRankResult<'w>, PageRankError> {
        config.validate()?;
        if policy.probe_iterations == 0 || policy.restart == 0 {
            return Err(PageRankError::InvalidConfig);
        }
        if workspace.worker_count() != 1 {
            let mut result = self.run_power(config, workspace, personalization, false)?;
            let mut selection = PageRankAutoSelection::new(PageRankAutoReason::ParallelPower);
            selection.power_iterations = result.progress.iterations;
            result.automatic = Some(selection);
            return Ok(result);
        }
        workspace.initialize(self.node_count(), personalization)?;
        let mut state = PowerState::new(self.execution.is_none());
        let mut selection = PageRankAutoSelection::new(PageRankAutoReason::Trivial);
        if self.node_count() == 0 || config.damping == 0.0 {
            state.progress.converged = true;
        } else {
            self.prepare_workspace(workspace);
            let model = WorkModel::from_rank(self);
            let mut checks = 0;
            loop {
                let stop = state
                    .progress
                    .iterations
                    .saturating_add(policy.probe_iterations);
                self.advance_power(config, workspace, None, &mut state, stop)?;
                checks += 1;
                if state.progress.converged {
                    selection = PageRankAutoSelection::new(PageRankAutoReason::PowerConverged);
                    break;
                }
                selection = model.select(policy, config, &state);
                if selection.reason != PageRankAutoReason::PowerEstimate {
                    break;
                }
                // Early fast modes need not predict the asymptotic contraction.
                // Recheck while retaining every computed update and state flag.
            }
            let probe = state.progress.iterations;
            selection.selection_checks = checks;
            if !state.progress.converged {
                if selection.restart > 0 {
                    let cycle_config = PageRankConfig {
                        max_iterations: selection.restart + 1,
                        ..config
                    };
                    let progress = self
                        .run_krylov_initialized(cycle_config, workspace, selection.restart)?
                        .progress;
                    selection.gmres_iterations = progress.iterations;
                    state.progress = Progress {
                        iterations: probe + progress.iterations,
                        ..progress
                    };
                    state.certified = true;
                    state.previous_change = progress.change_l1;
                }
                self.advance_power(config, workspace, None, &mut state, config.max_iterations)?;
            }
            selection.probe_iterations = probe;
            selection.power_iterations = state.progress.iterations - selection.gmres_iterations;
        }
        Ok(PageRankResult {
            scores: &workspace.scores,
            progress: state.progress,
            automatic: Some(selection),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_late_slow_mode_can_reverse_an_early_power_prediction() {
        let model = WorkModel {
            nodes: 65_536,
            contraction: 1_160_000,
            canonical: 8_000_000,
        };
        let config = PageRankConfig::default();
        let policy = PageRankAutoConfig::default();
        let mut state = PowerState::new(false);
        state.progress = Progress {
            iterations: 4,
            change_l1: 0.001,
            ..Default::default()
        };
        state.prior_change = 0.002;
        assert_eq!(
            model.select(policy, config, &state).reason,
            PageRankAutoReason::PowerEstimate
        );
        state.progress.iterations = 8;
        state.progress.change_l1 = 0.0001;
        state.prior_change = 0.0001 / 0.85;
        assert_eq!(
            model.select(policy, config, &state).reason,
            PageRankAutoReason::KrylovEstimate
        );
    }

    #[test]
    fn work_prediction_and_basis_admission_obey_generated_bounds() {
        let config = PageRankConfig::default();
        let mut state = PowerState::new(true);
        state.progress = Progress {
            iterations: 4,
            change_l1: 0.085,
            ..Default::default()
        };
        state.prior_change = 0.1;
        for nodes in [2, 8, 64, 128, 65_536] {
            for incidences in [1, 2, 16, 128] {
                let model = WorkModel {
                    nodes,
                    contraction: 2 * nodes as u128 * incidences,
                    canonical: 2 * nodes as u128 * incidences,
                };
                for restart in [1, 4, 20, 200] {
                    let policy = PageRankAutoConfig {
                        restart,
                        max_basis_bytes: usize::MAX,
                        ..Default::default()
                    };
                    let selected = model.select(policy, config, &state);
                    let bytes = selected.basis_bytes.unwrap();
                    assert!(selected.estimated_power_iterations <= config.max_iterations - 4);
                    assert_eq!(
                        selected.restart > 0,
                        selected.estimated_gmres_work < selected.estimated_power_work
                    );
                    let blocked = model.select(
                        PageRankAutoConfig {
                            max_basis_bytes: bytes - 1,
                            ..policy
                        },
                        config,
                        &state,
                    );
                    assert_eq!(blocked.reason, PageRankAutoReason::BasisBudget);
                    assert_eq!(blocked.restart, 0);
                    let admitted = model.select(
                        PageRankAutoConfig {
                            max_basis_bytes: bytes,
                            ..policy
                        },
                        config,
                        &state,
                    );
                    assert_eq!(
                        admitted, selected,
                        "admission is inclusive and deterministic"
                    );
                    let mut near = state;
                    near.progress.change_l1 = config.tolerance * 0.01;
                    let cheap = model.select(policy, config, &near);
                    assert_eq!(cheap.estimated_power_iterations, 1);
                    assert_eq!(cheap.reason, PageRankAutoReason::PowerEstimate);
                }
            }
        }
    }

    #[test]
    fn overflow_and_short_budgets_never_admit_a_basis() {
        let model = WorkModel {
            nodes: usize::MAX,
            contraction: 1,
            canonical: 1,
        };
        let mut state = PowerState::new(true);
        state.progress.iterations = 4;
        let config = PageRankConfig::default();
        let selection = model.select(PageRankAutoConfig::default(), config, &state);
        assert_eq!(selection.reason, PageRankAutoReason::BasisBudget);
        assert_eq!(selection.basis_bytes, None);
        for max_iterations in [4, 5] {
            let selected = model.select(
                PageRankAutoConfig::default(),
                PageRankConfig {
                    max_iterations,
                    ..config
                },
                &state,
            );
            assert_eq!(selected.reason, PageRankAutoReason::IterationBudget);
            assert_eq!(selected.restart, 0);
        }
    }
}
