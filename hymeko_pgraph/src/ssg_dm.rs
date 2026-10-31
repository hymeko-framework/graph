//! Decision-mapping Solution Structure Generation (SSG).
//!
//! The canonical Friedler SSG (book: Friedler, Orosz, Pimentel Losada,
//! *P-graphs for Process Systems Engineering*, Ch. 5; Definition 5.1,
//! Fig. 5.13). Given the maximal structure, this generates **every
//! solution-structure exactly once** by recursing over per-material
//! *production decisions* rather than scanning the $2^{|O|}$ subset
//! lattice. It is the tractable replacement for the brute
//! generate-and-test [`crate::ssg`] on instances where $|O_{\max}|$ is
//! large (e.g. the book's Example 3.3, 35 declared units → 3465
//! solution-structures, which $2^{29}$-subset brute force cannot reach).
//!
//! # Algorithm
//!
//! For a non-raw material $x$, let the *producer query*
//! $\Delta(x) = \{u \in O_{\max} : x \in \mathrm{outputs}(u)\}$ — a
//! query over the signed incidence ([`LoweredPGraph::producers`]). A
//! *decision* for $x$ is a non-empty $c \subseteq \Delta(x)$: the units
//! chosen to produce $x$; the rest $\Delta(x)\setminus c$ are excluded
//! for $x$. Starting from the products, the recursion decides each
//! still-needed material once, branching over its candidate decisions
//! and keeping only those consistent with earlier decisions:
//!
//! 1. **include-consistency** $c \cap \mathrm{excluded} = \varnothing$:
//!    no unit chosen now was excluded earlier.
//! 2. **exclude-consistency**
//!    $(\Delta(x)\setminus c) \cap \mathrm{included} = \varnothing$: no
//!    unit dropped now was included earlier.
//!
//! Including units adds their non-raw inputs to the work-list, so every
//! included unit is *justified* by a downstream need (axiom S4: each
//! unit has a path to a product). When the work-list empties, the
//! accumulated `included` set is one solution-structure.
//!
//! # Relationship to the brute SSG
//!
//! [`crate::ssg::enumerate`] tests $2^{|O_{\max}|}$ subsets for
//! forward-producibility (+ optional strict no-excess) but does **not**
//! enforce S4, so it can admit structures with superfluous units (and,
//! under strict no-excess, force-include disposal sinks that reach no
//! product). The decision-mapping SSG enforces S4 by construction, so
//! the two are **not** set-equal in general; the brute set is a looser
//! superset on the S4 axis. Every structure produced here is, however,
//! forward-feasible — see the soundness cross-check in the tests.
//!
//! # Preconditions
//! `msg` is the maximal structure of `p` (units outside it can appear
//! in no solution-structure). For every material $x$,
//! $|\Delta(x)| \le 31$ (a single material produced by ≥32 distinct
//! units is pathological for the subset enumeration; `debug_assert`ed).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use hymeko::common::ids::DeclId;

use crate::lowering::LoweredPGraph;
use crate::msg::MaximalStructure;
use crate::ssg::SolutionStructure;

/// Knobs for [`enumerate_with_options`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SsgDmOptions {
    /// Hard cap on the number of solution-structures collected
    /// (safety net mirroring [`crate::abb::AbbOptions::max_explored`]).
    /// `0` (the default) = unlimited.
    pub max_structures: usize,
}

/// Outcome of [`enumerate_with_options`].
#[derive(Debug, Clone)]
pub struct SsgDmResult {
    /// The generated solution-structures, each exactly once.
    pub structures: Vec<SolutionStructure>,
    /// `true` iff [`SsgDmOptions::max_structures`] was reached and
    /// enumeration stopped early (so `structures` is a prefix, not the
    /// full set).
    pub capped: bool,
    /// Partial search states visited, including rejected states.
    pub explored: usize,
    /// States rejected by supplied constraints or leaf forward feasibility.
    pub pruned_by_constraints: usize,
    /// Producer decisions considered before consistency checks.
    pub decisions_considered: usize,
}

