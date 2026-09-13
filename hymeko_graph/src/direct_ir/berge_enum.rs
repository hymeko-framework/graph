//! Bounded Berge **path** and **cycle** enumeration on the canonical `Ir`.
//!
//! Combinatorial enumeration is explicitly bounded: every run carries a [`EnumBudget`] with a
//! `max_depth` (hyperedges), a `max_results`, and a `max_expansions` (arc-expansion budget), and
//! stops deterministically. There is no implicit unbounded all-path enumeration. When a budget
//! prunes a branch or stops the run, [`EnumOutcome::bound`] reports it.
//!
//! A Berge path here is *simple*: distinct nodes and distinct hyperedges. A Berge cycle returns to
//! its start over `>= 2` distinct hyperedges with distinct intermediate nodes; cycles are
//! canonicalised (rotation + reflection) and de-duplicated.

use std::collections::HashSet;

use hymeko::common::ids::{DeclId, HyperArcId, NodeId};
use hymeko::ir::incidence::IncidenceIndex;

use super::traversal::{BergeIncidence, BergePath, BergeStep};
use super::{BudgetKind, DirectIrError, decl_to_node, is_node, node_to_decl};

/// Hard bounds on a combinatorial enumeration. All three are required; pick finite values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumBudget {
    /// Maximum path/cycle length in hyperedges. Longer branches are pruned locally.
    pub max_depth: usize,
    /// Maximum number of results to emit. Zero stops immediately after endpoint validation.
    pub max_results: usize,
    /// Maximum number of arc expansions before a deterministic stop.
    pub max_expansions: usize,
}

impl EnumBudget {
    /// A budget with the given depth and result caps and a generous expansion cap.
    #[must_use]
    pub fn new(max_depth: usize, max_results: usize, max_expansions: usize) -> Self {
        Self {
            max_depth,
            max_results,
            max_expansions,
        }
    }
}

/// The result of a bounded enumeration: emitted items and any encountered budget limit.
#[derive(Debug, Clone)]
pub struct EnumOutcome<T> {
    /// The enumerated paths or cycles.
    pub items: Vec<T>,
    /// `Depth` means an unused incident arc was left unopened at the depth cap; it does not
    /// prove that another result exists. Other branches continue. `Results` or `Expansions`
    /// stop the whole run and take precedence over an earlier `Depth`. Depth is checked before
    /// charging an expansion. `Results` is reported upon reaching the cap, including zero.
    /// `None` means enumeration finished without encountering a limit.
    pub bound: Option<BudgetKind>,
}

/// A Berge cycle: nodes in cyclic order (each consecutive pair, and last->first, joined by an arc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BergeCycle {
    /// The cycle's nodes in order (the start is not repeated at the end).
    pub nodes: Vec<NodeId>,
    /// The hyperarcs joining consecutive nodes (`arcs[i]` joins `nodes[i]`..`nodes[i+1]`, and
    /// `arcs[last]` joins `nodes[last]`..`nodes[0]`).
    pub arcs: Vec<HyperArcId>,
}

/// Enumerate simple Berge paths from `start`. With `goal = Some(g)`, only paths ending at `g` are
/// emitted; with `goal = None`, every simple path (each prefix that extends the frontier) is emitted.
///
/// # Errors
/// [`DirectIrError::InvalidStart`] for a bad endpoint; [`DirectIrError::TargetNotANode`] on a
/// malformed arc reference.
pub fn enumerate_berge_paths(
    index: &IncidenceIndex<'_>,
    start: NodeId,
    goal: Option<NodeId>,
    budget: EnumBudget,
) -> Result<EnumOutcome<BergePath>, DirectIrError> {
    let ir = index.ir();
    let start_decl = node_to_decl(ir, start)?;
    let goal_decl = goal.map(|g| node_to_decl(ir, g)).transpose()?;

    let mut en = Enumerator::new(index, budget, start);
    en.on_path_set(start_decl, true);
    en.dfs_paths(start_decl, goal_decl)?;
    Ok(EnumOutcome {
        items: en.paths,
        bound: en.bound,
    })
}

/// Enumerate simple Berge cycles through `start`, canonicalised and de-duplicated.
///
/// # Errors
/// [`DirectIrError::InvalidStart`] for a bad start; [`DirectIrError::TargetNotANode`] on a
/// malformed arc reference.
pub fn enumerate_berge_cycles(
    index: &IncidenceIndex<'_>,
    start: NodeId,
    budget: EnumBudget,
) -> Result<EnumOutcome<BergeCycle>, DirectIrError> {
    let ir = index.ir();
    let start_decl = node_to_decl(ir, start)?;
    let mut en = Enumerator::new(index, budget, start);
    en.on_path_set(start_decl, true);
    en.dfs_cycles(start_decl, start_decl)?;
    Ok(EnumOutcome {
        items: en.cycles,
        bound: en.bound,
    })
}

