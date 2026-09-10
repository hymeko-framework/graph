//! Full-extent queries on the SBSH point partition. Slots belong to the caller;
//! this index creates no IR identities or domain collision semantics.
use crate::{CountSplit, OrthConfig, OrthTree, build_orthtree, point_bbox};

type Bounds = ([f64; 3], [f64; 3]);

/// AABB overlay over the existing SBSH orthtree of object centres.
/// Ancestor bounds include full extents, including objects spanning partition planes.
/// Depth is bounded by 16; membership storage is O(n * depth). Dense output is O(n²).
/// Rebuild from current (or swept) bounds after motion. No stale pose cache is retained.
pub struct BoundsIndex<'a> {
    boxes: &'a [Bounds],
    tree: OrthTree,
    extents: Vec<Bounds>,
}

impl<'a> BoundsIndex<'a> {
    /// Borrow the current bounds. Invalid bounds fall back to one terminal patch,
    /// preserving the old inclusive IEEE pair predicate without unbounded splitting.
    pub fn new(boxes: &'a [Bounds]) -> Self {
        assert!(
            boxes.len() <= u32::MAX as usize,
            "SBSH point index capacity exceeded"
        );
        let valid = boxes.iter().all(|(lo, hi)| {
            (0..3).all(|k| lo[k].is_finite() && hi[k].is_finite() && lo[k] <= hi[k])
        });
        let points: Vec<_> = boxes
            .iter()
            .flat_map(|(lo, hi)| {
                (0..3).map(move |k| if valid { lo[k] * 0.5 + hi[k] * 0.5 } else { 0. })
            })
            .collect();
        let (mut lo, mut hi) = if points.is_empty() {
            (vec![-1.; 3], vec![1.; 3])
        } else {
            point_bbox(&points, 3, 0.05)
        };
        let bounded =
            valid && (0..3).all(|k| lo[k].is_finite() && hi[k].is_finite() && lo[k] < hi[k]);
        if !bounded {
            lo = vec![-1.; 3];
            hi = vec![1.; 3];
        }
        let ids: Vec<_> = (0..boxes.len() as u32).collect();
        let tree = build_orthtree(
            3,
            &lo,
            &hi,
            &points,
            &ids,
            &CountSplit { max_per_cell: 8 },
            &OrthConfig {
                max_depth: if bounded { 16 } else { 0 },
                drop_empty: true,
            },
        );
        let mut extents = vec![([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]); tree.n_cells()];
        for c in (0..tree.n_cells()).rev() {
            let mut add = |b: Bounds| {
                for k in 0..3 {
                    extents[c].0[k] = extents[c].0[k].min(b.0[k]);
                    extents[c].1[k] = extents[c].1[k].max(b.1[k]);
                }
            };
            if tree.is_leaf(c) {
                for &i in &tree.members[c] {
                    add(boxes[i as usize]);
                }
            } else {
                // Copy child bounds before mutating the parent.
                let children: Vec<_> = tree.children[c].iter().map(|&i| i as usize).collect();
                for child in children {
                    for k in 0..3 {
                        extents[c].0[k] = extents[c].0[k].min(extents[child].0[k]);
                        extents[c].1[k] = extents[c].1[k].max(extents[child].1[k]);
                    }
                }
            }
        }
        if !valid {
            extents[0] = ([f64::NEG_INFINITY; 3], [f64::INFINITY; 3]);
        }
        Self {
            boxes,
            tree,
            extents,
        }
    }

    /// Stream each inclusively overlapping pair once, i < j. Order unspecified.
    pub fn for_each_overlap(&self, mut emit: impl FnMut(usize, usize)) {
        let mut stack = vec![(0, 0)];
        while let Some((a, b)) = stack.pop() {
            if !touches(self.extents[a], self.extents[b]) {
                continue;
            }
            let (left, right) = (&self.tree.children[a], &self.tree.children[b]);
            if left.is_empty() && right.is_empty() {
                self.leaf_pairs(a, b, &mut emit);
            } else if a == b {
                for (i, &x) in left.iter().enumerate() {
                    stack.extend(left[i..].iter().map(|&y| (x as usize, y as usize)));
                }
            } else if !left.is_empty()
                && (right.is_empty() || self.tree.members[a].len() >= self.tree.members[b].len())
            {
                stack.extend(left.iter().map(|&x| (x as usize, b)));
            } else {
                stack.extend(right.iter().map(|&y| (a, y as usize)));
            }
        }
    }

    fn leaf_pairs(&self, a: usize, b: usize, emit: &mut impl FnMut(usize, usize)) {
        for (x, &i) in self.tree.members[a].iter().enumerate() {
            let start = if a == b { x + 1 } else { 0 };
            for &j in &self.tree.members[b][start..] {
                if touches(self.boxes[i as usize], self.boxes[j as usize]) {
                    emit(i.min(j) as usize, i.max(j) as usize);
                }
            }
        }
    }