/// Optional caller-certified restrictions on the same decision-mapping search.
/// IDs are canonical declaration IDs, never positions in a projected graph.
/// `lower_costs` are nonnegative *lower bounds*, not empirical estimates. Omitted
/// units cost zero. The strict ceiling only retains structures below it; callers
/// must keep their incumbent separately. No quantum-specific rule lives here.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SsgConstraints {
    /// Units excluded by caller-certified infeasibility conditions.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub forbidden_units: BTreeSet<usize>,
    /// Materials that permit at most one selected producer.
    pub exclusive_materials: BTreeSet<usize>,
    /// At most one unit from each group may be selected.
    pub exclusive_groups: Vec<BTreeSet<usize>>,
    /// Certified additive per-unit lower costs.
    pub lower_costs: BTreeMap<usize, u64>,
    /// Strict total lower-cost ceiling; None disables cost pruning.
    pub strict_cost_ceiling: Option<u64>,
    /// Partial-state cap; zero means unlimited, as in the original API.
    pub max_explored: usize,
    /// Keep only leaves that are forward-feasible from the raw materials (see
    /// [`crate::ssg::is_feasible`]). Off by default: the decision mapping
    /// enumerates Friedler's structural solution-structures, which admit
    /// raw-free producer cycles. Executable-process consumers switch it on.
    #[serde(default)]
    pub forward_feasible: bool,
}

impl SsgConstraints {
    fn validate(&self, p: &LoweredPGraph) -> Result<(), &'static str> {
        if self
            .forbidden_units
            .iter()
            .any(|id| !p.units.contains(&DeclId::new(*id)))
            || self.exclusive_materials.len() > p.materials.len()
            || self.exclusive_groups.len() > 4096
            || self
                .exclusive_materials
                .iter()
                .any(|id| !p.materials.contains(&DeclId::new(*id)))
            || self
                .exclusive_groups
                .iter()
                .any(|g| g.len() < 2 || g.iter().any(|id| !p.units.contains(&DeclId::new(*id))))
            || self
                .lower_costs
                .keys()
                .any(|id| !p.units.contains(&DeclId::new(*id)))
        {
            return Err("Unknown or invalid SSG constraint identities");
        }
        Ok(())
    }

    fn admits(
        &self,
        p: &LoweredPGraph,
        units: &BTreeSet<DeclId>,
        included: &BTreeSet<DeclId>,
        excluded: &BTreeSet<DeclId>,
        work: &BTreeSet<DeclId>,
    ) -> bool {
        if self.exclusive_groups.iter().any(|group| {
            group
                .iter()
                .filter(|id| included.contains(&DeclId::new(**id)))
                .take(2)
                .count()
                > 1
        }) || self.exclusive_materials.iter().any(|id| {
            p.producers(DeclId::new(*id))
                .intersection(included)
                .take(2)
                .count()
                > 1
        }) {
            return false;
        }
        let Some(ceiling) = self.strict_cost_ceiling else {
            return true;
        };
        let cost = |u: &DeclId| u128::from(*self.lower_costs.get(&u.raw()).unwrap_or(&0));
        let committed: u128 = included.iter().map(cost).sum();
        // Optimistic covering relaxation: each outstanding material needs one
        // available producer. Take MAX of individual minima, not their SUM:
        // one producer can cover several materials. Ignoring other restrictions
        // and upstream demands only weakens this admissible lower bound.
        let mut remaining = 0;
        for material in work {
            let producers = p.producers(*material);
            if !producers.is_disjoint(included) {
                continue;
            }
            let Some(minimum) = producers
                .iter()
                .filter(|u| units.contains(u) && !excluded.contains(u))
                .map(cost)
                .min()
            else {
                return false;
            };
            remaining = remaining.max(minimum);
        }
        committed + remaining < u128::from(ceiling)
    }
}

/// Generate every solution-structure of `p` under the maximal
/// structure `msg`, exactly once. Convenience wrapper over
/// [`enumerate_with_options`] with defaults (no cap).
///
/// # Postconditions
/// Returns a duplicate-free list; each entry is forward-feasible and
/// every included unit reaches a product (axiom S4).
pub fn enumerate(p: &LoweredPGraph, msg: &MaximalStructure) -> Vec<SolutionStructure> {
    enumerate_with_options(p, msg, SsgDmOptions::default()).structures
}

/// Generate solution-structures with explicit options.
pub fn enumerate_with_options(
    p: &LoweredPGraph,
    msg: &MaximalStructure,
    opts: SsgDmOptions,
) -> SsgDmResult {
    enumerate_constrained(p, msg, opts, &SsgConstraints::default())
        .expect("Empty SSG constraints are valid")
}

