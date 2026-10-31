//! Bounded basis exchanges for reconstructing required columns from a free prefix.
mod quotient;
use super::{BinaryBasis, BinaryMatroidWorkspace, MatroidError, MatroidRef, binary_weighted_basis};
use hymeko::common::ids::HyperArcId;
use std::collections::{BTreeMap, BTreeSet};

/// Resource and exploration limits for the one reconstruction search.
#[derive(Clone, Copy, Debug)]
pub struct BinaryReconstructionOptions {
    /// Total frontier width; different paid-cost tiers share these slots.
    pub beam: usize,
    /// Maximum number of attempted elementary exchanges.
    pub max_exchanges: usize,
    /// Per-basis attempt cap; large neighborhoods must leave budget for later levels.
    pub exchanges_per_state: usize,
    /// Maximum admitted increase above the initial paid-column count.
    pub max_extra_products: usize,
    /// Maximum number of distinct complete bases retained in the visited set.
    pub max_states: usize,
    /// Optional caller goal. Reaching it is not a proof of optimality.
    pub target_cost: Option<usize>,
    /// Starting offset of the rotating coordinate scan, for bounded multi-start searches.
    pub column_offset: usize,
}
impl Default for BinaryReconstructionOptions {
    fn default() -> Self {
        Self {
            beam: 4,
            max_exchanges: 4096,
            exchanges_per_state: 256,
            max_extra_products: 2,
            max_states: 16384,
            target_cost: None,
            column_offset: 0,
        }
    }
}

/// Bounded search result. Any quotient certificate applies only to the supplied
/// fixed free prefix and finite available-column catalogue, not quantum circuits.
#[derive(Debug)]
pub struct BinaryReconstructionSearch {
    /// Rank(required modulo free), valid for this fixed function-prefix model.
    pub lower_bound: usize,
    /// Paid-column goal examined by the quotient relaxation.
    pub relaxation_target: usize,
    /// Residual classes checked by the exact one-dimension completion.
    pub quotient_classes: usize,
    /// The finite supplied column catalogue cannot meet relaxation_target.
    pub target_infeasible_certified: bool,
    /// Initial additive-weight basis, before the required-column objective.
    pub baseline: BinaryBasis,
    /// Up to the requested beam width of retained exact certificates.
    pub candidates: Vec<BinaryBasis>,
    /// Number of fundamental-circuit substitutions attempted, including repeats.
    pub exchanges_evaluated: usize,
    /// The caller's exchange cap stopped further expansion.
    pub budget_reached: bool,
    /// Complete basis identities visited, including inactive basis columns.
    pub states_visited: usize,
    /// State storage, rather than exchange count, stopped the search.
    pub state_budget_reached: bool,
    /// The caller's requested cost was reached; not an optimality claim.
    pub target_reached: bool,
    /// Highest paid-column count admitted during exploration.
    pub maximum_paid_columns: usize,
    /// A per-basis attempt cap truncated at least one neighborhood.
    pub neighborhood_capped: bool,
}

fn objective(
    b: &BinaryBasis,
    free: &BTreeSet<HyperArcId>,
    required: &BTreeSet<HyperArcId>,
) -> (Vec<HyperArcId>, usize) {
    let mut used = BTreeSet::new();
    let mut incidences = 0;
    for c in &b.coordinates {
        if required.contains(&c.arc) {
            incidences += c.support.len();
            used.extend(c.support.iter().filter(|a| !free.contains(a)).copied());
        }
    }
    (used.into_iter().collect(), incidences)
}

/// Required-column reconstructions: the computation a consumer actually emits.
/// Bases differing only in coordinates of non-required columns share it.
fn structure(b: &BinaryBasis, required: &BTreeSet<HyperArcId>) -> Vec<Vec<HyperArcId>> {
    b.coordinates
        .iter()
        .filter(|c| required.contains(&c.arc))
        .map(|c| c.support.clone())
        .collect()
}

