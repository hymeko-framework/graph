//! Transient eight-way spatial indexing of projected object bounds.
//! Slot numbers are caller-owned observations, never new canonical IR identities.

type Bounds = ([f64; 3], [f64; 3]);

struct Node {
    bounds: Bounds,
    start: usize,
    end: usize,
    children: Vec<usize>,
}

/// Octree broad-phase accelerator over borrowed, world-space AABBs.
///
/// Full object extents contribute to every ancestor bound, so objects crossing
/// partition planes or spanning many octants cannot be missed. This is derived
/// scratch state, not an authored subdivision or a second scene/identity model.
/// Construction uses O(n) storage, with depth bounded by 16; buckets stop at
/// eight objects or when their centres cannot be separated. Dense overlap still
/// requires O(n²) output work. Rebuild after changing the source bounds.
pub struct BoundsIndex<'a> {
    boxes: &'a [Bounds],
    slots: Vec<usize>,
    nodes: Vec<Node>,
}

impl<'a> BoundsIndex<'a> {
    /// Index the current bounds without modifying them.
    ///
    /// Finite ordered bounds give spatial pruning. Non-finite or inverted bounds
    /// are retained in terminal buckets and use the same inclusive IEEE
    /// comparisons as a direct pair scan; they never cause unbounded subdivision.
    /// Empty inputs are valid. No caller identity or numerical state is changed.
    pub fn new(boxes: &'a [Bounds]) -> Self {
        let mut index = Self {
            boxes,
            slots: (0..boxes.len()).collect(),
            nodes: Vec::new(),
        };
        if !boxes.is_empty() {
            index.partition(0, boxes.len(), 0);
        }
        index
    }

    /// Stream each pair of overlapping bounds exactly once, with `i < j`.
    ///
    /// Face/edge/point touching counts as overlap. Callback order is unspecified;
    /// consumers needing stable lexicographic output must sort their observations.
    /// Source bounds stay borrowed and immutable for the lifetime of this index.
    pub fn for_each_overlap(&self, mut emit: impl FnMut(usize, usize)) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![(0, 0)];
        while let Some((a, b)) = stack.pop() {
            let (left, right) = (&self.nodes[a], &self.nodes[b]);
            if !touches(left.bounds, right.bounds) {
                continue;
            }
            if left.children.is_empty() && right.children.is_empty() {
                self.leaf_pairs(a, b, &mut emit);
            } else if a == b {
                // Unordered sibling pairs, including each child's own subtree.
                for (i, &x) in left.children.iter().enumerate() {
                    stack.extend(left.children[i..].iter().map(|&y| (x, y)));
                }
            } else if !left.children.is_empty()
                && (right.children.is_empty() || left.end - left.start >= right.end - right.start)
            {
                stack.extend(left.children.iter().map(|&x| (x, b)));
            } else {
                stack.extend(right.children.iter().map(|&y| (a, y)));
            }
        }
    }

    fn leaf_pairs(&self, a: usize, b: usize, emit: &mut impl FnMut(usize, usize)) {
        let (left, right) = (&self.nodes[a], &self.nodes[b]);
        for x in left.start..left.end {
            let i = self.slots[x];
            let bounds = self.boxes[i];
            let start = if a == b { x + 1 } else { right.start };
            for &j in &self.slots[start..right.end] {
                if touches(bounds, self.boxes[j]) {
                    emit(i.min(j), i.max(j));
                }
            }
        }
    }

    fn partition(&mut self, start: usize, end: usize, depth: usize) -> usize {
        let mut bounds = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        let mut centres = bounds;
        let mut valid = true;
        for &i in &self.slots[start..end] {
            let (lo, hi) = self.boxes[i];
            for k in 0..3 {
                valid &= lo[k].is_finite() && hi[k].is_finite() && lo[k] <= hi[k];
                bounds.0[k] = bounds.0[k].min(lo[k]);
                bounds.1[k] = bounds.1[k].max(hi[k]);
                let centre = lo[k] * 0.5 + hi[k] * 0.5;
                centres.0[k] = centres.0[k].min(centre);
                centres.1[k] = centres.1[k].max(centre);
            }
        }
        // Invalid geometry cannot tighten pruning: terminal all-space bounds
        // preserve the exact per-pair IEEE predicate, including infinities.
        if !valid {
            bounds = ([f64::NEG_INFINITY; 3], [f64::INFINITY; 3]);
        }
        let node = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            start,
            end,
            children: Vec::new(),
        });
        if end - start <= 8 || depth == 16 || !valid {
            return node;
        }
        let mid = std::array::from_fn(|k| centres.0[k] * 0.5 + centres.1[k] * 0.5);
        let boxes = self.boxes;
        self.slots[start..end].sort_unstable_by_key(|&i| octant(boxes[i], mid));
        if octant(boxes[self.slots[start]], mid) == octant(boxes[self.slots[end - 1]], mid) {
            return node;
        }
        let mut first = start;
        while first < end {
            let code = octant(boxes[self.slots[first]], mid);
            let mut last = first + 1;
            while last < end && octant(boxes[self.slots[last]], mid) == code {
                last += 1;
            }
            let child = self.partition(first, last, depth + 1);
            self.nodes[node].children.push(child);
            first = last;
        }
        node
    }
}

