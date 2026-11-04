//! The cell rank's sign storage and its parallel kernels.
//!
//! **Frozen oracle.** `oracle` below is the serial kernel set of `RankIncidence` as it stood at
//! `hymeko/graph` `411beec` (`hymeko_graph/src/cell_topology.rs`, the four contractions; SHA-256
//! prefix `4a4d71ead5c57827` of lines 400–489 there), rewritten over plain slices and otherwise
//! unchanged: same loops, same weight formula `(σ · s↑) · (1/k)`, same order. It is test-only, is
//! reachable from no production path, and is not developed further.

use hymeko_graph::cell_topology::{CellTopology, RankIncidence, RankScales, ScalePolicy};

mod cell_rank_common;
use cell_rank_common as fresh;

mod oracle {
    pub struct Rank<'a> {
        pub n_lower: usize,
        pub arity: usize,
        pub members: &'a [u32],
        pub signs: &'a [f32],
        pub to_cell: &'a [f32],
        pub to_lower: &'a [f32],
    }

    fn accumulate(out: &mut [f32], w: f32, x: &[f32]) {
        for (o, &xj) in out.iter_mut().zip(x) {
            *o += w * xj;
        }
    }

    impl Rank<'_> {
        fn n_cells(&self) -> usize {
            self.members.len() / self.arity
        }

        fn terms(&self) -> impl Iterator<Item = (usize, usize, f32)> + '_ {
            let inv_k = 1.0 / self.arity as f32;
            self.members
                .iter()
                .zip(self.signs)
                .enumerate()
                .map(move |(at, (&member, &sign))| {
                    let v = member as usize;
                    (at / self.arity, v, sign * self.to_cell[v] * inv_k)
                })
        }

        pub fn lower_to_cell(&self, x: &[f32], d: usize) -> Vec<f32> {
            let mut h = vec![0.0f32; self.n_cells() * d];
            for (c, v, w) in self.terms() {
                accumulate(&mut h[c * d..c * d + d], w, &x[v * d..v * d + d]);
            }
            h
        }

        pub fn lower_to_cell_adjoint(&self, grad_h: &[f32], d: usize) -> Vec<f32> {
            let mut grad_x = vec![0.0f32; self.n_lower * d];
            for (c, v, w) in self.terms() {
                accumulate(&mut grad_x[v * d..v * d + d], w, &grad_h[c * d..c * d + d]);
            }
            grad_x
        }

        pub fn cell_to_lower(&self, h: &[f32], d: usize) -> Vec<f32> {
            let mut y = vec![0.0f32; self.n_lower * d];
            for (at, (&member, &sign)) in self.members.iter().zip(self.signs).enumerate() {
                let (c, v) = (at / self.arity, member as usize);
                accumulate(&mut y[v * d..v * d + d], sign, &h[c * d..c * d + d]);
            }
            for (row, &scale) in y.chunks_mut(d.max(1)).zip(self.to_lower) {
                for value in row {
                    *value *= scale;
                }
            }
            y
        }

        pub fn cell_to_lower_adjoint(&self, grad_y: &[f32], d: usize) -> Vec<f32> {
            let mut grad_h = vec![0.0f32; self.n_cells() * d];
            for (at, (&member, &sign)) in self.members.iter().zip(self.signs).enumerate() {
                let (c, v) = (at / self.arity, member as usize);
                let w = sign * self.to_lower[v];
                accumulate(&mut grad_h[c * d..c * d + d], w, &grad_y[v * d..v * d + d]);
            }
            grad_h
        }
    }
}

fn lcg(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed;
    move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        state >> 33
    }
}

fn field(len: usize, seed: u64) -> Vec<f32> {
    let mut next = lcg(seed);
    (0..len)
        .map(|_| (next() % 2001) as f32 / 1000.0 - 1.0)
        .collect()
}

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}

/// `n_cells` cells of arity 5 over 97 lower cells: mixed signs, repeated members (the padding the
/// relation study has), and more cells than one parallel block holds.
fn fixture(n_cells: usize) -> (Vec<u32>, Vec<f32>) {
    let mut next = lcg(29);
    let members = (0..n_cells * 5)
        .map(|at| {
            if at % 7 == 3 {
                (at / 5 % 97) as u32
            } else {
                (next() % 97) as u32
            }
        })
        .collect();
    let signs = (0..n_cells * 5)
        .map(|_| if next().is_multiple_of(3) { -1.0 } else { 1.0 })
        .collect();
    (members, signs)
}