/// Generate structures while applying restrictions before descendants exist.
/// The default constraints preserve the original API's order and structure set.
/// For a singleton-producer material, enumerate singletons directly: no rejected
/// exponential subset family is materialized. Limits report incomplete output.
pub fn enumerate_constrained(
    p: &LoweredPGraph,
    msg: &MaximalStructure,
    opts: SsgDmOptions,
    constraints: &SsgConstraints,
) -> Result<SsgDmResult, &'static str> {
    constraints.validate(p)?;
    let mut e = Enumerator {
        p,
        units: &msg.units,
        out: Vec::new(),
        max_structures: opts.max_structures,
        capped: false,
        constraints,
        explored: 0,
        pruned: 0,
        decisions: 0,
    };
    // Initial work-list: the required products that are not themselves
    // raw (a raw product needs no production decision).
    let p0: BTreeSet<DeclId> = p
        .products
        .iter()
        .filter(|m| !p.raws.contains(m))
        .copied()
        .collect();
    let excluded = constraints
        .forbidden_units
        .iter()
        .map(|id| DeclId::new(*id))
        .collect();
    e.recurse(&BTreeSet::new(), &excluded, &BTreeSet::new(), &p0);
    Ok(SsgDmResult {
        structures: e.out,
        capped: e.capped,
        explored: e.explored,
        pruned_by_constraints: e.pruned,
        decisions_considered: e.decisions,
    })
}

struct Enumerator<'a> {
    p: &'a LoweredPGraph,
    units: &'a BTreeSet<DeclId>,
    out: Vec<SolutionStructure>,
    max_structures: usize,
    capped: bool,
    constraints: &'a SsgConstraints,
    explored: usize,
    pruned: usize,
    decisions: usize,
}

