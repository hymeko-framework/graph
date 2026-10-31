//! Binary column matroid over canonical hyperarc incidences.
//!
//! Columns are selected `HyperArcId`s; rows are canonical `NodeId`s. Repeated
//! incidences add modulo two. Signs coincide in GF(2); explicit weights must be
//! binary scalars. No graph projection is built. Sparse elimination pivots are
//! caller-owned algebraic scratch, and output coordinates refer to original arcs.
use super::{MatroidError, MatroidRef};
use crate::direct_ir::decl_to_node;
use hymeko::common::ids::{HyperArcId, NodeId};
use hymeko::ir::ir::ValueR;
use std::collections::{BTreeMap, BTreeSet};

/// Coordinates of a column as an XOR of selected canonical basis columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryCoordinate {
    /// Original column identity.
    pub arc: HyperArcId,
    /// Canonical basis identities whose columns XOR to this column.
    pub support: Vec<HyperArcId>,
}

/// A basis and exact reconstruction of every supplied ground element.
#[derive(Clone, Debug, PartialEq)]
pub struct BinaryBasis {
    /// Independent canonical columns, in greedy admission order.
    pub elements: Vec<HyperArcId>,
    /// All column coordinates, in caller ground order.
    pub coordinates: Vec<BinaryCoordinate>,
    /// Sum of the supplied finite weights of selected columns.
    pub weight: f64,
}

type Pivot = (BTreeSet<NodeId>, BTreeSet<HyperArcId>);

/// Bounded sparse elimination scratch; never a second semantic graph.
#[derive(Debug)]
pub struct BinaryMatroidWorkspace {
    pivots: BTreeMap<NodeId, Pivot>,
    max_rank: usize,
    max_entries: usize,
    entries: usize,
}
impl Default for BinaryMatroidWorkspace {
    fn default() -> Self {
        Self::with_limits(4096, 1_048_576)
    }
}
impl BinaryMatroidWorkspace {
    /// Bound admitted pivots and total stored sparse coordinates. Zero admits
    /// only rank-zero inputs. Limits do not count the caller-owned Core model.
    pub fn with_limits(max_rank: usize, max_entries: usize) -> Self {
        Self {
            pivots: BTreeMap::new(),
            max_rank,
            max_entries,
            entries: 0,
        }
    }
    fn clear(&mut self) {
        self.pivots.clear();
        self.entries = 0;
    }
}

fn toggle<T: Ord + Copy>(set: &mut BTreeSet<T>, item: T) {
    if !set.remove(&item) {
        set.insert(item);
    }
}

/// Visit the nonzero coordinates of one canonical binary incidence column.
/// Duplicate references cancel; numeric weights other than 0 and 1 are errors.
pub fn binary_column(m: MatroidRef<'_, '_>, arc: HyperArcId) -> Result<Vec<NodeId>, MatroidError> {
    let ir = m.ir();
    let record = ir
        .arcs
        .get(arc.raw())
        .ok_or(MatroidError::UnknownArc { arc })?;
    let mut rows = BTreeSet::new();
    for (slot, reference) in record.refs.iter().enumerate() {
        let invalid = MatroidError::NonBinaryIncidence {
            arc,
            slot: slot as u32,
        };
        let atom = ir.reference_atom(reference).map_err(|_| invalid)?;
        let active = match atom.weights {
            None | Some([]) => true,
            Some([ValueR::Num(v)]) if *v == 0.0 || *v == 1.0 => *v == 1.0,
            _ => return Err(invalid),
        };
        let node = decl_to_node(ir, reference.target()).ok_or(MatroidError::DanglingTarget {
            arc,
            slot: slot as u32,
        })?;
        if active {
            toggle(&mut rows, node);
        }
    }
    Ok(rows.into_iter().collect())
}