#[test]
fn a_rank_of_unit_signs_holds_one_bit_per_sign() {
    let (members, signs) = fixture(700);
    let topology = CellTopology::vertices(97)
        .with_rank(5, members.clone(), signs.clone())
        .expect("a valid rank");
    let rank = topology.rank(1).expect("rank 1");
    let held = rank.signs();
    assert!(held.is_unit());
    assert_eq!(held.as_values(), None);
    assert_eq!(held.len(), signs.len());
    assert_eq!(bits(&held.to_vec()), bits(&signs), "±1 round-trips exactly");
    assert_eq!(held.get(17), signs[17]);
    assert_eq!(held.stored_bytes(), signs.len().div_ceil(64) * 8);
    assert_eq!(
        rank.stored_bytes(),
        members.len() * 4 + signs.len().div_ceil(64) * 8
    );
}

#[test]
fn a_real_sign_keeps_the_rank_in_values() {
    let (members, mut signs) = fixture(40);
    signs[11] = 0.5;
    let topology = CellTopology::vertices(97)
        .with_rank(5, members, signs.clone())
        .expect("a valid rank");
    let held = topology.rank(1).expect("rank 1").signs();
    assert!(!held.is_unit());
    assert_eq!(held.as_values().map(bits), Some(bits(&signs)));
    assert_eq!(held.stored_bytes(), signs.len() * 4);
    // Neither zero nor a negative zero is a unit sign.
    for odd in [0.0f32, -0.0] {
        let (members, mut signs) = fixture(2);
        signs[0] = odd;
        let t = CellTopology::vertices(97)
            .with_rank(5, members, signs)
            .expect("valid");
        assert!(!t.rank(1).expect("rank 1").signs().is_unit(), "{odd:?}");
    }
}

#[test]
#[should_panic(expected = "sign 10 of 10")]
fn a_sign_past_the_table_is_refused() {
    let (members, signs) = fixture(2);
    let t = CellTopology::vertices(97)
        .with_rank(5, members, signs)
        .expect("valid");
    let _ = t.rank(1).expect("rank 1").signs().get(10);
}

/// **Every kernel, in both sign forms, is the frozen serial kernel bit for bit, on 1, 4 and 16
/// threads**: the bit table is exact for `±1`, and the parallel kernels sum each row in member
/// order.
#[test]
fn the_kernels_are_the_frozen_serial_kernels_in_both_forms_on_any_thread_count() {
    let n_cells = 1_000;
    let (members, signs) = fixture(n_cells);
    let topology = CellTopology::vertices(97)
        .with_rank(5, members.clone(), signs.clone())
        .expect("a valid rank");
    let scales: RankScales = topology
        .scales(
            1,
            ScalePolicy::InvSqrtDegree,
            ScalePolicy::InvSqrtDegreeOverDegree,
        )
        .expect("scales");
    let unit = topology.incidence(1, &scales).expect("a view");
    assert!(unit.signs().is_unit());
    let (to_cell, to_lower) = (scales.to_cell.clone(), scales.to_lower.clone());
    let values = RankIncidence::new(97, 5, &members, &signs, &to_cell, &to_lower).expect("valid");
    assert!(!values.signs().is_unit());
    let reference = oracle::Rank {
        n_lower: 97,
        arity: 5,
        members: &members,
        signs: &signs,
        to_cell: &to_cell,
        to_lower: &to_lower,
    };
    for d in [1usize, 3, 8] {
        let x = field(97 * d, 5 + d as u64);
        let h = field(n_cells * d, 7 + d as u64);
        let want = [
            reference.lower_to_cell(&x, d),
            reference.lower_to_cell_adjoint(&h, d),
            reference.cell_to_lower(&h, d),
            reference.cell_to_lower_adjoint(&x, d),
        ];
        for threads in [1, 4, 16] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .expect("a pool");
            for (form, view) in [("unit", unit), ("values", values)] {
                let got = pool.install(|| {
                    [
                        fresh::lower_to_cell(view, &x, d),
                        fresh::lower_to_cell_adjoint(view, &h, d),
                        fresh::cell_to_lower(view, &h, d),
                        fresh::cell_to_lower_adjoint(view, &x, d),
                    ]
                });
                for (kernel, (g, w)) in got.iter().zip(&want).enumerate() {
                    assert_eq!(
                        bits(g),
                        bits(w),
                        "{form}, kernel {kernel}, d {d}, {threads} threads"
                    );
                }
            }
        }
    }
}

