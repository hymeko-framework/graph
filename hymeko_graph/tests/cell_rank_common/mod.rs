//! The cell-rank kernels as values, for assertions. Each helper allocates a NaN-filled buffer,
//! lets the production kernel write into it, and returns it, so every assertion built on them
//! also checks that a kernel overwrites the buffer it is given.

// Shared by several test crates, each of which uses a subset.
#![allow(dead_code)]

use hymeko_graph::cell_topology::RankIncidence;

fn fresh(len: usize, kernel: impl FnOnce(&mut [f32])) -> Vec<f32> {
    let mut out = vec![f32::NAN; len];
    kernel(&mut out);
    out
}

pub fn lower_to_cell(view: RankIncidence<'_>, x: &[f32], d: usize) -> Vec<f32> {
    fresh(view.n_cells() * d, |out| view.lower_to_cell(x, d, out))
}

pub fn lower_to_cell_adjoint(view: RankIncidence<'_>, grad_h: &[f32], d: usize) -> Vec<f32> {
    fresh(view.n_lower() * d, |out| {
        view.lower_to_cell_adjoint(grad_h, d, out)
    })
}

pub fn cell_to_lower(view: RankIncidence<'_>, h: &[f32], d: usize) -> Vec<f32> {
    fresh(view.n_lower() * d, |out| view.cell_to_lower(h, d, out))
}

pub fn cell_to_lower_adjoint(view: RankIncidence<'_>, grad_y: &[f32], d: usize) -> Vec<f32> {
    fresh(view.n_cells() * d, |out| {
        view.cell_to_lower_adjoint(grad_y, d, out)
    })
}

pub fn lower_to_cell_sign_gradient(
    view: RankIncidence<'_>,
    x: &[f32],
    grad_h: &[f32],
    d: usize,
) -> Vec<f32> {
    fresh(view.members().len(), |out| {
        view.lower_to_cell_sign_gradient(x, grad_h, d, out);
    })
}

pub fn cell_to_lower_sign_gradient(
    view: RankIncidence<'_>,
    h: &[f32],
    grad_y: &[f32],
    d: usize,
) -> Vec<f32> {
    fresh(view.members().len(), |out| {
        view.cell_to_lower_sign_gradient(h, grad_y, d, out);
    })
}
