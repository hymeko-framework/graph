//! Bounded Berge **path** and **cycle** enumeration on the canonical `Ir`.
//!
//! Combinatorial enumeration is explicitly bounded: every run carries a [`EnumBudget`] with a
//! `max_depth` (hyperedges), a `max_results`, and a `max_expansions` (arc-expansion budget), and
//! stops deterministically. There is no implicit unbounded all-path enumeration. When a budget
//! stops the run, [`EnumOutcome::bound`] names which one -- truncation is reported, never silent.
//!
//! A Berge path here is *simple*: distinct nodes and distinct hyperedges. A Berge cycle returns to
//! its start over `>= 2` distinct hyperedges with distinct intermediate nodes; cycles are
//! canonicalised (rotation + reflection) and de-duplicated.

use std::collections::HashSet;

use hymeko::common::ids::{DeclId, HyperArcId, NodeId};
use hymeko::ir::incidence::IncidenceIndex;

use super::traversal::{BergePath, BergeStep};
use super::{BudgetKind, DirectIrError, decl_to_node, is_node, node_to_decl};

/// Hard bounds on a combinatorial enumeration. All three are required; pick finite values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumBudget {
    /// Maximum path/cycle length in hyperedges.
    pub max_depth: usize,
    /// Maximum number of results to emit.
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

/// The result of a bounded enumeration: the emitted items, and which budget (if any) stopped it.
#[derive(Debug, Clone)]
pub struct EnumOutcome<T> {
    /// The enumerated paths or cycles.
    pub items: Vec<T>,
    /// The budget that stopped enumeration, or `None` if it finished naturally.
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
    seen_cycles: HashSet<Vec<usize>>,
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
            bound: None,
            paths: Vec::new(),
            cycles: Vec::new(),
            seen_cycles: HashSet::new(),
        }
    }

    #[inline]
    fn on_path_set(&mut self, d: DeclId, v: bool) {
        self.on_path[d.raw()] = v;
    }

    /// True if a budget has stopped the run.
    #[inline]
    fn stopped(&self) -> bool {
        self.bound.is_some()
    }

    /// Charge one arc expansion; sets the expansion bound and returns false if over budget.
    #[inline]
    fn charge_expansion(&mut self) -> bool {
        if self.expansions >= self.budget.max_expansions {
            self.bound = Some(BudgetKind::Expansions);
            return false;
        }
        self.expansions += 1;
        true
    }

    fn dfs_paths(&mut self, cur: DeclId, goal: Option<DeclId>) -> Result<(), DirectIrError> {
        if self.steps.len() >= self.budget.max_depth || self.stopped() {
            return Ok(());
        }
        let ir = self.index.ir();
        for &arc in self.index.incident_arcs(cur) {
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
                    slot: slot as u32,
                    sign,
                    node: node_id,
                });
                self.on_path[target.raw()] = true;

                let emit = match goal {
                    Some(g) => target == g,
                    None => true,
                };
                if emit {
                    self.paths.push(BergePath {
                        start: self.start,
                        steps: self.steps.clone(),
                    });
                    if self.paths.len() >= self.budget.max_results {
                        self.bound = Some(BudgetKind::Results);
                    }
                }
                if !self.stopped() {
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
        if self.steps.len() >= self.budget.max_depth || self.stopped() {
            return Ok(());
        }
        let ir = self.index.ir();
        for &arc in self.index.incident_arcs(cur) {
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
                    slot: slot as u32,
                    sign,
                    node: node_id,
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
            nodes.push(st.node);
        }
        let mut arcs: Vec<HyperArcId> = self.steps.iter().map(|s| s.arc).collect();
        arcs.push(closing_arc);

        let key = canonical_cycle_key(&nodes);
        if self.seen_cycles.insert(key) {
            self.cycles.push(BergeCycle { nodes, arcs });
        }
    }
}

/// Canonical key for a cycle's node ring: rotate so the smallest id is first, then take the
/// lexicographically smaller of the forward and reversed rotations. Rotation + reflection invariant.
fn canonical_cycle_key(nodes: &[NodeId]) -> Vec<usize> {
    let raw: Vec<usize> = nodes.iter().map(|n| n.raw()).collect();
    let n = raw.len();
    if n == 0 {
        return raw;
    }
    let min_pos = (0..n).min_by_key(|&i| raw[i]).unwrap_or(0);
    let forward: Vec<usize> = (0..n).map(|k| raw[(min_pos + k) % n]).collect();
    // reversed ring starting at the same min element
    let mut backward: Vec<usize> = (0..n).map(|k| raw[(min_pos + n - k) % n]).collect();
    backward.rotate_left(0);
    if backward < forward {
        backward
    } else {
        forward
    }
}