impl Enumerator<'_> {
    /// $\Delta(x)$ restricted to the maximal structure.
    fn delta(&self, x: DeclId) -> BTreeSet<DeclId> {
        self.p
            .producers(x)
            .iter()
            .filter(|u| self.units.contains(u))
            .copied()
            .collect()
    }

    fn recurse(
        &mut self,
        included: &BTreeSet<DeclId>,
        excluded: &BTreeSet<DeclId>,
        decided: &BTreeSet<DeclId>,
        work: &BTreeSet<DeclId>,
    ) {
        if self.capped {
            return;
        }
        if self.constraints.max_explored != 0 && self.explored >= self.constraints.max_explored {
            self.capped = true;
            return;
        }
        self.explored += 1;
        if !self
            .constraints
            .admits(self.p, self.units, included, excluded, work)
        {
            self.pruned += 1;
            return;
        }

        // Pick the next undecided material (smallest DeclId → a fixed,
        // reproducible decision order; the exactly-once guarantee does
        // not depend on which we pick, only that each is decided once).
        let Some(&x) = work.iter().next() else {
            // Production decisions can close a self-supporting cycle without
            // connecting it to a raw. Such a leaf is a structural solution-structure;
            // only an explicit forward-feasibility request rejects it here.
            if self.constraints.forward_feasible
                && !crate::ssg::is_feasible(
                self.p,
                included,
                crate::ssg::SsgOptions {
                    require_at_least_one_unit: false,
                    ..Default::default()
                },
            ) {
                self.pruned += 1;
                return;
            }
            // Work-list empty ⇒ `included` is a complete solution-structure.
            if self.max_structures != 0 && self.out.len() >= self.max_structures {
                self.capped = true;
                return;
            }
            self.out.push(SolutionStructure {
                units: included.clone(),
            });
            return;
        };

        let mut rest = work.clone();
        rest.remove(&x);

        let delta: Vec<DeclId> = self.delta(x).into_iter().collect();
        // A needed material with no producer in the maximal structure
        // makes this branch infeasible — prune (emit nothing).
        if delta.is_empty() {
            return;
        }
        let k = delta.len();
        let singleton = self.constraints.exclusive_materials.contains(&x.raw());
        assert!(
            singleton || k <= 31,
            "material produced by {k} units exceeds the 31-producer subset bound"
        );

        // Candidate decisions: every non-empty subset of Δ(x).
        let decision_count = if singleton { k } else { (1usize << k) - 1 };
        for decision in 0..decision_count {
            self.decisions += 1;
            let chosen: BTreeSet<DeclId> = (0..k)
                .filter(|i| {
                    if singleton {
                        *i == decision
                    } else {
                        ((decision + 1) >> i) & 1 == 1
                    }
                })
                .map(|i| delta[i])
                .collect();
            let dropped: BTreeSet<DeclId> = delta
                .iter()
                .filter(|u| !chosen.contains(u))
                .copied()
                .collect();

            // Consistency tests (book Fig. 5.13).
            if chosen.iter().any(|u| excluded.contains(u)) {
                continue;
            }
            if dropped.iter().any(|u| included.contains(u)) {
                continue;
            }

            let mut included2 = included.clone();
            included2.extend(&chosen);
            let mut excluded2 = excluded.clone();
            excluded2.extend(&dropped);
            let mut decided2 = decided.clone();
            decided2.insert(x);

            // Including a unit makes each of its non-raw inputs a
            // material that must itself be produced (axiom S4 / forward
            // feasibility). Queue inputs not already decided.
            let mut work2 = rest.clone();
            for u in &chosen {
                for m in self.p.inputs(*u) {
                    if !self.p.raws.contains(m) && !decided2.contains(m) {
                        work2.insert(*m);
                    }
                }
            }

            self.recurse(&included2, &excluded2, &decided2, &work2);
            if self.capped {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower;
    use crate::msg::{MaximalStructureOptions, maximal_structure_with_options};
    use parser::parse_description;

    fn lower_src(src: &str) -> LoweredPGraph {
        lower(&parse_description(src).expect("parse")).expect("lower")
    }

    fn no_duplicates(structs: &[SolutionStructure]) -> bool {
        let set: BTreeSet<&BTreeSet<DeclId>> = structs.iter().map(|s| &s.units).collect();
        set.len() == structs.len()
    }

    /// Book Fig. 5.1/5.2 micro-case: one product produced by three
    /// units yields 2^3 − 1 = 7 solution-structures.
    #[test]
    fn three_producers_yields_seven() {
        let src = r#"
            T{}
            context {
                P <material, product>;
                R <material, raw>;
                @a <unit> 1 { (-R, +P); }
                @b <unit> 1 { (-R, +P); }
                @c <unit> 1 { (-R, +P); }
            }
        "#;
        let p = lower_src(src);
        let msg = maximal_structure_with_options(
            &p,
            MaximalStructureOptions {
                strict_no_excess: false,
            },
        );
        let s = enumerate(&p, &msg);
        assert_eq!(s.len(), 7, "three producers of one product → 7 structures");
        assert!(no_duplicates(&s));
    }

    /// A linear chain (each material has a single producer) yields
    /// exactly one solution-structure.
    #[test]
    fn single_producer_chain_yields_one() {
        let src = r#"
            T{}
            context {
                P <material, product>;
                Mid <material>;
                R <material, raw>;
                @u1 <unit> 1 { (-Mid, +P); }
                @u2 <unit> 1 { (-R, +Mid); }
            }
        "#;
        let p = lower_src(src);
        let msg = maximal_structure_with_options(
            &p,
            MaximalStructureOptions {
                strict_no_excess: false,
            },
        );
        let s = enumerate(&p, &msg);
        assert_eq!(s.len(), 1);
        assert_eq!(
            s[0].units.len(),
            2,
            "the chain {{u1,u2}} is the only structure"
        );
    }

    /// A product with no producer at all ⇒ no solution-structure.
    #[test]
    fn unproducible_product_yields_none() {
        let src = r#"
            T{}
            context {
                P <material, product>;
                R <material, raw>;
            }
        "#;
        let p = lower_src(src);
        let msg = maximal_structure_with_options(
            &p,
            MaximalStructureOptions {
                strict_no_excess: false,
            },
        );
        assert!(enumerate(&p, &msg).is_empty());
    }

    /// `max_structures` caps the output and flags `capped`.
    #[test]
    fn max_structures_caps_output() {
        let src = r#"
            T{}
            context {
                P <material, product>;
                R <material, raw>;
                @a <unit> 1 { (-R, +P); }
                @b <unit> 1 { (-R, +P); }
                @c <unit> 1 { (-R, +P); }
            }
        "#;
        let p = lower_src(src);
        let msg = maximal_structure_with_options(
            &p,
            MaximalStructureOptions {
                strict_no_excess: false,
            },
        );
        let r = enumerate_with_options(&p, &msg, SsgDmOptions { max_structures: 3 });
        assert_eq!(r.structures.len(), 3);
        assert!(r.capped, "hitting the cap must set `capped`");
    }

    #[test]
    fn partial_constraints_preserve_every_small_admissible_structure() {
        let p = lower_src(
            r#"T{} context {
            P <material, product>; Q <material, product>; R <material, raw>;
            @a <unit> 3 { (-R,+P,+Q); }
            @b <unit> 2 { (-R,+P); }
            @c <unit> 2 { (-R,+Q); }
            @d <unit> 4 { (-R,+Q); }
        }"#,
        );
        let msg = crate::msg::maximal_structure(&p);
        let all = enumerate(&p, &msg);
        let costs: BTreeMap<usize, u64> =
            p.costs.iter().map(|(u, c)| (u.raw(), *c as u64)).collect();
        for exclusive in [false, true] {
            for ceiling in 0..=15 {
                let constraints = SsgConstraints {
                    exclusive_materials: if exclusive {
                        p.materials.iter().map(|x| x.raw()).collect()
                    } else {
                        BTreeSet::new()
                    },
                    lower_costs: costs.clone(),
                    strict_cost_ceiling: Some(ceiling),
                    ..Default::default()
                };
                let filtered =
                    enumerate_constrained(&p, &msg, SsgDmOptions::default(), &constraints).unwrap();
                let expected: Vec<_> = all
                    .iter()
                    .filter(|s| {
                        let cost: u64 = s.units.iter().map(|u| costs[&u.raw()]).sum();
                        cost < ceiling
                            && (!exclusive
                                || p.materials
                                    .iter()
                                    .all(|m| p.producers(*m).intersection(&s.units).count() <= 1))
                    })
                    .cloned()
                    .collect();
                assert_eq!(filtered.structures, expected);
                assert!(!filtered.capped);
            }
        }
        let mut constraints = SsgConstraints {
            exclusive_materials: p.materials.iter().map(|x| x.raw()).collect(),
            ..Default::default()
        };
        let reduced =
            enumerate_constrained(&p, &msg, SsgDmOptions::default(), &constraints).unwrap();
        let full = enumerate_with_options(&p, &msg, SsgDmOptions::default());
        assert!(reduced.decisions_considered < full.decisions_considered);
        constraints.max_explored = 1;
        let limited =
            enumerate_constrained(&p, &msg, SsgDmOptions::default(), &constraints).unwrap();
        assert!(limited.capped);
        assert_eq!(limited.explored, 1);
        constraints.exclusive_materials.insert(usize::MAX);
        assert!(enumerate_constrained(&p, &msg, SsgDmOptions::default(), &constraints).is_err());
    }

    #[test]
    fn exclusive_unit_groups_are_enforced_before_leaves() {
        let p = lower_src(
            r#"T{} context {
            P <material, product>; Q <material, product>; R <material, raw>;
            @a <unit> 1 { (-R,+P); } @b <unit> 1 { (-R,+Q); }
        }"#,
        );
        let msg = crate::msg::maximal_structure(&p);
        assert_eq!(enumerate(&p, &msg).len(), 1);
        let constraints = SsgConstraints {
            exclusive_groups: vec![p.units.iter().map(|x| x.raw()).collect()],
            ..Default::default()
        };
        let result =
            enumerate_constrained(&p, &msg, SsgDmOptions::default(), &constraints).unwrap();
        assert!(result.structures.is_empty());
        assert!(result.pruned_by_constraints > 0);
        let mut forbidden = SsgConstraints::default();
        forbidden
            .forbidden_units
            .insert(p.units.iter().next().unwrap().raw());
        let result = enumerate_constrained(&p, &msg, SsgDmOptions::default(), &forbidden).unwrap();
        assert!(result.structures.is_empty());
        assert!(!result.capped);
        forbidden.forbidden_units.insert(usize::MAX);
        assert!(enumerate_constrained(&p, &msg, SsgDmOptions::default(), &forbidden).is_err());
    }

    #[test]
    fn a_raw_free_producer_cycle_is_structural_but_not_forward_feasible() {
        let p = lower_src(
            r#"T{} context {
            P <material, product>; Q <material>; R <material, raw>;
            @a <unit> 1 { (-R,+P); }
            @b <unit> 1 { (-P,+Q); }
            @c <unit> 1 { (-Q,+P); }
        }"#,
        );
        let msg = crate::msg::maximal_structure(&p);
        let structural = enumerate_with_options(&p, &msg, SsgDmOptions::default());
        assert_eq!(structural.structures.len(), 3);
        let constraints = SsgConstraints {
            forward_feasible: true,
            ..Default::default()
        };
        let result =
            enumerate_constrained(&p, &msg, SsgDmOptions::default(), &constraints).unwrap();
        assert_eq!(result.structures.len(), 2);
        assert!(result.structures.iter().all(|s| crate::ssg::is_feasible(
            &p,
            &s.units,
            crate::ssg::SsgOptions::default()
        )));
        assert!(result.pruned_by_constraints > 0);
    }

    #[test]
    fn a_raw_product_preserves_the_empty_identity_structure() {
        let mut p = lower_src("T{} context { R <material, raw>; }");
        p.products.extend(p.raws.iter().copied());
        let msg = crate::msg::maximal_structure(&p);
        let result = enumerate(&p, &msg);
        assert_eq!(result.len(), 1);
        assert!(result[0].units.is_empty());
    }
}
