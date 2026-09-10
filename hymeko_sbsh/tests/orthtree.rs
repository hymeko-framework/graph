//! The orthtree across dimensions, and the laminar invariant the width-1 backbone rests on.

use hymeko_sbsh::{CountSplit, OrthConfig, build_orthtree, point_bbox};

#[test]
fn leaf_lookup_uses_half_open_internal_faces_and_dropped_orthants() {
    let tree = build_orthtree(
        2,
        &[0., 0.],
        &[2., 2.],
        &[0.25, 0.25, 1., 1., 1.75, 1.75],
        &[0, 1, 2],
        &CountSplit { max_per_cell: 1 },
        &OrthConfig {
            max_depth: 2,
            drop_empty: true,
        },
    );
    for (i, p) in [[0.25, 0.25], [1., 1.], [1.75, 1.75]].iter().enumerate() {
        let cell = tree.leaf_of(p).unwrap();
        assert!(tree.members[cell].contains(&(i as u32)));
    }
    assert!(tree.leaf_of(&[0.25, 1.75]).is_none());
    assert!(tree.leaf_of(&[2.01, 2.]).is_none());
    assert!(tree.leaf_of(&[f64::NAN, 0.]).is_none());
    assert!(tree.members[tree.leaf_of(&[2., 2.]).unwrap()].contains(&2));
    tree.verify_laminar().unwrap();
}

/// A deterministic point cloud in `dim` dimensions — no RNG dependency, reproducible everywhere.
fn cloud(n: usize, dim: usize) -> Vec<f64> {
    (0..n * dim)
        .map(|i| {
            let x = ((i * 2654435761usize) % 1000) as f64 / 1000.0;
            x * 10.0
        })
        .collect()
}

#[test]
fn one_code_path_covers_quadtree_octree_and_beyond() {
    for dim in 1..=4usize {
        let pts = cloud(60, dim);
        let (lo, hi) = point_bbox(&pts, dim, 0.05);
        let ids: Vec<u32> = (0..60).collect();
        let t = build_orthtree(
            dim,
            &lo,
            &hi,
            &pts,
            &ids,
            &CountSplit { max_per_cell: 4 },
            &OrthConfig {
                max_depth: 6,
                drop_empty: true,
            },
        );
        assert_eq!(t.dim, dim);
        assert!(t.n_cells() >= 1);
        t.verify_laminar()
            .unwrap_or_else(|e| panic!("dim {dim} is not laminar: {e}"));
        // dim 2 is a quadtree (<= 4 children), dim 3 an octree (<= 8): the same code.
        for c in 0..t.n_cells() {
            assert!(
                t.children[c].len() <= 1usize << dim,
                "cell {c} in dim {dim} has more than 2^dim children"
            );
        }
    }
}

#[test]
fn every_point_lands_in_exactly_one_leaf() {
    let (n, dim) = (80usize, 3usize);
    let pts = cloud(n, dim);
    let (lo, hi) = point_bbox(&pts, dim, 0.05);
    let ids: Vec<u32> = (0..n as u32).collect();
    let t = build_orthtree(
        dim,
        &lo,
        &hi,
        &pts,
        &ids,
        &CountSplit { max_per_cell: 3 },
        &OrthConfig {
            max_depth: 8,
            drop_empty: true,
        },
    );
    // The partition property: the leaf member sets are disjoint and cover every point.
    let mut seen = vec![0usize; n];
    for c in t.leaves() {
        for &m in &t.members[c] {
            seen[m as usize] += 1;
        }
    }
    assert!(
        seen.iter().all(|&k| k == 1),
        "leaves must partition the points, got {seen:?}"
    );

    // And descending finds the same leaf the membership says.
    for i in 0..n {
        let p = &pts[i * dim..(i + 1) * dim];
        let leaf = t.leaf_of(p).expect("point inside the padded root box");
        assert!(t.is_leaf(leaf));
        assert!(
            t.members[leaf].contains(&(i as u32)),
            "point {i} not a member of its own leaf"
        );
    }
}

