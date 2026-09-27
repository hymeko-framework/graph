//! `CellTopology`: graded signed cells, their width-`d` contractions, and `SignedHyperIncidence`
//! as the one-scalar, one-rank case of the same kernels.

use hymeko_graph::cell_topology::{
    CellTopology, CellTopologyError, RankIncidence, RankScales, ScalePolicy,
};
use hymeko_graph::hyper_operator::SignedHyperIncidence;

mod cell_rank_common;
use cell_rank_common as fresh;

fn fingerprint(values: &[f32]) -> u64 {
    values.iter().fold(0xcbf2_9ce4_8422_2325, |h, v| {
        v.to_bits()
            .to_le_bytes()
            .iter()
            .fold(h, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
    })
}

fn lcg(seed: u64) -> impl FnMut() -> f32 {
    let mut state = seed;
    move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((state >> 40) as f32) / (1u64 << 24) as f32 - 0.5
    }
}

/// Seven vertices, five triangles, mixed signs, a repeated member, non-unit scale.
const MEMBERS: [u32; 15] = [0, 1, 2, 1, 2, 3, 3, 4, 5, 5, 6, 0, 2, 2, 6];
const SIGNS: [f32; 15] = [
    1.0, -1.0, 1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0,
];

fn scale() -> Vec<f32> {
    (0..7).map(|v| 0.5 + 0.125 * v as f32).collect()
}

fn operator() -> SignedHyperIncidence {
    SignedHyperIncidence::new(7, 3, MEMBERS.to_vec(), SIGNS.to_vec(), scale()).expect("valid")
}

fn inputs(seed: u64, len: usize) -> Vec<f32> {
    let mut next = lcg(seed);
    (0..len).map(|_| next()).collect()
}

fn triangles() -> CellTopology {
    CellTopology::vertices(7)
        .with_rank(3, MEMBERS.to_vec(), SIGNS.to_vec())
        .expect("valid rank")
}

fn convolution_scales(topology: &CellTopology) -> RankScales {
    topology
        .scales(
            1,
            ScalePolicy::InvSqrtDegree,
            ScalePolicy::InvSqrtDegreeOverDegree,
        )
        .expect("rank 1")
}

/// Recorded 2026-09-24 00:32 against `f8904b9`, before the operator delegated to the rank
/// kernels: `node_to_edge`, `edge_to_node`, `star_matvec(0.25)`, `rhs`, `assemble_clique(0.25)`.
#[test]
fn the_operator_reproduces_its_recorded_bits() {
    let op = operator();
    let mut next = lcg(3);
    let x: Vec<f32> = (0..7).map(|_| next()).collect();
    let h: Vec<f32> = (0..5).map(|_| next()).collect();
    assert_eq!(fingerprint(&op.node_to_edge(&x)), 0xca52_880a_e04b_bdd2);
    assert_eq!(fingerprint(&op.edge_to_node(&h)), 0x15b3_89cd_294e_054c);
    assert_eq!(
        fingerprint(&op.star_matvec(&x, 0.25)),
        0xe86f_a820_2bb8_abc8
    );
    assert_eq!(fingerprint(&op.rhs(&h)), 0x15b3_89cd_294e_054c);
    assert_eq!(
        fingerprint(&op.assemble_clique(0.25)),
        0x276f_ba10_4091_bb19
    );
}

