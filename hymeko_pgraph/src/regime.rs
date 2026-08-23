//! P-graph solving **regimes** (Strategy pattern, 2026-05-27).
//!
//! The canonical Friedler maximal structure (reduction + composition,
//! [`crate::msg`]) is the *general* substrate for every P-graph problem.
//! A [`Regime`] is a pluggable strategy that layers an optional
//! *refinement* on top of it, distinguishing solving conventions without
//! a Cartesian explosion of boolean flags (CLAUDE.md §6.5 #1/#7, §7).
//!
//! Two regimes ship:
//!
//! - [`Canonical`] — the identity: the textbook PNS semantics (axioms
//!   S1–S5, no no-excess rule). The default for PNS work.
//! - [`NoExcess`] — the non-canonical "no-waste" refinement: drops units
//!   that produce a material neither demanded nor consumed, and rejects
//!   solution-structures bearing such excess. Used deliberately by the
//!   HSiKAN architecture search, where an injected by-product encodes
//!   "wasted potential" so that dominated architectures are pruned.
//!
//! The legacy `strict_no_excess: bool` options are an **Adapter** onto
//! these: see [`from_strict_flag`]. `false → Canonical`, `true → NoExcess`,
//! so the regime layer is behavior-preserving.

use std::collections::BTreeSet;

use hymeko::common::ids::DeclId;

use crate::lowering::LoweredPGraph;

/// A solving regime: a refinement strategy over the canonical maximal
/// structure and solution-structure feasibility.
///
/// # Invariants
/// Implementations must be pure functions of their arguments (no hidden
/// state) so that MSG/SSG/ABB remain deterministic.
pub trait Regime {
    /// Refine the canonical maximal structure (applied *after* the
    /// reduction + composition phases). The default keeps every unit.
    ///
    /// # Postconditions
    /// Returns a subset of `units`.
    fn refine_maximal(&self, _p: &LoweredPGraph, units: BTreeSet<DeclId>) -> BTreeSet<DeclId> {
        units
    }

    /// Extra admissibility predicate on a candidate solution-structure,
    /// applied *in addition to* base forward-feasibility (every input
    /// raw-or-produced; every product produced). The default admits all.
    fn structure_admissible(&self, _p: &LoweredPGraph, _sel: &BTreeSet<DeclId>) -> bool {
        true
    }

    /// Stable identifier (echoed in JSON dumps and reports).
    fn name(&self) -> &'static str;
}

/// The canonical Friedler regime — the general substrate (no refinement).
#[derive(Debug, Clone, Copy, Default)]
pub struct Canonical;

impl Regime for Canonical {
    fn name(&self) -> &'static str {
        "canonical"
    }
}

/// The non-canonical no-waste regime: every produced material must be a
/// product or consumed by another selected unit.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoExcess;

impl Regime for NoExcess {
    fn refine_maximal(&self, p: &LoweredPGraph, mut units: BTreeSet<DeclId>) -> BTreeSet<DeclId> {
        // Fixpoint: drop units producing any material that is neither a
        // product nor consumed by another surviving unit.
        loop {
            let before = units.len();
            let consumed: BTreeSet<DeclId> = units
                .iter()
                .flat_map(|u| p.inputs(*u).iter().copied())
                .collect();
            let useful: BTreeSet<DeclId> = consumed.union(&p.products).copied().collect();
            units.retain(|u| p.outputs(*u).iter().all(|m| useful.contains(m)));
            if units.len() == before {
                break;
            }
        }
        units
    }

    fn structure_admissible(&self, p: &LoweredPGraph, sel: &BTreeSet<DeclId>) -> bool {
        // Every produced non-product, non-raw material must be consumed
        // by some selected unit (the strict P-graph no-excess rule).
        let mut produced: BTreeSet<DeclId> = BTreeSet::new();
        for u in sel {
            produced.extend(p.outputs(*u).iter().copied());
        }
        let mut consumed: BTreeSet<DeclId> = BTreeSet::new();
        for u in sel {
            consumed.extend(p.inputs(*u).iter().copied());
        }
        for m in &produced {
            if p.products.contains(m) || p.raws.contains(m) {
                continue;
            }
            if !consumed.contains(m) {
                return false;
            }
        }
        true
    }

