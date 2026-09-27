//! The cell rank's sign storage and its parallel kernels.
//!
//! **Frozen oracle.** `oracle` below is the serial kernel set of `RankIncidence` as it stood at
//! `hymeko/graph` `411beec` (`hymeko_graph/src/cell_topology.rs`, the four contractions; SHA-256
//! prefix `4a4d71ead5c57827` of lines 400–489 there), rewritten over plain slices and otherwise
//! unchanged: same loops, same weight formula `(σ · s↑) · (1/k)`, same order. It is test-only, is
//! reachable from no production path, and is not developed further.

use hymeko_graph::cell_topology::{CellTopology, RankIncidence, RankScales, ScalePolicy};

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
                        view.lower_to_cell(&x, d),
                        view.lower_to_cell_adjoint(&h, d),
                        view.cell_to_lower(&h, d),
                        view.cell_to_lower_adjoint(&x, d),
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
    assert!(view.lower_to_cell(&[], 0).is_empty());
    assert!(view.cell_to_lower_adjoint(&[], 0).is_empty());
}