/// Frozen reference: Nagare's `nagare_common::ops::hg_message` kernels (working tree over
/// `nagare@1fc4201`, 2026-09-24 00:30), run on this fixture at `d = 3` with the scales of its
/// `degree_scales` and inputs `lcg(5)` drawn in the order `x, h, grad_x, grad_h`. The six
/// kernels are the ones the rank view replaces.
#[test]
fn the_rank_kernels_reproduce_the_recorded_message_kernels() {
    let topology = triangles();
    let scales = convolution_scales(&topology);
    assert_eq!(fingerprint(&scales.to_cell), 0xd8ba_4088_f91b_1732);
    assert_eq!(fingerprint(&scales.to_lower), 0x7d8a_99ae_bebc_3116);
    let view = topology.incidence(1, &scales).expect("rank 1");
    let d = 3;
    let all = inputs(5, (7 + 5 + 7 + 5) * d);
    let (x, rest) = all.split_at(7 * d);
    let (h, rest) = rest.split_at(5 * d);
    let (grad_x, grad_h) = rest.split_at(7 * d);
    assert_eq!(
        fingerprint(&fresh::lower_to_cell(view, x, d)),
        0xfbb4_853d_c21d_6443
    );
    assert_eq!(
        fingerprint(&fresh::lower_to_cell_adjoint(view, grad_h, d)),
        0x4825_4e09_a671_4ad4
    );
    assert_eq!(
        fingerprint(&fresh::cell_to_lower(view, h, d)),
        0x459b_d0c3_49d8_530a
    );
    assert_eq!(
        fingerprint(&fresh::cell_to_lower_adjoint(view, grad_x, d)),
        0x27ab_dd31_7615_5363
    );
    assert_eq!(
        fingerprint(&fresh::lower_to_cell_sign_gradient(view, x, grad_h, d)),
        0x2b3d_4da3_44b0_30ec
    );
    assert_eq!(
        fingerprint(&fresh::cell_to_lower_sign_gradient(view, h, grad_x, d)),
        0x2cab_7fb4_025d_de91
    );
}

/// Independent of any recording: each column of a width-`d` contraction is the scalar
/// contraction of that column.
#[test]
fn every_column_of_a_wide_contraction_is_the_scalar_contraction() {
    let topology = triangles();
    let scales = convolution_scales(&topology);
    let view = topology.incidence(1, &scales).expect("rank 1");
    let d = 4;
    let x = inputs(11, 7 * d);
    let h = inputs(12, 5 * d);
    let wide_up = fresh::lower_to_cell(view, &x, d);
    let wide_down = fresh::cell_to_lower(view, &h, d);
    for j in 0..d {
        let column = |field: &[f32]| field.iter().skip(j).step_by(d).copied().collect::<Vec<_>>();
        let bits = |v: Vec<f32>| v.iter().map(|f| f.to_bits()).collect::<Vec<_>>();
        assert_eq!(
            bits(column(&wide_up)),
            bits(fresh::lower_to_cell(view, &column(&x), 1))
        );
        assert_eq!(
            bits(column(&wide_down)),
            bits(fresh::cell_to_lower(view, &column(&h), 1))
        );
    }
}

fn inner(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(&x, &y)| f64::from(x) * f64::from(y))
        .sum()
}

#[test]
fn the_adjoints_are_adjoint() {
    let topology = triangles();
    let scales = convolution_scales(&topology);
    let view = topology.incidence(1, &scales).expect("rank 1");
    let d = 2;
    let (x, g_cell) = (inputs(21, 7 * d), inputs(22, 5 * d));
    let (h, g_lower) = (inputs(23, 5 * d), inputs(24, 7 * d));
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-5 * a.abs().max(b.abs()).max(1.0);
    assert!(close(
        inner(&fresh::lower_to_cell(view, &x, d), &g_cell),
        inner(&x, &fresh::lower_to_cell_adjoint(view, &g_cell, d))
    ));
    assert!(close(
        inner(&fresh::cell_to_lower(view, &h, d), &g_lower),
        inner(&h, &fresh::cell_to_lower_adjoint(view, &g_lower, d))
    ));
}

#[test]
fn the_sign_gradients_match_central_differences() {
    let topology = triangles();
    let scales = convolution_scales(&topology);
    let base = topology.incidence(1, &scales).expect("rank 1");
    let d = 2;
    let (x, g_cell) = (inputs(31, 7 * d), inputs(32, 5 * d));
    let (h, g_lower) = (inputs(33, 5 * d), inputs(34, 7 * d));
    let up_grad = fresh::lower_to_cell_sign_gradient(base, &x, &g_cell, d);
    let down_grad = fresh::cell_to_lower_sign_gradient(base, &h, &g_lower, d);
    let eps = 1e-2f32;
    for at in 0..MEMBERS.len() {
        let loss = |delta: f32, up: bool| {
            let mut signs = SIGNS.to_vec();
            signs[at] += delta;
            let view = base.with_signs(&signs).expect("same length");
            if up {
                inner(&fresh::lower_to_cell(view, &x, d), &g_cell)
            } else {
                inner(&fresh::cell_to_lower(view, &h, d), &g_lower)
            }
        };
        for (up, analytic) in [(true, up_grad[at]), (false, down_grad[at])] {
            let fd = (loss(eps, up) - loss(-eps, up)) / f64::from(2.0 * eps);
            assert!(
                (fd - f64::from(analytic)).abs() <= 1e-3,
                "incidence {at}, up={up}: fd {fd} vs {analytic}"
            );
        }
    }
}