/// A zero-width field is empty, not a panic, in the parallel kernels.
#[test]
fn a_zero_width_field_is_empty() {
    let (members, signs) = fixture(3);
    let topology = CellTopology::vertices(97)
        .with_rank(5, members, signs)
        .expect("valid");
    let scales = topology
        .scales(1, ScalePolicy::Unit, ScalePolicy::Unit)
        .expect("scales");
    let view = topology.incidence(1, &scales).expect("a view");
    assert!(fresh::lower_to_cell(view, &[], 0).is_empty());
    assert!(fresh::cell_to_lower_adjoint(view, &[], 0).is_empty());
}

/// A caller may reuse one buffer: the second call into a buffer the first one filled gives the
/// same bits as a fresh buffer, for every kernel.
#[test]
fn a_reused_buffer_gives_the_bits_of_a_fresh_one() {
    let (members, signs) = fixture(600);
    let topology = CellTopology::vertices(97)
        .with_rank(5, members, signs)
        .expect("a valid rank");
    let scales = topology
        .scales(
            1,
            ScalePolicy::InvSqrtDegree,
            ScalePolicy::InvSqrtDegreeOverDegree,
        )
        .expect("scales");
    let view = topology.incidence(1, &scales).expect("a view");
    let d = 3;
    let (x1, x2) = (field(97 * d, 11), field(97 * d, 12));
    let (h1, h2) = (field(600 * d, 13), field(600 * d, 14));
    let mut cells = vec![0.0f32; 600 * d];
    let mut lower = vec![0.0f32; 97 * d];
    let mut per_incidence = vec![0.0f32; 600 * 5];
    view.lower_to_cell(&x1, d, &mut cells);
    view.lower_to_cell(&x2, d, &mut cells);
    assert_eq!(bits(&cells), bits(&fresh::lower_to_cell(view, &x2, d)));
    view.cell_to_lower_adjoint(&x1, d, &mut cells);
    view.cell_to_lower_adjoint(&x2, d, &mut cells);
    assert_eq!(
        bits(&cells),
        bits(&fresh::cell_to_lower_adjoint(view, &x2, d))
    );
    view.cell_to_lower(&h1, d, &mut lower);
    view.cell_to_lower(&h2, d, &mut lower);
    assert_eq!(bits(&lower), bits(&fresh::cell_to_lower(view, &h2, d)));
    view.lower_to_cell_adjoint(&h1, d, &mut lower);
    view.lower_to_cell_adjoint(&h2, d, &mut lower);
    assert_eq!(
        bits(&lower),
        bits(&fresh::lower_to_cell_adjoint(view, &h2, d))
    );
    view.lower_to_cell_sign_gradient(&x1, &h1, d, &mut per_incidence);
    view.lower_to_cell_sign_gradient(&x2, &h2, d, &mut per_incidence);
    assert_eq!(
        bits(&per_incidence),
        bits(&fresh::lower_to_cell_sign_gradient(view, &x2, &h2, d))
    );
    view.cell_to_lower_sign_gradient(&h1, &x1, d, &mut per_incidence);
    view.cell_to_lower_sign_gradient(&h2, &x2, d, &mut per_incidence);
    assert_eq!(
        bits(&per_incidence),
        bits(&fresh::cell_to_lower_sign_gradient(view, &h2, &x2, d))
    );
}

/// An output of the wrong shape is refused, not written past or left partly stale.
#[test]
#[should_panic(expected = "out must be (n_cells, d)")]
fn an_output_of_the_wrong_shape_is_refused() {
    let (members, signs) = fixture(4);
    let topology = CellTopology::vertices(97)
        .with_rank(5, members, signs)
        .expect("valid");
    let scales = topology
        .scales(1, ScalePolicy::Unit, ScalePolicy::Unit)
        .expect("scales");
    let view = topology.incidence(1, &scales).expect("a view");
    let x = field(97 * 2, 1);
    let mut short = vec![0.0f32; 4 * 2 - 1];
    view.lower_to_cell(&x, 2, &mut short);
}

