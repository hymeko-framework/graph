//! Reachability rules over the P-graph producibility closure.
//!
//! The audit-side analogue lives at
//! `hymeko_neuro/baselines/reachability.py`; the unifying argument is in
//! `docs/plans/2026-06-14-reachability-rules-audit-pgraph/argument.md`. A
//! *reachability rule* decides which units seed the producibility closure
//! [`crate::msg::close_producible`] — the same closure the ABB reachability
//! bound and SSG feasibility already evaluate. The three rules form a monotone
//! lattice `Strict ⊆ TransductiveTopology ⊆ TransductiveFull`:
//!
//! - **Strict** — only *confirmed* units seed the closure (the canonical PNS
//!   behaviour, `R₀`): candidates are unreachable.
//! - **TransductiveTopology** — candidate units' *connectivity* seeds the
//!   closure (their outputs become producible), but their costs are withheld —
//!   an optimistic, still-admissible bound for ABB.
//! - **TransductiveFull** — candidate units are fully available, costs included.
//!
//! Topology and Full produce the *same* producibility closure (both admit
//! candidate outputs); they diverge only at the cost/admissibility layer, via
//! [`ReachabilityRule::counts_candidate_cost`]. That split mirrors the audit's
//! topology-vs-full distinction (reachable connection vs reachable label).

use std::collections::BTreeSet;

use hymeko::common::ids::DeclId;

use crate::lowering::LoweredPGraph;
use crate::msg::close_producible;

/// Which units seed the producibility closure at synthesis time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReachabilityRule {
    /// Confirmed units only — canonical PNS (`R₀`).
    Strict,
    /// Confirmed ∪ candidate connectivity; candidate costs withheld.
    TransductiveTopology,
    /// Confirmed ∪ candidate units, costs included.
    TransductiveFull,
}

impl ReachabilityRule {
    /// Whether candidate (unconfirmed) units seed the producibility closure.
    pub fn admits_candidates(self) -> bool {
        !matches!(self, ReachabilityRule::Strict)
    }

    /// Whether candidate unit *costs* are counted (`Full`) vs withheld for an
    /// optimistic bound (`Strict` / `Topology`).
    pub fn counts_candidate_cost(self) -> bool {
        matches!(self, ReachabilityRule::TransductiveFull)
    }

    /// Stable identifier (echoed in dumps/reports), matching the audit tokens.
    pub fn name(self) -> &'static str {
        match self {
            ReachabilityRule::Strict => "strict",
            ReachabilityRule::TransductiveTopology => "topo",
            ReachabilityRule::TransductiveFull => "full",
        }
    }
}

/// Producibility closure under `rule`.
///
/// `units` are the confirmed (established) operating units; `candidates` the
/// unconfirmed ones admitted by the transductive rules.
///
/// # Preconditions
/// - `units` and `candidates` are O-nodes of `p`.
///
/// # Postconditions
/// - `Strict` ⇒ result `== close_producible(p, units, p.raws)` (reduction to
///   canonical PNS — candidates ignored).
/// - `TransductiveTopology` / `TransductiveFull` ⇒
///   `== close_producible(p, units ∪ candidates, p.raws)`.
/// - Monotone: the `Strict` closure is a subset of the transductive closure.
pub fn close_producible_under_rule(
    p: &LoweredPGraph,
    units: &BTreeSet<DeclId>,
    candidates: &BTreeSet<DeclId>,
    rule: ReachabilityRule,
) -> BTreeSet<DeclId> {
    if rule.admits_candidates() {
        let all: BTreeSet<DeclId> = units.union(candidates).copied().collect();
        close_producible(p, &all, &p.raws)
    } else {
        close_producible(p, units, &p.raws)
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

    /// Fixture: R --u_known--> M --u_cand--> P. The product P is producible
    /// ONLY via the candidate unit `u_cand`.
    fn fixture() -> (LoweredPGraph, DeclId, DeclId) {
        let p = lower_src(
            r#"T{} context {
                R <material, raw>; M <material>; P <material, product>;
                @u_known <unit> 1 { (-R, +M); }
                @u_cand  <unit> 1 { (-M, +P); }
            }"#,
        );
        let known = p.name_to_decl["u_known"];
        let cand = p.name_to_decl["u_cand"];
        (p, known, cand)
    }

    /// Reduction (soundness 1): Strict reproduces the canonical closure exactly,
    /// candidates ignored. This is `R_strict = R₀`.
    #[test]
    fn strict_reduces_to_canonical_closure() {
        let (p, known, cand) = fixture();
        let units = BTreeSet::from([known]);
        let candidates = BTreeSet::from([cand]);
        let got = close_producible_under_rule(&p, &units, &candidates, ReachabilityRule::Strict);
        let canonical = close_producible(&p, &units, &p.raws);
        assert_eq!(got, canonical);
    }

    /// The candidate unit unlocks the product: Strict cannot produce P, the
    /// transductive rules can — the reachability gain made concrete.
    #[test]
    fn candidate_unlocks_product() {
        let (p, known, cand) = fixture();
        let prod = p.name_to_decl["P"];
        let units = BTreeSet::from([known]);
        let candidates = BTreeSet::from([cand]);

        let strict = close_producible_under_rule(&p, &units, &candidates, ReachabilityRule::Strict);
        assert!(!strict.contains(&prod), "Strict must not reach the product");

        for rule in [
            ReachabilityRule::TransductiveTopology,
            ReachabilityRule::TransductiveFull,
        ] {
            let t = close_producible_under_rule(&p, &units, &candidates, rule);
            assert!(t.contains(&prod), "{} must reach the product", rule.name());
        }
    }

    /// Monotone lattice (soundness 2 backbone): producible(Strict) ⊆
    /// producible(Topology) == producible(Full).
    #[test]
    fn closure_is_monotone_in_rule() {
        let (p, known, cand) = fixture();
        let units = BTreeSet::from([known]);
        let candidates = BTreeSet::from([cand]);
        let s = close_producible_under_rule(&p, &units, &candidates, ReachabilityRule::Strict);
        let topo = close_producible_under_rule(
            &p,
            &units,
            &candidates,
            ReachabilityRule::TransductiveTopology,
        );
        let full = close_producible_under_rule(
            &p,
            &units,
            &candidates,
            ReachabilityRule::TransductiveFull,
        );
        assert!(s.is_subset(&topo), "Strict ⊆ Topology");
        assert_eq!(
            topo, full,
            "Topology and Full share the producibility closure"
        );
    }

    /// The cost/admissibility split: only Full counts candidate costs; only
    /// Strict refuses candidates. (The topology-vs-full discriminator.)
    #[test]
    fn rule_flags_truth_table() {
        assert!(!ReachabilityRule::Strict.admits_candidates());
        assert!(ReachabilityRule::TransductiveTopology.admits_candidates());
        assert!(ReachabilityRule::TransductiveFull.admits_candidates());

        assert!(!ReachabilityRule::Strict.counts_candidate_cost());
        assert!(!ReachabilityRule::TransductiveTopology.counts_candidate_cost());
        assert!(ReachabilityRule::TransductiveFull.counts_candidate_cost());
    }
}