    fn query(&self, test: impl Fn(Bounds) -> bool, mut emit: impl FnMut(usize)) {
        let mut stack = vec![0];
        while let Some(c) = stack.pop() {
            if !test(self.extents[c]) {
                continue;
            }
            if self.tree.is_leaf(c) {
                for &i in &self.tree.members[c] {
                    if test(self.boxes[i as usize]) {
                        emit(i as usize);
                    }
                }
            } else {
                stack.extend(self.tree.children[c].iter().map(|&i| i as usize));
            }
        }
    }

    /// Full object extents intersecting an inclusive region (not centre membership).
    pub fn for_each_in_region(&self, lo: [f64; 3], hi: [f64; 3], emit: impl FnMut(usize)) {
        self.query(|b| touches(b, (lo, hi)), emit);
    }

    /// Conservative candidates hit by a half-ray. Zero direction components are
    /// supported; a zero vector or nonfinite ray yields no hits. Caller owns exact picking.
    pub fn for_each_on_ray(&self, origin: [f64; 3], direction: [f64; 3], emit: impl FnMut(usize)) {
        if origin.iter().chain(&direction).any(|v| !v.is_finite()) || direction == [0.; 3] {
            return;
        }
        self.query(
            |b| {
                if (0..3).any(|k| b.0[k].is_nan() || b.1[k].is_nan() || b.0[k] > b.1[k]) {
                    return false;
                }
                let (mut near, mut far) = (0.0f64, f64::INFINITY);
                for k in 0..3 {
                    if direction[k] == 0. {
                        if origin[k] < b.0[k] || origin[k] > b.1[k] {
                            return false;
                        }
                    } else {
                        let a = (b.0[k] - origin[k]) / direction[k];
                        let z = (b.1[k] - origin[k]) / direction[k];
                        if a.is_nan() || z.is_nan() {
                            return false;
                        }
                        near = near.max(a.min(z));
                        far = far.min(a.max(z));
                    }
                }
                near <= far
            },
            emit,
        );
    }
}

#[inline]
fn touches(a: Bounds, b: Bounds) -> bool {
    (0..3).all(|k| a.0[k] <= b.1[k] && b.0[k] <= a.1[k])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_queries_keep_full_extents_touching_faces_and_updated_bounds() {
        let mut boxes: Vec<_> = (0..40)
            .map(|i| {
                let x = i as f64 * 3.;
                ([x, 0., 0.], [x + 1., 1., 1.])
            })
            .collect();
        boxes.push(([-100.; 3], [100.; 3]));
        for shift in [0., -30., 500.] {
            for (lo, hi) in &mut boxes {
                lo[0] += shift;
                hi[0] += shift;
            }
            let mut actual = Vec::new();
            BoundsIndex::new(&boxes)
                .for_each_in_region([1., 1., 1.], [6., 2., 2.], |i| actual.push(i));
            actual.sort_unstable();
            let expected: Vec<_> = boxes
                .iter()
                .enumerate()
                .filter(|(_, (lo, hi))| (0..3).all(|k| lo[k] <= [6., 2., 2.][k] && hi[k] >= 1.))
                .map(|(i, _)| i)
                .collect();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn rays_cover_parallel_inside_tangent_negative_and_invalid_inputs() {
        let mut boxes: Vec<_> = (0..40)
            .map(|i| {
                let x = i as f64 * 3.;
                ([x, 0., 0.], [x + 1., 1., 1.])
            })
            .collect();
        boxes.extend([
            ([-100.; 3], [100.; 3]),
            ([f64::NEG_INFINITY; 3], [f64::INFINITY; 3]),
            ([f64::NAN, 0., 0.], [1.; 3]),
            ([2.; 3], [1.; 3]),
        ]);
        for count in [40, 41, 44] {
            let index = BoundsIndex::new(&boxes[..count]);
            for (origin, direction, expected) in [
                (
                    [-1., 1., 0.5],
                    [1., 0., 0.],
                    (0..count.min(42)).collect::<Vec<_>>(),
                ),
                (
                    [0.5; 3],
                    [-1., 0., 0.],
                    [0, 40, 41].into_iter().filter(|i| *i < count).collect(),
                ),
                (
                    [-2., 2., 2.],
                    [-1., 0., 0.],
                    [40, 41].into_iter().filter(|i| *i < count).collect(),
                ),
                ([0.; 3], [0.; 3], vec![]),
                ([f64::NAN, 0., 0.], [1., 0., 0.], vec![]),
                ([0.; 3], [f64::INFINITY, 0., 0.], vec![]),
            ] {
                let mut actual = Vec::new();
                index.for_each_on_ray(origin, direction, |i| actual.push(i));
                actual.sort_unstable();
                assert_eq!(actual, expected, "count={count}, origin={origin:?}");
            }
        }
    }

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
                index.tree.n_cells() > 1,
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
        assert_eq!(index.tree.n_cells(), 1);
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