/// Arity 4 (so `1/k` is exact), members over 13 lower cells, signs `±1` and `±½`, scales in
/// `{½, 1, 2}`: every product and partial sum is exact in `f32` and in `f64`.
fn exact_rank() -> (Vec<u32>, Vec<f32>, Vec<f32>, Vec<f32>) {
    let mut next = lcg(41);
    let members = (0..40 * 4).map(|_| (next() % 13) as u32).collect();
    let signs = (0..40 * 4)
        .map(|_| [1.0, -1.0, 0.5, -0.5][(next() % 4) as usize])
        .collect();
    let to_cell = (0..13)
        .map(|_| [0.5, 1.0, 2.0][(next() % 3) as usize])
        .collect();
    let to_lower = (0..13)
        .map(|_| [0.5, 1.0, 2.0][(next() % 3) as usize])
        .collect();
    (members, signs, to_cell, to_lower)
}

fn widened(values: &[f32]) -> Vec<f64> {
    values.iter().map(|&v| f64::from(v)).collect()
}

/// The `f64` view is the `f32` view at another scalar: on data both represent exactly, all four
/// contractions agree value for value.
#[test]
fn the_f64_view_is_the_f32_view_on_exact_data() {
    let (members, signs, to_cell, to_lower) = exact_rank();
    let (signs64, cell64, lower64) = (widened(&signs), widened(&to_cell), widened(&to_lower));
    let single = RankIncidence::new(13, 4, &members, &signs, &to_cell, &to_lower).unwrap();
    let double: RankIncidence<'_, f64> =
        RankIncidence::new(13, 4, &members, &signs64, &cell64, &lower64).unwrap();
    let d = 3;
    let mut next = lcg(43);
    let x: Vec<f32> = (0..13 * d).map(|_| (next() % 9) as f32 - 4.0).collect();
    let h: Vec<f32> = (0..40 * d).map(|_| (next() % 9) as f32 - 4.0).collect();
    let (x64, h64) = (widened(&x), widened(&h));
    let check = |a: &[f32], b: &[f64]| {
        assert!(a.iter().zip(b).all(|(p, q)| f64::from(*p) == *q));
    };
    let (mut c32, mut c64) = (vec![0.0f32; 40 * d], vec![0.0f64; 40 * d]);
    single.lower_to_cell(&x, d, &mut c32);
    double.lower_to_cell(&x64, d, &mut c64);
    check(&c32, &c64);
    single.cell_to_lower_adjoint(&x, d, &mut c32);
    double.cell_to_lower_adjoint(&x64, d, &mut c64);
    check(&c32, &c64);
    let (mut l32, mut l64) = (vec![0.0f32; 13 * d], vec![0.0f64; 13 * d]);
    single.cell_to_lower(&h, d, &mut l32);
    double.cell_to_lower(&h64, d, &mut l64);
    check(&l32, &l64);
    single.lower_to_cell_adjoint(&h, d, &mut l32);
    double.lower_to_cell_adjoint(&h64, d, &mut l64);
    check(&l32, &l64);
}

/// In `f64` each contraction and its adjoint satisfy `⟨Bx, y⟩ = ⟨x, Bᵀy⟩` to rounding.
#[test]
fn the_f64_contractions_are_adjoint() {
    let (members, signs) = fixture(300);
    let signs64 = widened(&signs);
    let scales: Vec<f64> = widened(&field(97, 47)).iter().map(|v| 1.5 + v).collect();
    let view: RankIncidence<'_, f64> =
        RankIncidence::new(97, 5, &members, &signs64, &scales, &scales).unwrap();
    let d = 2;
    let x = widened(&field(97 * d, 53));
    let y = widened(&field(300 * d, 59));
    let inner = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(p, q)| p * q).sum::<f64>();
    let (mut bx, mut bty) = (vec![0.0; 300 * d], vec![0.0; 97 * d]);
    view.lower_to_cell(&x, d, &mut bx);
    view.lower_to_cell_adjoint(&y, d, &mut bty);
    assert!((inner(&bx, &y) - inner(&x, &bty)).abs() < 1e-12 * inner(&bx, &bx).sqrt().max(1.0));
    view.cell_to_lower_adjoint(&x, d, &mut bx);
    view.cell_to_lower(&y, d, &mut bty);
    assert!((inner(&bx, &y) - inner(&x, &bty)).abs() < 1e-12 * inner(&bx, &bx).sqrt().max(1.0));
}