fn octant((lo, hi): Bounds, mid: [f64; 3]) -> u8 {
    (0..3).fold(0, |code, k| {
        code | (u8::from(lo[k] * 0.5 + hi[k] * 0.5 >= mid[k]) << k)
    })
}

#[inline]
fn touches(a: Bounds, b: Bounds) -> bool {
    (0..3).all(|k| a.0[k] <= b.1[k] && b.0[k] <= a.1[k])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_pairs_equal_inclusive_reference_for_extents_boundaries_and_motion() {
        let mut boxes: Vec<_> = (0..1000)
            .map(|i| {
                let lo = [(i % 11) as f64, ((i / 11) % 11) as f64, (i / 121) as f64];
                (lo, lo.map(|v| v + 1.0))
            })
            .collect();
        boxes.push(([-100.0; 3], [100.0; 3]));
        boxes.push(([0.0; 3], [0.0; 3]));
        for _ in 0..3 {
            let mut expected = Vec::new();
            // Closed-form AABB intersection, independent of partition/traversal.
            for i in 0..boxes.len() {
                for j in i + 1..boxes.len() {
                    if (0..3)
                        .all(|k| boxes[i].0[k] <= boxes[j].1[k] && boxes[j].0[k] <= boxes[i].1[k])
                    {
                        expected.push((i, j));
                    }
                }
            }
            let mut actual = Vec::new();
            let index = BoundsIndex::new(&boxes);
            assert!(
                index.nodes.len() > 1,
                "spanning object must not disable partitioning"
            );
            index.for_each_overlap(|i, j| actual.push((i, j)));
            actual.sort_unstable();
            assert_eq!(actual, expected);
            boxes.rotate_left(17);
            for (lo, hi) in &mut boxes {
                for k in 0..3 {
                    lo[k] = lo[k] * 0.01 - 3.0;
                    hi[k] = hi[k] * 0.01 - 3.0;
                }
            }
        }
    }

    #[test]
    fn empty_dense_and_nonfinite_bounds_terminate_without_losing_pairs() {
        let mut actual = Vec::new();
        BoundsIndex::new(&[]).for_each_overlap(|i, j| actual.push((i, j)));
        assert!(actual.is_empty());
        let boxes = vec![([0.0; 3], [1.0; 3]); 40];
        let index = BoundsIndex::new(&boxes);
        assert_eq!(index.nodes.len(), 1);
        index.for_each_overlap(|i, j| actual.push((i, j)));
        assert_eq!(actual.len(), 40 * 39 / 2);
        let boxes = [
            ([f64::NEG_INFINITY; 3], [f64::INFINITY; 3]),
            ([0.0; 3], [1.0; 3]),
            ([f64::NAN; 3], [1.0; 3]),
        ];
        actual.clear();
        BoundsIndex::new(&boxes).for_each_overlap(|i, j| actual.push((i, j)));
        assert_eq!(actual, [(0, 1)]);
    }
}
