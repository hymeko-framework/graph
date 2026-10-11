//! The orthtree across dimensions, and the laminar invariant the width-1 backbone rests on.

use hymeko_sbsh::{CountSplit, Laminar, OrthConfig, build_orthtree, point_bbox};

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
    // The dropped orthant has no leaf, but its point still has its chain: the root alone.
    assert_eq!(tree.path(&[0.25, 1.75]), vec![0]);
    assert!(tree.path(&[2.01, 2.]).is_empty());
    tree.verify_laminar().unwrap();
}

/// The path is every cell that holds the point, root first: over a sparse 3-D tree and query
/// points inside, on the far face, in dropped orthants and outside, it equals the brute-force set
/// in level order, each cell the parent of the next, and `leaf_of` is its last cell exactly when
/// that cell is a leaf.
#[test]
fn a_path_is_every_cell_holding_its_point() {
    let dim = 3;
    let points = cloud(200, dim);
    let (lo, hi) = point_bbox(&points, dim, 0.0);
    let ids: Vec<u32> = (0..200).collect();
    let tree = build_orthtree(
        dim,
        &lo,
        &hi,
        &points,
        &ids,
        &CountSplit { max_per_cell: 4 },
        &OrthConfig {
            max_depth: 5,
            drop_empty: true,
        },
    );
    let mut queries: Vec<Vec<f64>> = cloud(150, dim)
        .chunks(dim)
        .map(|q| q.iter().map(|x| x * 1.1 - 0.4).collect())
        .collect();
    queries.push(hi.clone());
    queries.extend(points.chunks(dim).take(20).map(<[f64]>::to_vec));
    let mut reused = Vec::new();
    let (mut inside, mut stopped) = (0, 0);
    for q in &queries {
        let path = tree.path(q);
        let mut holding: Vec<usize> = (0..tree.n_cells()).filter(|&c| tree.holds(c, q)).collect();
        holding.sort_by_key(|&c| tree.level[c]);
        assert_eq!(path, holding, "{q:?}");
        for pair in path.windows(2) {
            assert_eq!(tree.parent[pair[1]], pair[0] as i32);
        }
        tree.path_into(q, &mut reused);
        assert_eq!(reused, path);
        let leaf = path.last().copied().filter(|&c| tree.is_leaf(c));
        assert_eq!(tree.leaf_of(q), leaf);
        inside += usize::from(!path.is_empty());
        stopped += usize::from(!path.is_empty() && leaf.is_none());
    }
    assert!(
        inside > 20 && stopped > 0,
        "the queries reach both cases: {inside}, {stopped}"
    );
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

/// The coordinate type is a parameter. Over coordinates both `f32` and `f64` represent exactly, the
/// two builds split at the same places: the same cells, levels, parents, children and members, and
/// boxes equal after widening.
#[test]
fn an_f32_tree_is_the_f64_tree_over_exact_coordinates() {
    for dim in 1..=3usize {
        // Quarter-integer coordinates on a small lattice: exact in both precisions.
        let n = 60u32;
        let wide: Vec<f64> = (0..n as usize * dim)
            .map(|i| ((i * 7 + 3) % 29) as f64 * 0.25)
            .collect();
        let narrow: Vec<f32> = wide.iter().map(|&v| v as f32).collect();
        let ids: Vec<u32> = (0..n).collect();
        let rule = CountSplit { max_per_cell: 3 };
        let cfg = OrthConfig::default();
        let (lo, hi) = point_bbox(&wide, dim, 0.0);
        let (lo32, hi32) = point_bbox(&narrow, dim, 0.0);
        assert_eq!(lo32, lo.iter().map(|&v| v as f32).collect::<Vec<_>>());
        assert_eq!(hi32, hi.iter().map(|&v| v as f32).collect::<Vec<_>>());
        let a = build_orthtree(dim, &lo, &hi, &wide, &ids, &rule, &cfg);
        let b = build_orthtree(dim, &lo32, &hi32, &narrow, &ids, &rule, &cfg);
        b.verify_laminar().unwrap();
        assert_eq!(
            (&a.level, &a.parent, &a.children, &a.members),
            (&b.level, &b.parent, &b.children, &b.members),
            "dim {dim}"
        );
        let widen = |v: &[f32]| v.iter().map(|&x| f64::from(x)).collect::<Vec<_>>();
        assert_eq!(a.lo, widen(&b.lo), "dim {dim}");
        assert_eq!(a.hi, widen(&b.hi), "dim {dim}");
        for p in 0..n as usize {
            assert_eq!(
                a.leaf_of(&wide[p * dim..(p + 1) * dim]),
                b.leaf_of(&narrow[p * dim..(p + 1) * dim])
            );
        }
    }
}

/// A caller's own rule is written once for every coordinate type it supports.
#[test]
fn a_split_rule_can_be_generic_over_the_coordinate() {
    struct Wider<T>(T);
    impl<T: hymeko_sbsh::Coordinate> hymeko_sbsh::SplitRule<T> for Wider<T> {
        fn should_split(&self, lo: &[T], hi: &[T], _level: u32, members: &[u32]) -> bool {
            members.len() > 1 && hi[0] - lo[0] > self.0
        }
    }
    let pts = [0.0f32, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let ids: Vec<u32> = (0..8).collect();
    let (lo, hi) = point_bbox(&pts, 1, 0.0);
    let coarse = build_orthtree(
        1,
        &lo,
        &hi,
        &pts,
        &ids,
        &Wider(4.0f32),
        &OrthConfig::default(),
    );
    let fine = build_orthtree(
        1,
        &lo,
        &hi,
        &pts,
        &ids,
        &Wider(1.0f32),
        &OrthConfig::default(),
    );
    assert!(coarse.leaves().len() < fine.leaves().len());
}

/// A lattice coordinate splits at the floor midpoint, without the overflow `(lo + hi) / 2` has, and
/// a unit side cannot be halved. The float midpoint is unchanged.
#[test]
fn a_lattice_coordinate_splits_at_the_floor_midpoint() {
    use hymeko_sbsh::Coordinate;
    assert_eq!(<u32 as Coordinate>::midpoint(0, 11), 5);
    assert_eq!(<usize as Coordinate>::midpoint(2, 5), 3);
    assert_eq!(<u64 as Coordinate>::midpoint(7, 8), 7);
    assert_eq!(
        <u32 as Coordinate>::midpoint(u32::MAX - 3, u32::MAX),
        u32::MAX - 2
    );
    assert_eq!(<f64 as Coordinate>::midpoint(2.0, 5.0), 3.5);
    assert_eq!(<f32 as Coordinate>::midpoint(2.0, 5.0), 3.5);
}

/// A raster over an integer lattice, with every orthant kept: cells have integer corners at the
/// floor midpoints, no cell is without volume, and the leaves tile the grid exactly, including
/// where a unit side stops one axis from halving while the other still can.
#[test]
fn a_lattice_raster_tree_tiles_the_grid_with_integer_cells() {
    struct Always;
    impl hymeko_sbsh::SplitRule<usize> for Always {
        fn should_split(&self, _lo: &[usize], _hi: &[usize], _level: u32, _m: &[u32]) -> bool {
            true
        }
    }
    let (w, h) = (11usize, 3usize);
    let tree = build_orthtree(
        2,
        &[0, 0],
        &[w, h],
        &[],
        &[],
        &Always,
        &OrthConfig {
            max_depth: 6,
            drop_empty: false,
        },
    );
    tree.verify_laminar().unwrap();
    let kids = &tree.children[0];
    assert_eq!(kids.len(), 4);
    let top = kids[3] as usize;
    assert_eq!(
        (tree.cell_lo(top), tree.cell_hi(top)),
        (&[5, 1][..], &[11, 3][..])
    );
    for c in 0..tree.n_cells() {
        assert!(
            (0..2).all(|d| tree.cell_lo(c)[d] < tree.cell_hi(c)[d]),
            "cell {c} has no volume: {:?}..{:?}",
            tree.cell_lo(c),
            tree.cell_hi(c)
        );
    }
    let mut cover = vec![0u32; w * h];
    for c in tree.leaves() {
        let (lo, hi) = (tree.cell_lo(c), tree.cell_hi(c));
        for y in lo[1]..hi[1] {
            for x in lo[0]..hi[0] {
                cover[y * w + x] += 1;
            }
        }
    }
    assert!(cover.iter().all(|&n| n == 1), "leaves must tile the grid");
    assert_eq!(
        tree.leaves().len(),
        w * h,
        "depth 6 reaches single lattice points"
    );
}