/// Backtracking scratch for one enumeration run.
struct Enumerator<'a, 'ir> {
    index: &'a IncidenceIndex<'ir>,
    budget: EnumBudget,
    start: NodeId,
    on_path: Vec<bool>,  // by DeclId
    used_arc: Vec<bool>, // by HyperArcId
    steps: Vec<BergeStep>,
    expansions: usize,
    bound: Option<BudgetKind>,
    paths: Vec<BergePath>,
    cycles: Vec<BergeCycle>,
    seen_cycles: HashSet<Vec<(usize, usize)>>,
}

impl<'a, 'ir> Enumerator<'a, 'ir> {
    fn new(index: &'a IncidenceIndex<'ir>, budget: EnumBudget, start: NodeId) -> Self {
        let ir = index.ir();
        Self {
            index,
            budget,
            start,
            on_path: vec![false; ir.decl_nodes.len()],
            used_arc: vec![false; ir.arcs.len()],
            steps: Vec::new(),
            expansions: 0,
            bound: (budget.max_results == 0).then_some(BudgetKind::Results),
            paths: Vec::new(),
            cycles: Vec::new(),
            seen_cycles: HashSet::new(),
        }
    }

    #[inline]
    fn on_path_set(&mut self, d: DeclId, v: bool) {
        self.on_path[d.raw()] = v;
    }

    /// True if a budget has stopped the whole run rather than pruned a branch.
    #[inline]
    fn stopped(&self) -> bool {
        matches!(
            self.bound,
            Some(BudgetKind::Results | BudgetKind::Expansions)
        )
    }

    /// Check an unused frontier arc against depth, then charge an allowed expansion.
    #[inline]
    fn charge_expansion(&mut self) -> bool {
        if self.steps.len() >= self.budget.max_depth {
            self.bound = Some(BudgetKind::Depth);
            return false;
        }
        if self.expansions >= self.budget.max_expansions {
            self.bound = Some(BudgetKind::Expansions);
            return false;
        }
        self.expansions += 1;
        true
    }

    fn dfs_paths(&mut self, cur: DeclId, goal: Option<DeclId>) -> Result<(), DirectIrError> {
        if self.stopped() {
            return Ok(());
        }
        let ir = self.index.ir();
        // The entry coordinate comes from the index's own per-incidence arrays, which run parallel
        // to `incident_arcs`. It is recorded while it is known: `cur` may occupy several slots of
        // one arc, so recovering it afterwards by scanning `refs` has no unique answer.
        for (&arc, &entry_slot) in self
            .index
            .incident_arcs(cur)
            .iter()
            .zip(self.index.incident_slots(cur))
        {
            if self.used_arc[arc.raw()] {
                continue;
            }
            if !self.charge_expansion() {
                return Ok(());
            }
            self.used_arc[arc.raw()] = true;
            let n_refs = ir.arcs[arc.raw()].refs.len();
            for slot in 0..n_refs {
                let sref = &ir.arcs[arc.raw()].refs[slot];
                let target = sref.target();
                if !is_node(ir, target) {
                    self.used_arc[arc.raw()] = false;
                    return Err(DirectIrError::TargetNotANode {
                        arc,
                        slot: slot as u32,
                        target,
                    });
                }
                if self.on_path[target.raw()] {
                    continue;
                }
                let sign = sref.sign();
                let node_id = match decl_to_node(ir, target) {
                    Some(n) => n,
                    None => continue,
                };
                self.steps.push(BergeStep {
                    arc,
                    from: match BergeIncidence::at(ir, arc, entry_slot) {
                        Some(i) => i,
                        None => {
                            self.used_arc[arc.raw()] = false;
                            return Err(DirectIrError::TargetNotANode {
                                arc,
                                slot: entry_slot,
                                target: ir.arcs[arc.raw()].refs[entry_slot as usize].target(),
                            });
                        }
                    },
                    to: BergeIncidence {
                        node: node_id,
                        slot: slot as u32,
                        sign,
                    },
                });
                self.on_path[target.raw()] = true;

                let reached_goal = goal == Some(target);
                let emit = reached_goal || goal.is_none();
                if emit {
                    self.paths.push(BergePath {
                        start: self.start,
                        steps: self.steps.clone(),
                    });
                    if self.paths.len() >= self.budget.max_results {
                        self.bound = Some(BudgetKind::Results);
                    }
                }
                if !reached_goal && !self.stopped() {
                    self.dfs_paths(target, goal)?;
                }

                self.on_path[target.raw()] = false;
                self.steps.pop();
                if self.stopped() {
                    break;
                }
            }
            self.used_arc[arc.raw()] = false;
            if self.stopped() {
                break;
            }
        }
        Ok(())
    }