/// The bit-packed `±1` reads as the same bits at either scalar.
#[test]
fn a_unit_sign_is_one_with_its_sign_bit() {
    use hymeko_graph::cell_topology::IncidenceScalar;
    assert_eq!(f32::unit_sign(0).to_bits(), 0x3f80_0000);
    assert_eq!(f32::unit_sign(1).to_bits(), 0xbf80_0000);
    assert_eq!(f64::unit_sign(0), 1.0);
    assert_eq!(f64::unit_sign(1), -1.0);
    assert_eq!(f64::from_count(5), 5.0);
}

/// A rank of `±1` signs is viewed at `f64` from its bits, no table converted, and contracts as the
/// `f32` view does on exact data; a rank of real signs has no such view.
#[test]
fn a_unit_rank_is_viewed_at_any_scalar() {
    let (members, signs) = fixture(40);
    let topology = CellTopology::vertices(97)
        .with_rank(5, members.clone(), signs.clone())
        .unwrap();
    let ones32 = vec![1.0f32; 97];
    let ones64 = vec![1.0f64; 97];
    let scales = RankScales {
        to_cell: ones32.clone(),
        to_lower: ones32,
    };
    let single = topology.incidence(1, &scales).unwrap();
    let double = topology
        .unit_incidence(1, &ones64, &ones64)
        .unwrap()
        .expect("unit signs");
    assert!(double.signs().is_unit());
    let mut next = lcg(61);
    let h: Vec<f32> = (0..40).map(|_| (next() % 9) as f32 - 4.0).collect();
    let (mut y32, mut y64) = (vec![0.0f32; 97], vec![0.0f64; 97]);
    single.cell_to_lower(&h, 1, &mut y32);
    double.cell_to_lower(&widened(&h), 1, &mut y64);
    assert!(y32.iter().zip(&y64).all(|(a, b)| f64::from(*a) == *b));
    let mut real = signs;
    real[0] = 0.5;
    let weighted = CellTopology::vertices(97)
        .with_rank(5, members, real)
        .unwrap();
    assert!(
        weighted
            .unit_incidence(1, &ones64, &ones64)
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        weighted.unit_incidence(1, &ones64[..3], &ones64),
        Err(hymeko_graph::cell_topology::CellTopologyError::ShapeMismatch { rank: 1 })
    ));
}

/// **A view holding its cofaces scatters by gathering, bit for bit** (plan
/// 2026-10-20-rank-incidence-scalar, Amendment 2): both scatters against the frozen serial kernels,
/// in both sign forms, at widths 1, 3 and 8, on 1, 4 and 16 threads, below and above the serial
/// floor. The fixture repeats members within a cell, and lower cell 97 has no coface.
#[test]
fn a_view_with_its_cofaces_scatters_as_the_frozen_serial_kernels() {
    for n_cells in [40, 8_000] {
        let (members, signs) = fixture(n_cells);
        let topology = CellTopology::vertices(98)
            .with_rank(5, members.clone(), signs.clone())
            .expect("a valid rank");
        let cofaces = topology.coface_positions(1).expect("rank 1");
        assert!(cofaces.of(97).is_empty());
        let repeats = |cell: &[u32]| (1..cell.len()).any(|i| cell[..i].contains(&cell[i]));
        assert!(
            members.chunks_exact(5).any(repeats),
            "a cell repeats a member"
        );
        let scales = topology
            .scales(
                1,
                ScalePolicy::InvSqrtDegree,
                ScalePolicy::InvSqrtDegreeOverDegree,
            )
            .expect("scales");
        let (to_cell, to_lower) = (scales.to_cell.clone(), scales.to_lower.clone());
        let unit = topology
            .incidence(1, &scales)
            .expect("a view")
            .with_cofaces(&cofaces)
            .expect("its own cofaces");
        let values = RankIncidence::new(98, 5, &members, &signs, &to_cell, &to_lower)
            .expect("valid")
            .with_cofaces(&cofaces)
            .expect("the same rank's cofaces");
        let reference = oracle::Rank {
            n_lower: 98,
            arity: 5,
            members: &members,
            signs: &signs,
            to_cell: &to_cell,
            to_lower: &to_lower,
        };
        for d in [1usize, 3, 8] {
            let h = field(n_cells * d, 7 + d as u64);
            let want = [
                reference.lower_to_cell_adjoint(&h, d),
                reference.cell_to_lower(&h, d),
            ];
            for threads in [1, 4, 16] {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .expect("a pool");
                for (form, view) in [("unit", unit), ("values", values)] {
                    let got = pool.install(|| {
                        [
                            fresh::lower_to_cell_adjoint(view, &h, d),
                            fresh::cell_to_lower(view, &h, d),
                        ]
                    });
                    for (kernel, (g, w)) in got.iter().zip(&want).enumerate() {
                        assert_eq!(
                            bits(g),
                            bits(w),
                            "{n_cells} cells, {form}, scatter {kernel}, d {d}, {threads} threads"
                        );
                    }
                }
            }
        }
    }
}