#[test]
fn a_range_query_agrees_with_brute_force() {
    let (n, dim) = (120usize, 2usize);
    let pts = cloud(n, dim);
    let (lo, hi) = point_bbox(&pts, dim, 0.05);
    let ids: Vec<u32> = (0..n as u32).collect();
    let t = build_orthtree(
        dim,
        &lo,
        &hi,
        &pts,
        &ids,
        &CountSplit { max_per_cell: 4 },
        &OrthConfig {
            max_depth: 8,
            drop_empty: true,
        },
    );
    for &(qlo, qhi) in &[
        ([1.0, 1.0], [4.0, 4.0]),
        ([0.0, 0.0], [10.0, 10.0]),
        ([9.9, 9.9], [10.0, 10.0]),
    ] {
        let got = t.objects_in_region(&pts, &qlo, &qhi);
        let want: Vec<u32> = (0..n as u32)
            .filter(|&i| {
                (0..dim).all(|d| {
                    let p = pts[i as usize * dim + d];
                    p >= qlo[d] && p < qhi[d]
                })
            })
            .collect();
        assert_eq!(
            got, want,
            "pruned query must equal brute force on [{qlo:?}, {qhi:?})"
        );
    }
}

#[test]
fn max_depth_caps_subdivision_even_when_the_rule_wants_more() {
    let (n, dim) = (200usize, 2usize);
    let pts = cloud(n, dim);
    let (lo, hi) = point_bbox(&pts, dim, 0.05);
    let ids: Vec<u32> = (0..n as u32).collect();
    let t = build_orthtree(
        dim,
        &lo,
        &hi,
        &pts,
        &ids,
        &CountSplit { max_per_cell: 1 }, // wants to split almost everywhere
        &OrthConfig {
            max_depth: 2,
            drop_empty: true,
        },
    );
    assert!(
        t.level.iter().all(|&l| l <= 2),
        "max_depth must cap the tree"
    );
}

#[test]
fn dropping_empty_orthants_keeps_the_tree_sparse_without_losing_points() {
    let (n, dim) = (40usize, 3usize);
    let pts = cloud(n, dim);
    let (lo, hi) = point_bbox(&pts, dim, 0.05);
    let ids: Vec<u32> = (0..n as u32).collect();
    let cfg_drop = OrthConfig {
        max_depth: 5,
        drop_empty: true,
    };
    let cfg_keep = OrthConfig {
        max_depth: 5,
        drop_empty: false,
    };
    let rule = CountSplit { max_per_cell: 2 };
    let sparse = build_orthtree(dim, &lo, &hi, &pts, &ids, &rule, &cfg_drop);
    let dense = build_orthtree(dim, &lo, &hi, &pts, &ids, &rule, &cfg_keep);
    assert!(
        sparse.n_cells() <= dense.n_cells(),
        "dropping empties cannot grow the tree"
    );
    let count = |t: &hymeko_sbsh::OrthTree| -> usize {
        t.leaves().iter().map(|&c| t.members[c].len()).sum()
    };
    assert_eq!(
        count(&sparse),
        n,
        "no point may be lost by pruning empty orthants"
    );
    assert_eq!(count(&dense), n);
    for tree in [&sparse, &dense] {
        assert_eq!(tree.members[0], ids);
        for cell in 0..tree.n_cells() {
            if tree.is_leaf(cell) {
                continue;
            }
            let mut descendants: Vec<_> = tree.children[cell]
                .iter()
                .flat_map(|&child| tree.members[child as usize].iter().copied())
                .collect();
            descendants.sort_unstable();
            assert_eq!(descendants, tree.members[cell]);
        }
        tree.verify_laminar().unwrap();
    }
}

#[test]
fn a_degenerate_axis_still_yields_a_usable_root_box() {
    // Every point shares one coordinate: the span is zero and a naive bbox would be empty.
    let pts = vec![1.0, 5.0, 2.0, 5.0, 3.0, 5.0];
    let (lo, hi) = point_bbox(&pts, 2, 0.05);
    assert!(
        hi[1] > lo[1],
        "a degenerate axis must be widened, got [{}, {})",
        lo[1],
        hi[1]
    );
    let t = build_orthtree(
        2,
        &lo,
        &hi,
        &pts,
        &[0, 1, 2],
        &CountSplit { max_per_cell: 1 },
        &OrthConfig::default(),
    );
    t.verify_laminar().unwrap();
    assert_eq!(
        t.leaves()
            .iter()
            .map(|&c| t.members[c].len())
            .sum::<usize>(),
        3
    );
}
