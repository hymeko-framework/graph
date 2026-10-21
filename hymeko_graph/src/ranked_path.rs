//! Inline ranked records for signed-graph cycles and open walks.
//! Capacity differences encode shape: V == E for a cycle, V == E + 1 for a walk.

use std::cmp::Ordering;

#[derive(Clone, Debug)]
pub(crate) struct RankedPath<const V: usize, const E: usize> {
    pub(crate) score: f64,
    len: u8,
    vertices: [u32; V],
    signs: [i8; E],
}

impl<const V: usize, const E: usize> RankedPath<V, E> {
    /// Callers supply the shape encoded by the two capacities. One length byte
    /// suffices; the record never allocates. Existing owners use 8/8 and 17/16.
    #[inline]
    pub(crate) fn from_slices(score: f64, vertices: &[u32], signs: &[i8]) -> Self {
        debug_assert!(V >= E && E <= u8::MAX as usize);
        debug_assert_eq!(vertices.len(), signs.len() + V - E);
        debug_assert!(signs.len() <= E);
        let mut record = Self {
            score,
            len: signs.len() as u8,
            vertices: [0; V],
            signs: [0; E],
        };
        record.vertices[..vertices.len()].copy_from_slice(vertices);
        record.signs[..signs.len()].copy_from_slice(signs);
        record
    }

    #[inline]
    pub(crate) fn vertices(&self) -> &[u32] {
        &self.vertices[..self.len as usize + V - E]
    }
    #[inline]
    pub(crate) fn signs_slice(&self) -> &[i8] {
        &self.signs[..self.len as usize]
    }

    /// Higher score wins, then lexicographically larger vertices. Signs do not
    /// participate. Raw NaN/zero bits stay stored; only comparisons normalize NaN.
    pub(crate) fn cmp_preference(&self, other: &Self) -> Ordering {
        self.cmp_preference_vs_slice(other.score, other.vertices())
    }

    pub(crate) fn cmp_preference_vs_slice(&self, other_score: f64, vertices: &[u32]) -> Ordering {
        fn comparable(score: f64) -> f64 {
            if score.is_nan() {
                f64::NEG_INFINITY
            } else {
                score
            }
        }
        comparable(self.score)
            .partial_cmp(&comparable(other_score))
            .unwrap_or(Ordering::Equal)
            .then_with(|| self.vertices().cmp(vertices))
    }
}

impl<const V: usize, const E: usize> Eq for RankedPath<V, E> {}
impl<const V: usize, const E: usize> PartialEq for RankedPath<V, E> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp_preference(other).is_eq()
    }
}
impl<const V: usize, const E: usize> PartialOrd for RankedPath<V, E> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<const V: usize, const E: usize> Ord for RankedPath<V, E> {
    // BinaryHeap removes the greatest Ord entry: reverse preference so its
    // root is the next eviction candidate.
    fn cmp(&self, other: &Self) -> Ordering {
        other.cmp_preference(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BinaryHeap;

    fn ordering_laws<const V: usize, const E: usize>(vertices: &[u32], signs: &[i8]) {
        let records: Vec<_> = [
            f64::NAN,
            f64::NEG_INFINITY,
            -2.0,
            -0.0,
            0.0,
            1.0,
            f64::INFINITY,
        ]
        .into_iter()
        .map(|score| RankedPath::<V, E>::from_slices(score, vertices, signs))
        .collect();
        for a in &records {
            assert_eq!(a, &a.clone());
            assert_eq!(a.vertices(), vertices);
            assert_eq!(a.signs_slice(), signs);
            for b in &records {
                assert_eq!(a.cmp(b), b.cmp(a).reverse());
                assert_eq!(a.partial_cmp(b), Some(a.cmp(b)));
                for c in &records {
                    if a <= b && b <= c {
                        assert!(a <= c);
                    }
                }
            }
        }
        let mut heap: BinaryHeap<_> = records.into_iter().collect();
        let mut previous = heap.pop().unwrap();
        while let Some(next) = heap.pop() {
            assert!(!previous.cmp_preference(&next).is_gt());
            previous = next;
        }
    }

    #[test]
    fn cycle_and_walk_shapes_obey_one_preference_order() {
        ordering_laws::<8, 8>(&[0, 1, 2], &[1, -1, 1]);
        ordering_laws::<17, 16>(&[0, 1, 2, 3], &[1, -1, 1]);
        let cycle = RankedPath::<8, 8>::from_slices(-0.0, &[1; 8], &[-1; 8]);
        let walk = RankedPath::<17, 16>::from_slices(f64::NAN, &[2; 17], &[1; 16]);
        assert_eq!(cycle.score.to_bits(), (-0.0f64).to_bits());
        assert_eq!(walk.vertices().len(), 17);
        assert_eq!(walk.signs_slice().len(), 16);
        assert_eq!(walk.score.to_bits(), f64::NAN.to_bits());
        let better = RankedPath::<8, 8>::from_slices(0.0, &[2; 8], &[1; 8]);
        assert!(better.cmp_preference(&cycle).is_gt());
        assert!(cycle.cmp_preference_vs_slice(0.0, &[2; 8]).is_lt());
        assert!(
            std::panic::catch_unwind(|| RankedPath::<8, 8>::from_slices(0.0, &[1; 9], &[1; 9]))
                .is_err()
        );
    }
}