/// The gather is the serial scatter at `f64` too, on a unit rank viewed from its bits.
#[test]
fn a_unit_view_with_its_cofaces_scatters_as_the_serial_one_at_f64() {
    let (members, signs) = fixture(8_000);
    let topology = CellTopology::vertices(98)
        .with_rank(5, members, signs)
        .expect("a valid rank");
    let cofaces = topology.coface_positions(1).expect("rank 1");
    let scales: Vec<f64> = widened(&field(98, 67)).iter().map(|v| 1.5 + v).collect();
    let serial = topology
        .unit_incidence(1, &scales, &scales)
        .expect("rank 1")
        .expect("unit signs");
    let gather = serial.with_cofaces(&cofaces).expect("its own cofaces");
    let to_bits = |x: &[f64]| x.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
    for d in [1usize, 3] {
        let h = widened(&field(8_000 * d, 71 + d as u64));
        let (mut a, mut b) = (vec![0.0; 98 * d], vec![7.0; 98 * d]);
        serial.cell_to_lower(&h, d, &mut a);
        gather.cell_to_lower(&h, d, &mut b);
        assert_eq!(to_bits(&a), to_bits(&b), "cell_to_lower, d {d}");
        serial.lower_to_cell_adjoint(&h, d, &mut a);
        gather.lower_to_cell_adjoint(&h, d, &mut b);
        assert_eq!(to_bits(&a), to_bits(&b), "lower_to_cell_adjoint, d {d}");
    }
}

/// A reverse incidence of another rank is refused: another number of lower cells, another number of
/// incidences, or positions that name another lower cell.
#[test]
fn a_reverse_incidence_of_another_rank_is_refused() {
    let (members, signs) = fixture(40);
    let ones = vec![1.0f32; 98];
    let view = RankIncidence::new(98, 5, &members, &signs, &ones, &ones).expect("valid");
    let fewer_lower = CellTopology::vertices(97)
        .with_rank(5, members.clone(), signs.clone())
        .expect("valid");
    let other_count = CellTopology::vertices(98)
        .with_rank(5, members[..195].to_vec(), signs[..195].to_vec())
        .expect("valid");
    let mut shuffled = members.clone();
    shuffled.rotate_left(1);
    let other_members = CellTopology::vertices(98)
        .with_rank(5, shuffled, signs.clone())
        .expect("valid");
    for (what, other) in [
        ("lower cells", &fewer_lower),
        ("incidences", &other_count),
        ("members", &other_members),
    ] {
        let cofaces = other.coface_positions(1).expect("rank 1");
        assert!(view.with_cofaces(&cofaces).is_err(), "{what}");
    }
}

/// The reverse incidence by position lists, for each lower cell, the member-table positions that
/// name it, ascending; divided by the arity they are the reverse incidence by cell.
#[test]
fn the_positions_are_the_cofaces_times_the_arity_plus_the_slot() {
    let (members, signs) = fixture(300);
    let topology = CellTopology::vertices(98)
        .with_rank(5, members.clone(), signs)
        .expect("a valid rank");
    let (cells, positions) = (
        topology.cofaces(1).expect("rank 1"),
        topology.coface_positions(1).expect("rank 1"),
    );
    assert_eq!(positions.n_lower(), 98);
    assert_eq!(positions.parts().0, cells.parts().0, "the same rows");
    for v in 0..98 {
        let row = positions.of(v);
        assert!(
            row.windows(2).all(|pair| pair[0] < pair[1]),
            "row {v} ascends"
        );
        assert!(row.iter().all(|&at| members[at as usize] as usize == v));
        let divided: Vec<u32> = row.iter().map(|&at| at / 5).collect();
        assert_eq!(divided, cells.of(v), "row {v}");
    }
}