/// Minimum-weight maximal independent subset and lossless XOR coordinates.
///
/// Every column is read directly from the borrowed Core snapshot. The greedy
/// theorem applies to the supplied additive column weights, not circuit cost.
/// Equal-weight ties use canonical arc IDs. A failed call may alter scratch but
/// cannot mutate Core; the next call resets scratch before use.
pub fn binary_weighted_basis(
    m: MatroidRef<'_, '_>,
    ground: &[(HyperArcId, f64)],
    ws: &mut BinaryMatroidWorkspace,
) -> Result<BinaryBasis, MatroidError> {
    ws.clear();
    let mut unique = BTreeSet::new();
    for &(arc, weight) in ground {
        if !unique.insert(arc) {
            return Err(MatroidError::DuplicateGroundElement { arc });
        }
        if !weight.is_finite() {
            return Err(MatroidError::NonFiniteWeight { arc });
        }
    }
    let mut order: Vec<_> = ground.iter().copied().enumerate().collect();
    order.sort_by(|a, b| a.1.1.total_cmp(&b.1.1).then_with(|| a.1.0.cmp(&b.1.0)));
    let mut coordinates: Vec<Option<BinaryCoordinate>> = vec![None; ground.len()];
    let mut result = BinaryBasis {
        elements: vec![],
        coordinates: vec![],
        weight: 0.0,
    };
    for (position, (arc, weight)) in order {
        let mut rows: BTreeSet<_> = binary_column(m, arc)?.into_iter().collect();
        let mut support = BTreeSet::new();
        while let Some(&pivot) = rows.first() {
            if let Some((other, combination)) = ws.pivots.get(&pivot) {
                for &row in other {
                    toggle(&mut rows, row);
                }
                for &element in combination {
                    toggle(&mut support, element);
                }
            } else {
                let mut provenance = support.clone();
                provenance.insert(arc);
                let size = rows
                    .len()
                    .checked_add(provenance.len())
                    .ok_or(MatroidError::Overflow)?;
                let entries = ws.entries.checked_add(size).ok_or(MatroidError::Overflow)?;
                if ws.pivots.len() == ws.max_rank || entries > ws.max_entries {
                    return Err(MatroidError::BinaryWorkspaceBudget);
                }
                ws.entries = entries;
                ws.pivots.insert(pivot, (rows, provenance));
                result.elements.push(arc);
                result.weight += weight;
                if !result.weight.is_finite() {
                    return Err(MatroidError::NonFiniteWeight { arc });
                }
                support.clear();
                support.insert(arc);
                break;
            }
        }
        coordinates[position] = Some(BinaryCoordinate {
            arc,
            support: support.into_iter().collect(),
        });
    }
    result.coordinates = coordinates.into_iter().map(Option::unwrap).collect();
    Ok(result)
}

/// Rank certificates at one operation boundary in a separate-bank linear model.
#[derive(Clone, Debug)]
pub struct BinaryFrontierStep {
    /// Position in the supplied paired column lists.
    pub term: usize,
    /// Still-needed input forms, including this operation's read.
    pub input_before: BinaryBasis,
    /// Emitted output span, including this operation's write.
    pub output_after: BinaryBasis,
    /// Input rank only after this operation may retire its last uses.
    pub input_rank_after: usize,
}

/// A bounded ordering search with exact witnesses, not a quantum circuit.
#[derive(Clone, Debug)]
pub struct BinaryFrontierSearch {
    /// Original term order's maximum gate-time rank sum.
    pub baseline_peak: usize,
    /// Chosen order's maximum gate-time rank sum.
    pub peak: usize,
    /// Rank-only lower bound valid in this formal separate-bank model.
    pub lower_bound: usize,
    /// Exact selected input/output coordinates for every step.
    pub steps: Vec<BinaryFrontierStep>,
    /// Candidate extensions evaluated (not elapsed time).
    pub evaluated: usize,
    /// Some same-depth alternatives were dropped by the beam.
    pub beam_truncated: bool,
    /// The extension budget interrupted exploration.
    pub budget_reached: bool,
}

#[derive(Clone)]
struct FrontierOrder {
    done: u128,
    order: Vec<usize>,
    peak: usize,
}

/// Exactly checked sufficient obstruction in the separate-bank linear model.
/// A failed test is unknown, never proof of a better schedule.
#[derive(Debug, Clone)]
pub struct BinaryFrontierBarrier {
    /// Every finite algebraic condition passed.
    pub certified: bool,
    /// Rank of the entire input bank.
    pub input_rank: usize,
    /// Rank of the entire output bank.
    pub output_rank: usize,
    /// Group/functional conditions checked.
    pub checks: usize,
    /// Number of groups partitioning the paired terms.
    pub groups: usize,
    /// Any this many input groups span the input bank.
    pub input_cover: usize,
    /// Guaranteed groups activated by any nonzero input functional.
    pub active_groups: usize,
}