    fn dfs_cycles(&mut self, cur: DeclId, start_decl: DeclId) -> Result<(), DirectIrError> {
        if self.stopped() {
            return Ok(());
        }
        let ir = self.index.ir();
        // The entry coordinate comes from the index's own per-incidence arrays, which run parallel
        // to `incident_arcs`. It is recorded while it is known: `cur` may occupy several slots of
        // one arc, so recovering it afterwards by scanning `refs` has no unique answer.
        for (&arc, &entry_slot) in self
            .index
            .incident_arcs(cur)
            .iter()
            .zip(self.index.incident_slots(cur))
        {
            if self.used_arc[arc.raw()] {
                continue;
            }
            if !self.charge_expansion() {
                return Ok(());
            }
            self.used_arc[arc.raw()] = true;
            let n_refs = ir.arcs[arc.raw()].refs.len();
            for slot in 0..n_refs {
                let sref = &ir.arcs[arc.raw()].refs[slot];
                let target = sref.target();
                if !is_node(ir, target) {
                    self.used_arc[arc.raw()] = false;
                    return Err(DirectIrError::TargetNotANode {
                        arc,
                        slot: slot as u32,
                        target,
                    });
                }
                // Closing the cycle: back to start over >= 2 distinct hyperedges.
                if target == start_decl {
                    if self.steps.len() + 1 >= 2 {
                        self.record_cycle(arc);
                        if self.cycles.len() >= self.budget.max_results {
                            self.bound = Some(BudgetKind::Results);
                        }
                    }
                    if self.stopped() {
                        break;
                    }
                    continue;
                }
                if self.on_path[target.raw()] {
                    continue;
                }
                let sign = sref.sign();
                let node_id = match decl_to_node(ir, target) {
                    Some(n) => n,
                    None => continue,
                };
                self.steps.push(BergeStep {
                    arc,
                    from: match BergeIncidence::at(ir, arc, entry_slot) {
                        Some(i) => i,
                        None => {
                            self.used_arc[arc.raw()] = false;
                            return Err(DirectIrError::TargetNotANode {
                                arc,
                                slot: entry_slot,
                                target: ir.arcs[arc.raw()].refs[entry_slot as usize].target(),
                            });
                        }
                    },
                    to: BergeIncidence {
                        node: node_id,
                        slot: slot as u32,
                        sign,
                    },
                });
                self.on_path[target.raw()] = true;
                self.dfs_cycles(target, start_decl)?;
                self.on_path[target.raw()] = false;
                self.steps.pop();
                if self.stopped() {
                    break;
                }
            }
            self.used_arc[arc.raw()] = false;
            if self.stopped() {
                break;
            }
        }
        Ok(())
    }

    /// Record a canonicalised cycle formed by the current step stack closed by `closing_arc`.
    fn record_cycle(&mut self, closing_arc: HyperArcId) {
        let mut nodes: Vec<NodeId> = Vec::with_capacity(self.steps.len() + 1);
        nodes.push(self.start);
        for st in &self.steps {
            nodes.push(st.to.node);
        }
        let mut arcs: Vec<HyperArcId> = self.steps.iter().map(|s| s.arc).collect();
        arcs.push(closing_arc);

        let key = canonical_cycle_key(&nodes, &arcs);
        if self.seen_cycles.insert(key) {
            self.cycles.push(BergeCycle { nodes, arcs });
        }
    }
}

/// Canonical key for the alternating node/arc ring. Every rotation and both traversal directions
/// compare lexicographically, preserving the identity of parallel hyperarcs.
fn canonical_cycle_key(nodes: &[NodeId], arcs: &[HyperArcId]) -> Vec<(usize, usize)> {
    debug_assert_eq!(nodes.len(), arcs.len());
    let n = nodes.len();
    if n == 0 {
        return Vec::new();
    }

    let mut best: Option<Vec<(usize, usize)>> = None;
    for start in 0..n {
        let forward: Vec<_> = (0..n)
            .map(|step| {
                let i = (start + step) % n;
                (nodes[i].raw(), arcs[i].raw())
            })
            .collect();
        let backward: Vec<_> = (0..n)
            .map(|step| {
                let node_i = (start + n - step) % n;
                let arc_i = (node_i + n - 1) % n;
                (nodes[node_i].raw(), arcs[arc_i].raw())
            })
            .collect();
        for candidate in [forward, backward] {
            if best
                .as_ref()
                .is_none_or(|current| candidate.as_slice() < current.as_slice())
            {
                best = Some(candidate);
            }
        }
    }
    best.unwrap_or_default()
}
