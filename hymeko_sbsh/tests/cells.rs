//! The leaves as cells of the partition: which leaves a box meets, and which leaves share a face.

use hymeko_sbsh::{CountSplit, OrthConfig, OrthTree, SplitRule, build_orthtree, point_bbox};

/// Split by position alone, never by points: a tree over space rather than over a point set.
struct Region(fn(&[f64], u32) -> bool);

impl SplitRule for Region {
    fn should_split(&self, lo: &[f64], _hi: &[f64], level: u32, _members: &[u32]) -> bool {
        (self.0)(lo, level)
    }
}

fn space_tree(dim: usize, side: f64, rule: fn(&[f64], u32) -> bool, max_depth: u32) -> OrthTree {
    build_orthtree(
        dim,
        &vec![0.0; dim],
        &vec![side; dim],
        &[],
        &[],
        &Region(rule),
        &OrthConfig {
            max_depth,
            drop_empty: false,
        },
    )
}

fn cloud(n: usize, dim: usize) -> Vec<f64> {
    (0..n * dim)
        .map(|i| ((i * 2654435761usize) % 1000) as f64 / 100.0)
        .collect()
}

/// A tree over `n` cloud points at depth up to 5, and its root's far corner.
fn point_tree(dim: usize, n: u32, max_per_cell: usize, drop_empty: bool) -> (OrthTree, Vec<f64>) {
    let points = cloud(n as usize, dim);
    let (lo, hi) = point_bbox(&points, dim, 0.0);
    let ids: Vec<u32> = (0..n).collect();
    let config = OrthConfig {
        max_depth: 5,
        drop_empty,
    };
    let tree = build_orthtree(
        dim,
        &lo,
        &hi,
        &points,
        &ids,
        &CountSplit { max_per_cell },
        &config,
    );
    (tree, hi)
}

/// The definition, pair by pair: one axis where one box ends and the other begins, positive
/// overlap on every other axis.
fn adjacency_by_definition(tree: &OrthTree) -> Vec<(u32, u32)> {
    let leaves = tree.leaves();
    let mut pairs = Vec::new();
    for (i, &a) in leaves.iter().enumerate() {
        for &b in &leaves[i + 1..] {
            let (alo, ahi, blo, bhi) = (
                tree.cell_lo(a),
                tree.cell_hi(a),
                tree.cell_lo(b),
                tree.cell_hi(b),
            );
            let touching: Vec<usize> = (0..tree.dim)
                .filter(|&d| ahi[d] == blo[d] || bhi[d] == alo[d])
                .collect();
            let overlapping = (0..tree.dim)
                .filter(|&d| alo[d].max(blo[d]) < ahi[d].min(bhi[d]))
                .count();
            if touching.len() == 1 && overlapping == tree.dim - 1 {
                pairs.push((a as u32, b as u32));
            }
        }
    }
    pairs
}

/// The definition of meeting: the leaf's closed box and the closed query box intersect.
fn meeting_by_definition(tree: &OrthTree, qlo: &[f64], qhi: &[f64]) -> Vec<usize> {
    tree.leaves()
        .into_iter()
        .filter(|&c| {
            let (lo, hi) = (tree.cell_lo(c), tree.cell_hi(c));
            (0..tree.dim).all(|d| lo[d] <= qhi[d] && qlo[d] <= hi[d])
        })
        .collect()
}

#[test]
fn a_uniform_quadtree_is_the_grid_graph() {
    let tree = space_tree(2, 4.0, |_, _| true, 2);
    assert_eq!(tree.leaves().len(), 16);
    let pairs = tree.face_adjacency();
    // A 4 x 4 grid: 3 neighbours along each of 4 rows and 4 columns, no diagonals.
    assert_eq!(pairs.len(), 24);
    for &(a, b) in &pairs {
        let (ca, cb) = (tree.cell_lo(a as usize), tree.cell_lo(b as usize));
        let steps: f64 = (0..2).map(|d| (ca[d] - cb[d]).abs()).sum();
        assert_eq!(steps, 1.0, "{a} and {b} are not face neighbours");
    }
    assert_eq!(pairs, adjacency_by_definition(&tree));
}

/// A refined quadrant borders its coarse neighbours through each fine cell on the shared face;
/// the two coarse quadrants that meet only at the centre corner are not adjacent.
#[test]
fn adjacency_crosses_levels_and_ignores_corners() {
    let tree = space_tree(
        2,
        2.0,
        |lo, level| level == 0 || (level == 1 && lo == [0.0, 0.0]),
        3,
    );
    assert_eq!(tree.leaves().len(), 7);
    // 4 inside the refined quadrant, 2 + 2 across its two coarse borders, 2 between coarse ones.
    assert_eq!(tree.face_adjacency().len(), 10);
    assert_eq!(tree.face_adjacency(), adjacency_by_definition(&tree));
}

#[test]
fn face_adjacency_agrees_with_the_definition_in_two_and_three_dimensions() {
    for dim in [2, 3] {
        for drop_empty in [false, true] {
            let (tree, _) = point_tree(dim, 120, 3, drop_empty);
            let pairs = tree.face_adjacency();
            assert!(!pairs.is_empty());
            assert_eq!(
                pairs,
                adjacency_by_definition(&tree),
                "dim {dim}, drop_empty {drop_empty}"
            );
        }
    }
}

#[test]
fn leaves_meeting_agrees_with_the_definition() {
    let dim = 3;
    let (tree, hi) = point_tree(dim, 200, 4, true);
    let corners = cloud(80, dim);
    for pair in corners.chunks(2 * dim) {
        let qlo: Vec<f64> = (0..dim)
            .map(|d| pair[d].min(pair[dim + d]) * 1.1 - 0.5)
            .collect();
        let qhi: Vec<f64> = (0..dim)
            .map(|d| pair[d].max(pair[dim + d]) * 1.1 - 0.5)
            .collect();
        assert_eq!(
            tree.leaves_meeting(&qlo, &qhi),
            meeting_by_definition(&tree, &qlo, &qhi)
        );
    }
    // A degenerate box on the root's far corner, the boundary point location treats specially.
    assert_eq!(
        tree.leaves_meeting(&hi, &hi),
        meeting_by_definition(&tree, &hi, &hi)
    );
}

/// Touching is meeting, from either side: a box that ends on an internal face meets the leaves on
/// both sides, the centre point meets all four quadrants, and a box beyond the root meets nothing.
#[test]
fn a_box_on_a_face_meets_both_sides() {
    let tree = space_tree(2, 2.0, |_, level| level == 0, 1);
    assert_eq!(tree.leaves().len(), 4);
    for (qlo, qhi, meets) in [
        ([0.5, 0.25], [1.0, 0.75], 2),
        ([1.0, 0.25], [1.5, 0.75], 2),
        ([1.0, 1.0], [1.0, 1.0], 4),
        ([2.5, 2.5], [3.0, 3.0], 0),
    ] {
        assert_eq!(
            tree.leaves_meeting(&qlo, &qhi).len(),
            meets,
            "{qlo:?}..{qhi:?}"
        );
    }
}
