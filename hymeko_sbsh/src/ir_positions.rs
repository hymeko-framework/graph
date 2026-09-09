//! Coordinates read **out of the IR itself**, with no side table.
//!
//! A HyMeKo declaration carries its attributes as child declarations, and a numeric list attribute
//! lands in the IR as `ValueR::List([Num, ...])`. So
//!
//! ```text
//!     a { position [1.0, 2.0, 0.5]; }
//! ```
//!
//! compiles to a `Node` declaration `a` with a child `position` whose `anno.value` is that list.
//! The coordinates are therefore *already in the model*, and a scene built over a model that
//! declares its own poses needs no external provider, no parallel array, and no synchronisation
//! between the two.
//!
//! [`IrPositions`] is that reading. It implements the same
//! [`hymeko_graph::direct_ir::PositionProvider`] seam the informed-search code already uses, so the
//! two sources are interchangeable: a model that declares poses uses this, and a caller whose poses
//! live outside the model (live simulation state, a tracker) supplies its own provider instead.
//! Neither is privileged and no third mechanism exists.
//!
//! # Which attribute
//!
//! The attribute name is a [`SymId`] the caller resolves from its own interner, because the IR
//! stores names interned and this crate has no interner of its own. There is deliberately no
//! default string: guessing `"position"` here would be this crate inventing vocabulary, which
//! belongs to the model, not to the graph layer.

use hymeko::common::ids::{DeclId, SymId};
use hymeko::ir::ir::{Ir, ValueR};
use hymeko_graph::direct_ir::PositionProvider;

/// Reads each declaration's coordinate from a named numeric-list attribute of the declaration
/// itself.
///
/// Coordinates are extracted once at construction — the `PositionProvider` contract hands back a
/// borrowed slice, so the values must outlive the call, and a numeric list in the IR is
/// `Vec<ValueR>` rather than a contiguous `[f64]`. That conversion is the one allocation here, and
/// it is `O(total coordinates)`, done once.
pub struct IrPositions {
    /// Ascending by `DeclId`, so lookup is a binary search rather than a map.
    entries: Vec<(DeclId, Vec<f64>)>,
}

impl IrPositions {
    /// Read every declaration that carries `attr` as a numeric list.
    ///
    /// A declaration without the attribute, or whose attribute is not an all-numeric list, is
    /// simply absent — it is not an error and not a zero coordinate. Absence means "not positioned",
    /// which is exactly what [`crate::SbshScene`] excludes from the partition.
    ///
    /// # Postconditions
    /// `entries` is sorted and duplicate-free by `DeclId`.
    pub fn from_attribute(ir: &Ir, attr: SymId) -> Self {
        let mut entries = Vec::new();
        for raw in 0..ir.decl_nodes.len() {
            let owner = DeclId::new(raw);
            if let Some(p) = read_child_vector(ir, owner, attr) {
                entries.push((owner, p));
            }
        }
        Self { entries }
    }

    /// How many declarations carry a position.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Is nothing positioned?
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The positioned declarations, ascending.
    pub fn decls(&self) -> impl Iterator<Item = DeclId> + '_ {
        self.entries.iter().map(|(d, _)| *d)
    }
}

impl PositionProvider for IrPositions {
    fn position(&self, node: DeclId) -> Option<&[f64]> {
        self.entries
            .binary_search_by_key(&node.raw(), |(d, _)| d.raw())
            .ok()
            .map(|i| self.entries[i].1.as_slice())
    }
}

/// The numeric vector of `owner`'s child declaration named `attr`, if it has one.
///
/// Walks `first_child` / `next_sibling` — the IR's own child list — and stops at the first match.
/// A list containing anything that is not a `Num` yields `None` rather than a partial coordinate:
/// a half-read position is worse than no position.
fn read_child_vector(ir: &Ir, owner: DeclId, attr: SymId) -> Option<Vec<f64>> {
    let mut child = ir.decl_nodes.get(owner.raw())?.first_child;
    while child.is_some() {
        let node = ir.decl_nodes.get(child.raw())?;
        if node.name == attr {
            return match node.anno.value.as_ref()? {
                ValueR::List(xs) => xs
                    .iter()
                    .map(|v| match v {
                        ValueR::Num(n) => Some(*n),
                        _ => None,
                    })
                    .collect::<Option<Vec<f64>>>()
                    .filter(|v| !v.is_empty()),
                ValueR::Num(n) => Some(vec![*n]),
                _ => None,
            };
        }
        child = node.next_sibling;
    }
    None
}
