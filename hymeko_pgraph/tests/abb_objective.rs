//! Objective-driven ABB (`solve_with_objective`): additive objectives agree with
//! `solve`; a non-additive peak objective and objective-side rejection agree with
//! brute force over the decision-mapping solution structures. The classic ABB
//! reports an unproven incumbent.

use std::collections::{BTreeMap, BTreeSet};

use hymeko::common::ids::DeclId;
use hymeko_pgraph::lowering::LoweredPGraph;
use hymeko_pgraph::msg::maximal_structure_with_regime;
use hymeko_pgraph::regime::{Canonical, Regime};
use hymeko_pgraph::{
    Abb, AbbOptions, AdditiveCost, StructureObjective, abb_solve, lower, solve_with_objective,
    ssg_dm_enumerate,
};
use parser::parse_description;

const SYNTH: &str = r#"
T{}
context {
    P   <material, product>;
    R   <material, raw>;
    Mid <material>;
    W   <material>;
    @a <unit> 1 { (-R, +P); }
    @b <unit> 5 { (-R, +P); }
    @c <unit> 2 { (-Mid, +P); }
    @d <unit> 1 { (-R, +Mid); }
    @e <unit> 1 { (-R, +P, +W); }
}
"#;

fn graph() -> LoweredPGraph {
    lower(&parse_description(SYNTH).unwrap()).unwrap()
}

fn ids(p: &LoweredPGraph) -> BTreeMap<String, DeclId> {
    p.decl_to_name.iter().map(|(d, n)| (n.clone(), *d)).collect()
}

fn cost(p: &LoweredPGraph, units: &BTreeSet<DeclId>) -> f64 {
    units.iter().map(|u| p.costs.get(u).copied().unwrap_or(1.0)).sum()
}

/// (max unit width)^2 * total cost, the shape of a Q^2*T objective; monotone in the
/// selection, so the included part is an admissible bound. `reject` vetoes structures.
struct Peak<'a> {
    p: &'a LoweredPGraph,
    width: BTreeMap<DeclId, f64>,
    reject: Option<DeclId>,
}

impl Peak<'_> {
    fn value(&self, units: &BTreeSet<DeclId>) -> f64 {
        let peak = units.iter().map(|u| self.width[u]).fold(0.0, f64::max);
        peak * peak * cost(self.p, units)
    }
}

impl StructureObjective for Peak<'_> {
    fn lower_bound(&self, included: &BTreeSet<DeclId>, _open: &BTreeSet<DeclId>) -> f64 {
        self.value(included)
    }
    fn evaluate(&self, units: &BTreeSet<DeclId>) -> Option<f64> {
        (!self.reject.is_some_and(|r| units.contains(&r))).then(|| self.value(units))
    }
}

fn brute_force(p: &LoweredPGraph, objective: &dyn StructureObjective) -> (f64, BTreeSet<DeclId>) {
    let msg = maximal_structure_with_regime(p, &Canonical);
    ssg_dm_enumerate(p, &msg)
        .into_iter()
        .filter(|s| Canonical.structure_admissible(p, &s.units))
        .filter_map(|s| objective.evaluate(&s.units).map(|v| (v, s.units)))
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
        .unwrap()
}

#[test]
fn additive_objective_matches_classic_abb() {
    let p = graph();
    let msg = maximal_structure_with_regime(&p, &Canonical);
    let classic = abb_solve(&p, &msg).unwrap();
    let general = solve_with_objective(&p, &msg, &AbbOptions::default(), &Canonical, &AdditiveCost::new(&p, None)).unwrap();
    assert!(general.complete);
    assert_eq!(general.value, classic.cost);
    assert_eq!(cost(&p, &general.units), classic.cost);
}

#[test]
fn peak_objective_and_rejection_match_brute_force() {
    let p = graph();
    let id = ids(&p);
    let width: BTreeMap<_, _> = [("a", 10.0), ("b", 10.0), ("c", 2.0), ("d", 2.0), ("e", 10.0)]
        .into_iter()
        .map(|(n, w)| (id[n], w))
        .collect();
    let msg = maximal_structure_with_regime(&p, &Canonical);
    for reject in [None, Some(id["c"])] {
        let objective = Peak { p: &p, width: width.clone(), reject };
        let found = solve_with_objective(&p, &msg, &AbbOptions::default(), &Canonical, &objective).unwrap();
        let (value, units) = brute_force(&p, &objective);
        assert!(found.complete);
        assert_eq!(found.value, value);
        assert_eq!(found.units, units);
    }
    // The peak objective prefers the narrow two-unit route over the cheapest unit.
    let objective = Peak { p: &p, width, reject: None };
    let found = solve_with_objective(&p, &msg, &AbbOptions::default(), &Canonical, &objective).unwrap();
    assert_eq!(found.units, [id["c"], id["d"]].into_iter().collect());
}

#[test]
fn exploration_limit_reports_an_unproven_incumbent() {
    let p = graph();
    let msg = maximal_structure_with_regime(&p, &Canonical);
    let opts = AbbOptions { max_explored: 7, ..AbbOptions::default() };
    let found = solve_with_objective(&p, &msg, &opts, &Canonical, &AdditiveCost::new(&p, None));
    assert!(found.is_none_or(|s| !s.complete));
}

/// Rejects every structure containing `banned`, with an infinite bound as soon as it is included.
struct Banned<'a> {
    p: &'a LoweredPGraph,
    banned: DeclId,
}

impl StructureObjective for Banned<'_> {
    fn lower_bound(&self, included: &BTreeSet<DeclId>, _open: &BTreeSet<DeclId>) -> f64 {
        if included.contains(&self.banned) { f64::INFINITY } else { cost(self.p, included) }
    }
    fn evaluate(&self, units: &BTreeSet<DeclId>) -> Option<f64> {
        (!units.contains(&self.banned)).then(|| cost(self.p, units))
    }
}

#[test]
fn an_infinite_bound_prunes_before_any_incumbent_exists() {
    let p = graph();
    let msg = maximal_structure_with_regime(&p, &Canonical);
    // Include-first order enters the first unit's subtree first; it must be cut at once.
    let banned = *msg.units.iter().next().unwrap();
    let opts = AbbOptions { max_explored: 2 * msg.units.len() as u64 + 2, ..AbbOptions::default() };
    let found = solve_with_objective(&p, &msg, &opts, &Canonical, &Banned { p: &p, banned }).unwrap();
    assert!(!found.units.contains(&banned));
    assert!(found.pruned_by_bound > 0);
}

#[test]
fn classic_abb_reports_whether_its_search_completed() {
    let p = graph();
    let msg = maximal_structure_with_regime(&p, &Canonical);
    let full = Abb::new(&Canonical, 0).solve(&p, &msg, None).unwrap();
    assert!(full.complete);
    assert_eq!(full.cost, brute_force(&p, &AdditiveCost::new(&p, None)).0);
    let cut = Abb::new(&Canonical, 7).solve(&p, &msg, None);
    assert!(cut.is_none_or(|s| !s.complete));
}