    fn name(&self) -> &'static str {
        "no-excess"
    }
}

/// Cost-dominance reduction — an **optimum-preserving** refinement.
///
/// Prunes a unit `u` from the maximal structure when some *other* surviving
/// unit `v` **dominates** it:
/// - `outputs(v) == outputs(u)` (interchangeable producers — equal output
///   sets, so the swap is *output-neutral*),
/// - `inputs(v) ⊆ inputs(u)` (v needs no more), and
/// - `cost(v) ≤ cost(u)` with a *strict* improvement (cheaper, or strictly
///   fewer inputs) — so genuine alternatives (all-equal pairs) are both kept.
///
/// Replacing `u` by `v` in any structure is feasible (same outputs, ⊆ inputs)
/// at no greater cost, so the **cost-optimum is preserved under any regime**,
/// including [`Composite`] with [`NoExcess`] (equal outputs ⇒ no new excess).
/// Domination is transitive and strict, so a single pass against the original
/// unit set is sound (every pruned unit is dominated by a surviving maximal one).
///
/// NOTE: this preserves the *optimum*, **not** the full set of solution-
/// structures — it is a search reduction for ABB, not for SSG enumeration of
/// *every* structure.
#[derive(Debug, Clone, Copy, Default)]
pub struct CostDominance;

/// True iff `v` strictly dominates `u` (see [`CostDominance`]).
fn dominates(p: &LoweredPGraph, v: DeclId, u: DeclId) -> bool {
    if p.outputs(v) != p.outputs(u) {
        return false;
    }
    if !p.inputs(v).is_subset(p.inputs(u)) {
        return false;
    }
    let cv = p.costs.get(&v).copied().unwrap_or(1.0);
    let cu = p.costs.get(&u).copied().unwrap_or(1.0);
    if cv > cu {
        return false;
    }
    // Strict improvement, so an all-equal pair dominates neither way.
    cv < cu || p.inputs(v).len() < p.inputs(u).len()
}

impl Regime for CostDominance {
    fn refine_maximal(&self, p: &LoweredPGraph, units: BTreeSet<DeclId>) -> BTreeSet<DeclId> {
        let all: Vec<DeclId> = units.iter().copied().collect();
        units
            .iter()
            .copied()
            .filter(|&u| !all.iter().any(|&v| v != u && dominates(p, v, u)))
            .collect()
    }

    fn name(&self) -> &'static str {
        "cost-dominance"
    }
}

/// Singleton instances (no allocation; `'static` for `&dyn Regime`).
pub static CANONICAL: Canonical = Canonical;
/// See [`CANONICAL`].
pub static NO_EXCESS: NoExcess = NoExcess;
/// See [`CANONICAL`].
pub static COST_DOMINANCE: CostDominance = CostDominance;

/// Adapter from the legacy `strict_no_excess: bool` to a [`Regime`].
///
/// `true → NoExcess`, `false → Canonical`. This keeps every existing
/// bool-keyed API behavior-identical while routing through the strategy.
pub fn from_strict_flag(strict_no_excess: bool) -> &'static dyn Regime {
    if strict_no_excess {
        &NO_EXCESS
    } else {
        &CANONICAL
    }
}

/// A **composite** regime: applies several refinements together (the
/// Composite pattern over [`Regime`]). Because the canonical
/// reduction + composition is always the base substrate, a `Composite`
/// expresses *"canonical + R1 + R2 + …"* — the way to *mix* the general
/// solver with one or more specific refinements in a single solve.
///
/// - [`Regime::refine_maximal`] threads the unit set through every
///   component and **iterates the whole stack to a combined fixpoint**, so
///   one component removing a unit can trigger another to remove more
///   (the components need not commute or be applied only once).
/// - [`Regime::structure_admissible`] requires **all** components to admit.
///
/// An empty component list behaves like [`Canonical`] (identity).
///
/// # Example
/// ```
/// use hymeko_pgraph::regime::{Composite, NoExcess, Regime};
/// # use hymeko_pgraph::regime::Canonical;
/// let no_excess = NoExcess;
/// let extra = Canonical; // stand-in for a second specific refinement
/// let mixed = Composite::new(vec![&no_excess as &dyn Regime, &extra]);
/// assert_eq!(mixed.name(), "composite");
/// ```
pub struct Composite<'a> {
    components: Vec<&'a dyn Regime>,
}

