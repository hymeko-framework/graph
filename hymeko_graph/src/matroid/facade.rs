//! The one-word, provenance-safe matroid facade.
//!
//! [`MatroidRef`] holds **only** `&IncidenceIndex`. The canonical `Ir` is obtained through
//! `IncidenceIndex::ir()`, so there is no way to pair an index with a *foreign* `Ir` — that invalid state
//! is unrepresentable at the type level, not merely rejected at runtime. This is the P3.4 correction of
//! the P3.3 two-field facade, which stored a separate `&Ir` a caller could mismatch.

use hymeko::ir::incidence::IncidenceIndex;
use hymeko::ir::ir::Ir;

/// A borrowed, `Copy`, **one-word** O(1) facade over the canonical objects: it holds only a reference to
/// the `IncidenceIndex`, and reaches the `Ir` through it. No separate `&Ir`, so a foreign `(Ir, index)`
/// pairing cannot be built. Cannot outlive the borrowed snapshot; never serialized.
///
/// The provenance-safe one-argument constructor:
/// ```
/// # use hymeko_graph::MatroidRef;
/// # use hymeko::ir::incidence::build_incidence_index;
/// # use hymeko::ir::ir::Ir;
/// let ir = Ir::default();
/// let index = build_incidence_index(&ir).unwrap();
/// let _m = MatroidRef::new(&index); // the Ir travels with the index — no way to mismatch it
/// ```
///
/// The old two-argument constructor no longer exists, so a separately supplied `Ir` cannot be paired
/// with a foreign index — this must not compile:
/// ```compile_fail
/// # use hymeko_graph::MatroidRef;
/// # use hymeko::ir::incidence::build_incidence_index;
/// # use hymeko::ir::ir::Ir;
/// let ir = Ir::default();
/// let index = build_incidence_index(&ir).unwrap();
/// let _m = MatroidRef::new(&ir, &index); // ERROR: new() takes one argument, not two
/// ```
#[derive(Clone, Copy)]
pub struct MatroidRef<'a, 'ir> {
    incidence: &'a IncidenceIndex<'ir>,
}

impl<'a, 'ir> MatroidRef<'a, 'ir> {
    /// Borrow the incidence index (and, through it, the canonical `Ir`). O(1), zero allocation, no loop,
    /// no clone. There is no separate `Ir` argument by construction.
    #[inline]
    #[must_use]
    pub fn new(incidence: &'a IncidenceIndex<'ir>) -> Self {
        Self { incidence }
    }

    /// The borrowed sparse incidence index (the sole permitted derived view).
    #[inline]
    #[must_use]
    pub fn incidence(&self) -> &'a IncidenceIndex<'ir> {
        self.incidence
    }

    /// The canonical `Ir`, reached through the index — never a separately stored reference. Internal to
    /// the matroid module (callers use [`MatroidRef::incidence`] and `IncidenceIndex::ir()` if needed).
    #[inline]
    pub(in crate::matroid) fn ir(&self) -> &'ir Ir {
        self.incidence.ir()
    }
}