/// Certify a gate-time peak >= input_rank + output_rank, for every term order.
///
/// Proof: immediately before the first input-rank drop the input rank is full.
/// After that gate, a nonzero input functional f vanishes on all remaining forms;
/// all forms on which f is nonzero must have been emitted. If any k input groups
/// span the input space, f activates at least g-k+1 groups. Check that every
/// nonzero local functional's active forms have outputs spanning that group's
/// entire output space, and that any g-k+1 output groups span the output bank.
/// That gate therefore already has full output rank as well.
///
/// This does NOT cover mixing the banks, recomputing products or changing the
/// tensor. As for `binary_frontier_search`, functional independence of the formal
/// contributions must be established separately before inferring storage costs.
/// Limits: 16 groups, local input rank <=8, at most 16384 checks.
pub fn binary_frontier_barrier(
    m: MatroidRef<'_, '_>,
    inputs: &[HyperArcId],
    outputs: &[HyperArcId],
    groups: &[Vec<usize>],
    ws: &mut BinaryMatroidWorkspace,
) -> Result<BinaryFrontierBarrier, MatroidError> {
    let mut sorted: Vec<_> = groups.iter().flatten().copied().collect();
    sorted.sort_unstable();
    if inputs.is_empty()
        || inputs.len() != outputs.len()
        || inputs.len() > 128
        || groups.is_empty()
        || groups.len() > 16
        || groups.iter().any(Vec::is_empty)
        || sorted != (0..inputs.len()).collect::<Vec<_>>()
    {
        return Err(MatroidError::InvalidFrontierRequest);
    }
    let basis = |columns: &[HyperArcId], ws: &mut BinaryMatroidWorkspace| {
        let unique: BTreeSet<_> = columns.iter().copied().collect();
        binary_weighted_basis(
            m,
            &unique.into_iter().map(|a| (a, 1.0)).collect::<Vec<_>>(),
            ws,
        )
    };
    let input_rank = basis(inputs, ws)?.elements.len();
    let output_rank = basis(outputs, ws)?.elements.len();
    let mut result = BinaryFrontierBarrier {
        certified: false,
        input_rank,
        output_rank,
        checks: 0,
        groups: groups.len(),
        input_cover: 0,
        active_groups: 0,
    };
    if input_rank == 0 || output_rank == 0 {
        return Ok(result);
    }
    for group in groups {
        let ins: Vec<_> = group.iter().map(|i| inputs[*i]).collect();
        let outs: Vec<_> = group.iter().map(|i| outputs[*i]).collect();
        let b = basis(&ins, ws)?;
        let rank = b.elements.len();
        if rank == 0 || rank > 8 {
            return Ok(result);
        }
        let expected = basis(&outs, ws)?.elements.len();
        let positions: BTreeMap<_, _> = b
            .elements
            .iter()
            .enumerate()
            .map(|(i, a)| (*a, i))
            .collect();
        let coordinates: BTreeMap<_, _> = b
            .coordinates
            .iter()
            .map(|c| {
                (
                    c.arc,
                    c.support.iter().fold(0u16, |v, a| v ^ (1 << positions[a])),
                )
            })
            .collect();
        for f in 1u16..(1 << rank) {
            let active: Vec<_> = group
                .iter()
                .filter(|i| (coordinates[&inputs[**i]] & f).count_ones() % 2 == 1)
                .map(|i| outputs[*i])
                .collect();
            result.checks += 1;
            if basis(&active, ws)?.elements.len() != expected {
                return Ok(result);
            }
        }
    }
    for k in 1..=groups.len() {
        let active = groups.len() - k + 1;
        let mut valid = true;
        for mask in 1u32..(1 << groups.len()) {
            let count = mask.count_ones() as usize;
            if count != k && count != active {
                continue;
            }
            let terms: Vec<_> = groups
                .iter()
                .enumerate()
                .filter(|(i, _)| mask >> i & 1 != 0)
                .flat_map(|(_, g)| g.iter().copied())
                .collect();
            if result.checks + 2 > 16384 {
                return Ok(result);
            }
            for (needed, columns, rank) in [(k, inputs, input_rank), (active, outputs, output_rank)]
            {
                if count != needed {
                    continue;
                }
                result.checks += 1;
                if basis(&terms.iter().map(|i| columns[*i]).collect::<Vec<_>>(), ws)?
                    .elements
                    .len()
                    != rank
                {
                    valid = false;
                    break;
                }
            }
            if !valid {
                break;
            }
        }
        if valid {
            result.certified = true;
            result.input_cover = k;
            result.active_groups = active;
            break;
        }
    }
    Ok(result)
}

