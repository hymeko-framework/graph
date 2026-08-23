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

use std::collections::BTreeSet;

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
    let mut e = Enumerator {
        p,
        units: &msg.units,
        out: Vec::new(),
        max_structures: opts.max_structures,
        capped: false,
    };
    // Initial work-list: the required products that are not themselves
    // raw (a raw product needs no production decision).
    let p0: BTreeSet<DeclId> = p
        .products
        .iter()
        .filter(|m| !p.raws.contains(m))
        .copied()
        .collect();
    e.recurse(&BTreeSet::new(), &BTreeSet::new(), &BTreeSet::new(), &p0);
    SsgDmResult {
        structures: e.out,
        capped: e.capped,
    }
}

struct Enumerator<'a> {
    p: &'a LoweredPGraph,
    units: &'a BTreeSet<DeclId>,
    out: Vec<SolutionStructure>,
    max_structures: usize,
    capped: bool,
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

        // Pick the next undecided material (smallest DeclId → a fixed,
        // reproducible decision order; the exactly-once guarantee does
        // not depend on which we pick, only that each is decided once).
        let Some(&x) = work.iter().next() else {
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
        debug_assert!(
            k <= 31,
            "material produced by {k} units exceeds the 31-producer subset bound"
        );

        // Candidate decisions: every non-empty subset of Δ(x).
        for mask in 1u32..(1u32 << k) {
            let chosen: BTreeSet<DeclId> = (0..k)
                .filter(|i| (mask >> i) & 1 == 1)
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
}
