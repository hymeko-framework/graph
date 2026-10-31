//! Exact one-dimension completion of a binary target quotient.
//!
//! Let F be the free span and T=span(F, required), with d=rank(T)-rank(F).
//! Any solution with <=d+1 paid columns either lies inside T, or spans exactly
//! one dimension outside T. Over GF(2) all its nonzero residuals modulo T are
//! equal. Testing the zero class and each residual class is therefore exhaustive
//! for this finite column catalogue and budget, not for general circuit synthesis.
use super::*;

pub(super) struct Completion {
    pub lower_bound: usize,
    pub classes: usize,
    pub exhaustive: bool,
    pub candidate: Option<BinaryBasis>,
}

pub(super) struct QuotientSearch<'a, 'ir, 'work> {
    pub view: MatroidRef<'a, 'ir>,
    pub ground: &'work [(HyperArcId, f64)],
    pub free: &'work BTreeSet<HyperArcId>,
    pub available: &'work BTreeSet<HyperArcId>,
    pub required: &'work BTreeSet<HyperArcId>,
    pub workspace: &'work mut BinaryMatroidWorkspace,
}
impl QuotientSearch<'_, '_, '_> {
    fn target_basis(&mut self) -> Result<BinaryBasis, MatroidError> {
        let ground: Vec<_> = self
            .free
            .union(self.required)
            .map(|a| (*a, if self.free.contains(a) { 0.0 } else { 1.0 }))
            .collect();
        binary_weighted_basis(self.view, &ground, self.workspace)
    }

    fn admit(
        &mut self,
        eligible: &BTreeSet<HyperArcId>,
        goal: usize,
    ) -> Result<Option<BinaryBasis>, MatroidError> {
        let allowed: BTreeSet<_> = self.free.union(eligible).copied().collect();
        let ground: Vec<_> = allowed
            .union(self.required)
            .map(|a| {
                (
                    *a,
                    if self.free.contains(a) {
                        0.0
                    } else if eligible.contains(a) {
                        1.0
                    } else {
                        1000.0
                    },
                )
            })
            .collect();
        let local = binary_weighted_basis(self.view, &ground, self.workspace)?;
        let paid = objective(&local, self.free, self.required).0;
        if paid.len() > goal || paid.iter().any(|a| !eligible.contains(a)) {
            return Ok(None);
        }
        // Extend the admitted basis to the entire original ground set. Targets
        // are already spanned before other columns enter, preserving their
        // cheap coordinates while supplying the consumer's complete witness.
        let weighted: Vec<_> = self
            .ground
            .iter()
            .map(|(a, _)| {
                (
                    *a,
                    if self.free.contains(a) {
                        0.0
                    } else if eligible.contains(a) {
                        1.0
                    } else if self.available.contains(a) {
                        100.0
                    } else {
                        1000.0
                    },
                )
            })
            .collect();
        let mut full = binary_weighted_basis(self.view, &weighted, self.workspace)?;
        let paid = objective(&full, self.free, self.required).0;
        if paid.len() > goal || paid.iter().any(|a| !eligible.contains(a)) {
            return Err(MatroidError::InvalidReconstructionRequest);
        }
        let weights: BTreeMap<_, _> = self.ground.iter().copied().collect();
        full.weight = full.elements.iter().map(|a| weights[a]).sum();
        if !full.weight.is_finite() {
            return Err(MatroidError::NonFiniteWeight {
                arc: *full
                    .elements
                    .first()
                    .ok_or(MatroidError::InvalidReconstructionRequest)?,
            });
        }
        Ok(Some(full))
    }

    pub fn solve(&mut self, goal: usize) -> Result<Completion, MatroidError> {
        let target = self.target_basis()?;
        let lower_bound = target
            .elements
            .len()
            .checked_sub(self.free.len())
            .ok_or(MatroidError::InvalidReconstructionRequest)?;
        let mut result = Completion {
            lower_bound,
            classes: 0,
            exhaustive: goal <= lower_bound + 1,
            candidate: None,
        };
        if goal < lower_bound || !result.exhaustive {
            return Ok(result);
        }
        let target_elements: BTreeSet<_> = target.elements.into_iter().collect();
        let preferred: Vec<_> = self
            .ground
            .iter()
            .map(|(a, _)| {
                (
                    *a,
                    if self.free.contains(a) {
                        0.0
                    } else if self.required.contains(a) {
                        1.0
                    } else {
                        2.0
                    },
                )
            })
            .collect();
        let basis = binary_weighted_basis(self.view, &preferred, self.workspace)?;
        if !target_elements.iter().all(|a| basis.elements.contains(a)) {
            return Err(MatroidError::InvalidReconstructionRequest);
        }
        let mut classes = BTreeMap::<Vec<HyperArcId>, BTreeSet<HyperArcId>>::new();
        for row in &basis.coordinates {
            if self.available.contains(&row.arc) {
                let residual = row
                    .support
                    .iter()
                    .filter(|a| !target_elements.contains(a))
                    .copied()
                    .collect();
                classes.entry(residual).or_default().insert(row.arc);
            }
        }
        let inside = classes.remove(&Vec::new()).unwrap_or_default();
        result.classes += 1;
        result.candidate = self.admit(&inside, goal)?;
        if result.candidate.is_some() || goal == lower_bound {
            return Ok(result);
        }
        for class in classes.into_values() {
            let eligible: BTreeSet<_> = inside.union(&class).copied().collect();
            result.classes += 1;
            if eligible.len() < lower_bound {
                continue;
            }
            result.candidate = self.admit(&eligible, goal)?;
            if result.candidate.is_some() {
                break;
            }
        }
        Ok(result)
    }
}