struct FrontierRanks<'a, 'ir, 'ws> {
    view: MatroidRef<'a, 'ir>,
    inputs: &'a [HyperArcId],
    outputs: &'a [HyperArcId],
    ws: &'ws mut BinaryMatroidWorkspace,
    cache: BTreeMap<u128, (usize, usize)>,
}
impl FrontierRanks<'_, '_, '_> {
    fn basis(&mut self, done: u128, output: bool) -> Result<BinaryBasis, MatroidError> {
        let columns = if output { self.outputs } else { self.inputs };
        let ground: BTreeSet<_> = columns
            .iter()
            .enumerate()
            .filter(|(i, _)| (done >> i & 1 != 0) == output)
            .map(|(_, a)| *a)
            .collect();
        binary_weighted_basis(
            self.view,
            &ground.into_iter().map(|a| (a, 1.0)).collect::<Vec<_>>(),
            self.ws,
        )
    }
    fn ranks(&mut self, done: u128) -> Result<(usize, usize), MatroidError> {
        if let Some(r) = self.cache.get(&done) {
            return Ok(*r);
        }
        let r = (
            self.basis(done, false)?.elements.len(),
            self.basis(done, true)?.elements.len(),
        );
        self.cache.insert(done, r);
        Ok(r)
    }
    fn evaluate(
        &mut self,
        order: &[usize],
    ) -> Result<(usize, Vec<BinaryFrontierStep>), MatroidError> {
        let mut done = 0;
        let mut peak = 0;
        let mut steps = vec![];
        for &term in order {
            let input_before = self.basis(done, false)?;
            done |= 1 << term;
            let output_after = self.basis(done, true)?;
            peak = peak.max(input_before.elements.len() + output_after.elements.len());
            steps.push(BinaryFrontierStep {
                term,
                input_before,
                output_after,
                input_rank_after: self.ranks(done)?.0,
            });
        }
        Ok((peak, steps))
    }
}

/// Search paired input-read/output-span orders using the existing binary matroid.
///
/// At a gate, count input rank BEFORE retiring this read plus output rank AFTER
/// adding this write. Input and output are distinct linear banks. Output columns
/// describe independent formal contributions; callers must separately establish
/// functional independence before treating these ranks as a storage lower bound.
/// Phase cleanup, transitions between bases and physical realizability are NOT
/// certified here. Rank cost is neither a universal quantum-space lower bound
/// nor a Toffoli count. Beam truncation is explicit; untruncated completion gives
/// the optimum only for this supplied finite paired-column family.
pub fn binary_frontier_search(
    m: MatroidRef<'_, '_>,
    inputs: &[HyperArcId],
    outputs: &[HyperArcId],
    ws: &mut BinaryMatroidWorkspace,
    beam: usize,
    budget: usize,
) -> Result<BinaryFrontierSearch, MatroidError> {
    if inputs.is_empty()
        || inputs.len() != outputs.len()
        || inputs.len() > 128
        || !(1..=256).contains(&beam)
        || !(1..=1_000_000).contains(&budget)
    {
        return Err(MatroidError::InvalidFrontierRequest);
    }
    let mut ranks = FrontierRanks {
        view: m,
        inputs,
        outputs,
        ws,
        cache: BTreeMap::new(),
    };
    let order: Vec<_> = (0..inputs.len()).collect();
    let (baseline_peak, baseline) = ranks.evaluate(&order)?;
    let initial_rank = ranks.ranks(0)?.0;
    let all = if inputs.len() == 128 {
        u128::MAX
    } else {
        (1u128 << inputs.len()) - 1
    };
    let lower_bound = initial_rank.max(ranks.ranks(all)?.1);
    let mut frontier = vec![FrontierOrder {
        done: 0,
        order: vec![],
        peak: initial_rank,
    }];
    let mut evaluated = 0;
    let mut truncated = false;
    let mut capped = false;
    for _ in 0..inputs.len() {
        let mut next = BTreeMap::<u128, FrontierOrder>::new();
        for state in frontier {
            let before = ranks.ranks(state.done)?.0;
            for term in 0..inputs.len() {
                if state.done >> term & 1 != 0 {
                    continue;
                }
                if evaluated == budget {
                    capped = true;
                    break;
                }
                evaluated += 1;
                let done = state.done | (1 << term);
                let (_, after) = ranks.ranks(done)?;
                let mut order = state.order.clone();
                order.push(term);
                let child = FrontierOrder {
                    done,
                    order,
                    peak: state.peak.max(before + after),
                };
                if next
                    .get(&done)
                    .is_none_or(|old| (child.peak, &child.order) < (old.peak, &old.order))
                {
                    next.insert(done, child);
                }
            }
            if capped {
                break;
            }
        }
        let mut rows: Vec<_> = next.into_values().collect();
        rows.sort_by(|a, b| {
            (a.peak, ranks.cache[&a.done], &a.order).cmp(&(b.peak, ranks.cache[&b.done], &b.order))
        });
        truncated |= rows.len() > beam;
        rows.truncate(beam);
        frontier = rows;
        if capped {
            break;
        }
    }
    let best = frontier
        .into_iter()
        .filter(|s| s.done == all && s.peak < baseline_peak)
        .min_by(|a, b| (a.peak, &a.order).cmp(&(b.peak, &b.order)));
    let (peak, steps) = if let Some(best) = best {
        ranks.evaluate(&best.order)?
    } else {
        (baseline_peak, baseline)
    };
    Ok(BinaryFrontierSearch {
        baseline_peak,
        peak,
        lower_bound,
        steps,
        evaluated,
        beam_truncated: truncated,
        budget_reached: capped,
    })
}