#[test]
fn degrees_scales_and_cofaces_follow_their_definitions() {
    let topology = triangles();
    let degrees = topology.degrees(1).expect("rank 1");
    // Vertex 2 appears in triangles 0, 1, 4 and twice in 4.
    assert_eq!(degrees, vec![2.0, 2.0, 4.0, 2.0, 1.0, 2.0, 2.0]);
    let unit = ScalePolicy::Unit.apply(&degrees);
    assert!(unit.iter().all(|&s| s == 1.0));
    let sqrt = ScalePolicy::InvSqrtDegree.apply(&[0.0, 4.0]);
    assert_eq!(sqrt, vec![1.0, 0.5]);
    assert_eq!(
        ScalePolicy::InvSqrtDegreeOverDegree.apply(&[4.0]),
        vec![0.125]
    );

    let cofaces = topology.cofaces(1).expect("rank 1");
    assert_eq!(cofaces.n_lower(), 7);
    assert_eq!(cofaces.of(2), &[0, 1, 4, 4]);
    assert_eq!(cofaces.of(6), &[3, 4]);
    for (v, &degree) in degrees.iter().enumerate() {
        assert_eq!(cofaces.of(v).len() as f32, degree);
        assert!(cofaces.of(v).windows(2).all(|pair| pair[0] <= pair[1]));
    }
}

#[test]
fn a_second_rank_indexes_the_first() {
    // Two faces over the five triangles.
    let topology = triangles()
        .with_rank(2, vec![0, 1, 3, 4], vec![1.0, -1.0, 1.0, 1.0])
        .expect("valid second rank");
    assert_eq!(topology.max_rank(), 2);
    assert_eq!(topology.n_cells(2), Ok(2));
    assert_eq!(
        topology.degrees(2).expect("rank 2"),
        vec![1.0, 1.0, 0.0, 1.0, 1.0]
    );
    let refused = triangles().with_rank(2, vec![0, 5], vec![1.0, 1.0]);
    assert_eq!(
        refused,
        Err(CellTopologyError::MemberOutOfRange {
            rank: 2,
            at: 1,
            member: 5
        })
    );
}

#[test]
fn malformed_ranks_and_views_are_refused() {
    let base = || CellTopology::vertices(3);
    assert_eq!(
        base().with_rank(0, vec![], vec![]),
        Err(CellTopologyError::ZeroArity { rank: 1 })
    );
    assert_eq!(
        base().with_rank(2, vec![0, 1, 2], vec![1.0; 3]),
        Err(CellTopologyError::ShapeMismatch { rank: 1 })
    );
    assert_eq!(
        base().with_rank(2, vec![0, 1], vec![1.0]),
        Err(CellTopologyError::ShapeMismatch { rank: 1 })
    );
    assert_eq!(
        triangles().degrees(3),
        Err(CellTopologyError::NoSuchRank { rank: 3 })
    );
    let wrong_scales = RankScales {
        to_cell: vec![1.0; 6],
        to_lower: vec![1.0; 7],
    };
    assert!(matches!(
        triangles().incidence(1, &wrong_scales),
        Err(CellTopologyError::ShapeMismatch { rank: 1 })
    ));
    let one = [1.0f32; 3];
    assert!(RankIncidence::new(3, 3, &[0, 1, 3], &one, &one, &one).is_err());
    let view = RankIncidence::new(3, 3, &[0, 1, 2], &one, &one, &one).expect("valid");
    assert!(view.with_signs(&[1.0, 1.0]).is_err());
}