impl<'a> Composite<'a> {
    /// Compose refinements, applied left-to-right within each fixpoint pass.
    pub fn new(components: Vec<&'a dyn Regime>) -> Self {
        Self { components }
    }
}

impl Regime for Composite<'_> {
    fn refine_maximal(&self, p: &LoweredPGraph, mut units: BTreeSet<DeclId>) -> BTreeSet<DeclId> {
        loop {
            let before = units.len();
            for r in &self.components {
                units = r.refine_maximal(p, units);
            }
            if units.len() == before {
                break;
            }
        }
        units
    }

    fn structure_admissible(&self, p: &LoweredPGraph, sel: &BTreeSet<DeclId>) -> bool {
        self.components
            .iter()
            .all(|r| r.structure_admissible(p, sel))
    }

    fn name(&self) -> &'static str {
        "composite"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower;
    use parser::parse_description;

    fn lower_src(src: &str) -> LoweredPGraph {
        lower(&parse_description(src).expect("parse")).expect("lower")
    }

    #[test]
    fn canonical_is_identity() {
        let p = lower_src(
            r#"T{} context { P <material, product>; R <material, raw>; @u <unit> 1 { (-R, +P); } }"#,
        );
        let units: BTreeSet<DeclId> = p.units.clone();
        assert_eq!(Canonical.refine_maximal(&p, units.clone()), units);
        assert!(Canonical.structure_admissible(&p, &units));
        assert_eq!(Canonical.name(), "canonical");
    }

    #[test]
    fn no_excess_drops_byproduct_producer_and_rejects_excess() {
        // u produces P (product) and W (waste, consumed by nobody).
        let p = lower_src(
            r#"T{} context {
                P <material, product>; R <material, raw>; W <material>;
                @u <unit> 1 { (-R, +P, +W); }
            }"#,
        );
        let units: BTreeSet<DeclId> = p.units.clone();
        // refine_maximal drops u (W is excess).
        assert!(NoExcess.refine_maximal(&p, units.clone()).is_empty());
        // structure_admissible rejects {u} for the same reason.
        assert!(!NoExcess.structure_admissible(&p, &units));
        assert!(
            Canonical.structure_admissible(&p, &units),
            "canonical admits excess"
        );
        assert_eq!(NoExcess.name(), "no-excess");
    }

    #[test]
    fn from_strict_flag_maps_bool() {
        assert_eq!(from_strict_flag(true).name(), "no-excess");
        assert_eq!(from_strict_flag(false).name(), "canonical");
    }

    /// Test-only refinement: drop one unit by name (to exercise stacking).
    struct DropNamed(&'static str);
    impl Regime for DropNamed {
        fn refine_maximal(&self, p: &LoweredPGraph, units: BTreeSet<DeclId>) -> BTreeSet<DeclId> {
            let target = p.name_to_decl.get(self.0).copied();
            units.into_iter().filter(|u| Some(*u) != target).collect()
        }
        fn name(&self) -> &'static str {
            "drop-named"
        }
    }

    fn names(p: &LoweredPGraph, s: &BTreeSet<DeclId>) -> BTreeSet<String> {
        s.iter().map(|d| p.decl_to_name[d].clone()).collect()
    }

    #[test]
    fn composite_stacks_refinements() {
        // Three units all produce the product P; canonical keeps all three.
        let p = lower_src(
            r#"T{} context {
                P <material, product>; R <material, raw>;
                @a <unit> 1 { (-R, +P); }
                @b <unit> 1 { (-R, +P); }
                @c <unit> 1 { (-R, +P); }
            }"#,
        );
        let units = p.units.clone();
        let drop_a = DropNamed("a");
        let drop_b = DropNamed("b");
        let comp = Composite::new(vec![&drop_a as &dyn Regime, &drop_b]);
        // Both a and b removed (the union of the two refinements); c remains.
        assert_eq!(
            names(&p, &comp.refine_maximal(&p, units)),
            BTreeSet::from(["c".to_string()])
        );
    }

    #[test]
    fn cost_dominance_prunes_dearer_interchangeable_unit() {
        // `cheap` and `dear` produce the same product P from the same raw R;
        // `dear` costs more → dominated → pruned. A genuine alternative
        // (`alt`, different output) is kept.
        let p = lower_src(
            r#"T{} context {
                P <material, product>; Q <material, product>; R <material, raw>;
                @cheap <unit> 1 { (-R, +P); }
                @dear  <unit> 5 { (-R, +P); }
                @alt   <unit> 1 { (-R, +Q); }
            }"#,
        );
        let kept = CostDominance.refine_maximal(&p, p.units.clone());
        assert_eq!(
            names(&p, &kept),
            BTreeSet::from(["cheap".to_string(), "alt".to_string()]),
            "the dearer interchangeable producer is pruned; alternatives kept"
        );
    }

    #[test]
    fn cost_dominance_keeps_equal_alternatives() {
        // Identical I/O *and* cost → neither strictly dominates → both kept.
        let p = lower_src(
            r#"T{} context {
                P <material, product>; R <material, raw>;
                @x <unit> 2 { (-R, +P); }
                @y <unit> 2 { (-R, +P); }
            }"#,
        );
        assert_eq!(CostDominance.refine_maximal(&p, p.units.clone()).len(), 2);
    }

    #[test]
    fn composite_cost_dominance_and_no_excess_mix() {
        // Headline: mix two specific refinements on the canonical base.
        // `dear` (dearer twin of `cheap`) is cost-dominated; `waster`
        // produces an unconsumed by-product W (no-excess violation).
        // Composite([CostDominance, NoExcess]) must prune BOTH.
        let p = lower_src(
            r#"T{} context {
                P <material, product>; R <material, raw>; W <material>;
                @cheap  <unit> 1 { (-R, +P); }
                @dear   <unit> 5 { (-R, +P); }
                @waster <unit> 1 { (-R, +P, +W); }
            }"#,
        );
        let mixed = Composite::new(vec![&CostDominance as &dyn Regime, &NoExcess]);
        let kept = mixed.refine_maximal(&p, p.units.clone());
        assert_eq!(
            names(&p, &kept),
            BTreeSet::from(["cheap".to_string()]),
            "composite prunes the dominated unit AND the excess-bearing unit"
        );
        // Each refinement alone prunes only its own target.
        assert_eq!(CostDominance.refine_maximal(&p, p.units.clone()).len(), 2); // drops `dear`
        assert_eq!(NoExcess.refine_maximal(&p, p.units.clone()).len(), 2); // drops `waster`
    }

    #[test]
    fn composite_empty_is_identity_and_admissible_is_conjunction() {
        // Unit produces P (product) and W (waste consumed by nobody).
        let p = lower_src(
            r#"T{} context {
                P <material, product>; R <material, raw>; W <material>;
                @u <unit> 1 { (-R, +P, +W); }
            }"#,
        );
        let units = p.units.clone();
        // Empty composite = identity, admits all (≡ Canonical).
        let empty = Composite::new(vec![]);
        assert_eq!(empty.refine_maximal(&p, units.clone()), units);
        assert!(empty.structure_admissible(&p, &units));
        // Composite containing NoExcess inherits its rejection (AND of all).
        let ne = NoExcess;
        let comp = Composite::new(vec![&ne as &dyn Regime]);
        assert!(!comp.structure_admissible(&p, &units));
        assert!(comp.refine_maximal(&p, units).is_empty());
    }
}