fn exchange(
    b: &BinaryBasis,
    enter: HyperArcId,
    leave: HyperArcId,
    weights: &BTreeMap<HyperArcId, f64>,
) -> Result<BinaryBasis, MatroidError> {
    let pivot = &b
        .coordinates
        .iter()
        .find(|c| c.arc == enter)
        .ok_or(MatroidError::InvalidReconstructionRequest)?
        .support;
    let mut out = b.clone();
    *out.elements
        .iter_mut()
        .find(|a| **a == leave)
        .ok_or(MatroidError::InvalidReconstructionRequest)? = enter;
    out.weight = out.elements.iter().map(|a| weights[a]).sum();
    if !out.weight.is_finite() {
        return Err(MatroidError::NonFiniteWeight { arc: enter });
    }
    for c in &mut out.coordinates {
        if c.support.contains(&leave) {
            let mut support: BTreeSet<_> = c.support.iter().copied().collect();
            for &a in pivot {
                if !support.remove(&a) {
                    support.insert(a);
                }
            }
            support.insert(enter);
            c.support = support.into_iter().collect();
        }
    }
    Ok(out)
}

/// Search exact fundamental-circuit exchanges across complete basis states.
///
/// Free columns must be independent and present in the initial weighted basis.
/// `available` lists physically realizable paid columns; required-only columns
/// cannot be introduced as implementations. Cost is the number of distinct paid
/// columns used in required reconstructions, with support size as a tie-breaker.
/// Rank solving and source validation reuse `binary_weighted_basis`. Search is
/// preceded, for an explicit goal within one column of the rank lower bound,
/// by exhaustive GF(2) quotient-class completion. A completion meeting the goal
/// is returned directly. A negative completion is kept as a catalogue-scoped
/// certificate, and exchanges still enumerate reconstructions above the goal:
/// equal-cost structures can differ for later rewriting and lifetime stages.
/// Other goals use exchanges. With no explicit goal, the existing exploratory
/// exchange behaviour is kept. Retained candidates have distinct required
/// reconstructions, not merely distinct basis identities.
///
/// Exchange search is
/// allowed to leave the initial active span and temporarily increase paid cost
/// within `options.max_extra_products`. Cost tiers share the bounded frontier,
/// while the incumbent is retained separately. This is not an ABB lower bound.
pub fn binary_reconstruction_search(
    m: MatroidRef<'_, '_>,
    ground: &[(HyperArcId, f64)],
    free: &[HyperArcId],
    available: &[HyperArcId],
    required: &[HyperArcId],
    ws: &mut BinaryMatroidWorkspace,
    options: BinaryReconstructionOptions,
) -> Result<BinaryReconstructionSearch, MatroidError> {
    if !(1..=32).contains(&options.beam)
        || !(1..=1_000_000).contains(&options.max_exchanges)
        || options.exchanges_per_state == 0
        || options.max_extra_products > 16
        || !(1..=262144).contains(&options.max_states)
        || ground.len() > 2048
        || required.is_empty()
    {
        return Err(MatroidError::InvalidReconstructionRequest);
    }
    let free: BTreeSet<_> = free.iter().copied().collect();
    let available: BTreeSet<_> = available.iter().copied().collect();
    let required: BTreeSet<_> = required.iter().copied().collect();
    let weights: BTreeMap<_, _> = ground.iter().copied().collect();
    if !free.is_disjoint(&available)
        || free
            .iter()
            .chain(&available)
            .chain(&required)
            .any(|a| !weights.contains_key(a))
    {
        return Err(MatroidError::InvalidReconstructionRequest);
    }
    let baseline = binary_weighted_basis(m, ground, ws)?;
    let initial = objective(&baseline, &free, &required);
    if free.iter().any(|a| !baseline.elements.contains(a))
        || initial.0.iter().any(|a| !available.contains(a))
    {
        return Err(MatroidError::InvalidReconstructionRequest);
    }
    let key = |b: &BinaryBasis| {
        let (used, n) = objective(b, &free, &required);
        (used.len(), n, used)
    };
    let goal = options
        .target_cost
        .unwrap_or(initial.0.len().saturating_sub(1));
    let exact_goal = if options.target_cost.is_some() && options.max_states >= 2 {
        goal
    } else {
        usize::MAX
    };
    let completion = quotient::QuotientSearch {
        view: m,
        ground,
        free: &free,
        available: &available,
        required: &required,
        workspace: ws,
    }
    .solve(exact_goal)?;
    let mut result = BinaryReconstructionSearch {
        lower_bound: completion.lower_bound,
        relaxation_target: goal,
        quotient_classes: completion.classes,
        target_infeasible_certified: completion.exhaustive && completion.candidate.is_none(),
        candidates: vec![baseline.clone()],
        baseline,
        exchanges_evaluated: 0,
        budget_reached: false,
        states_visited: 1,
        state_budget_reached: false,
        target_reached: false,
        maximum_paid_columns: initial.0.len(),
        neighborhood_capped: false,
    };
    if let Some(candidate) = completion.candidate {
        result.target_reached = options
            .target_cost
            .is_some_and(|g| objective(&candidate, &free, &required).0.len() <= g);
        result.candidates = vec![candidate];
        result.states_visited = 2;
        return Ok(result);
    }
    let baseline = &result.baseline;
    let mut frontier = vec![baseline.clone()];
    let mut elite = frontier.clone();
    let identity = |b: &BinaryBasis| {
        b.elements
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    };
    // Bound identity storage and temporary certificate buckets structurally.
    let entries = baseline.elements.len()
        + baseline
            .coordinates
            .iter()
            .map(|c| c.support.len())
            .sum::<usize>();
    if baseline.elements.len().saturating_mul(options.max_states) > 8_000_000
        || entries.saturating_mul(options.beam * (options.max_extra_products + 3)) > 8_000_000
    {
        return Err(MatroidError::InvalidReconstructionRequest);
    }
    let ceiling = initial.0.len().saturating_add(options.max_extra_products);
    let mut seen = BTreeSet::from([identity(&baseline)]);
    let mut evaluated = 0;
    let mut capped = false;
    let mut state_capped = false;
    let mut reached = options
        .target_cost
        .is_some_and(|goal| initial.0.len() <= goal);
    let mut maximum_paid = initial.0.len();
    let mut neighborhood_capped = false;
    let mut cursor = options.column_offset;
    while !frontier.is_empty() && !capped && !state_capped && !reached {
        let mut tiers = BTreeMap::<usize, Vec<BinaryBasis>>::new();
        'expansion: for b in frontier {
            let first_exchange = evaluated;
            let offset = cursor % b.coordinates.len();
            for position in 0..b.coordinates.len() {
                let slot = (offset + position) % b.coordinates.len();
                cursor = slot + 1;
                let c = &b.coordinates[slot];
                if !available.contains(&c.arc) || b.elements.contains(&c.arc) {
                    continue;
                }
                for &leave in c.support.iter().filter(|a| !free.contains(a)) {
                    if evaluated == options.max_exchanges {
                        capped = true;
                        break 'expansion;
                    }
                    if evaluated - first_exchange == options.exchanges_per_state {
                        neighborhood_capped = true;
                        continue 'expansion;
                    }
                    evaluated += 1;
                    let candidate = exchange(&b, c.arc, leave, &weights)?;
                    let candidate_key = key(&candidate);
                    if candidate_key.0 > ceiling || seen.contains(&identity(&candidate)) {
                        continue;
                    }
                    if seen.len() == options.max_states {
                        state_capped = true;
                        break 'expansion;
                    }
                    seen.insert(identity(&candidate));
                    maximum_paid = maximum_paid.max(candidate_key.0);
                    reached = options
                        .target_cost
                        .is_some_and(|goal| candidate_key.0 <= goal);
                    let tier = tiers.entry(candidate_key.0).or_default();
                    tier.push(candidate);
                    tier.sort_by_key(&key);
                    tier.truncate(options.beam);
                    if reached {
                        break 'expansion;
                    }
                }
            }
        }
        elite.extend(tiers.values().flatten().cloned());
        elite.sort_by_key(&key);
        let mut unique = BTreeSet::new();
        elite.retain(|b| unique.insert(structure(b, &required)));
        elite.truncate(options.beam);
        // Round-robin across cost tiers admits bounded exploratory steps even
        // when the cheapest tier has enough candidates to fill the beam.
        frontier = Vec::new();
        for rank in 0..options.beam {
            for tier in tiers.values() {
                if frontier.len() == options.beam {
                    break;
                }
                if let Some(candidate) = tier.get(rank) {
                    frontier.push(candidate.clone());
                }
            }
        }
    }
    result.candidates = elite;
    result.exchanges_evaluated = evaluated;
    result.budget_reached = capped;
    result.states_visited = seen.len();
    result.state_budget_reached = state_capped;
    result.target_reached = reached;
    result.maximum_paid_columns = maximum_paid;
    result.neighborhood_capped = neighborhood_capped;
    Ok(result)
}
