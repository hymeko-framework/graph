# A zero-copy rank-2 graphic matroid over the canonical Ir

**STRATEGY-ID:** HYMEKO-PLATFORM-2026-09-01-v1 · **ROADMAP-TASK:** P3.3 · module
`hymeko_graph::matroid`. This document is CLI-independent design prose; every claim it makes is
backed by a named test (`hymeko_graph/tests/matroid_vv.rs`, `matroid_facade.rs`) or a measured
row (`reports/data/2026-09-03-p33-matroid-latency.tsv`).

## The invariant this preserves

The canonical `hymeko_core::ir::Ir` is the **only** semantic model, and the borrowed
`IncidenceIndex` its **only** permitted derived view. A matroid is therefore expressed as a *lens*
over those two objects — never as a fourth data structure. Concretely the module introduces:

- **No** second graph representation, copied array, or matroid adjacency;
- **No** materialised Levi / star / clique / 2-section on the production path;
- **No** serialized matroid and **no** second caller-facing ID space — inputs and outputs are the
  canonical `NodeId` / `HyperArcId`.

Everything the module computes is a function of `(&Ir, &IncidenceIndex)`; nothing is retained
except a caller-owned scratch buffer that holds union-find bookkeeping, never topology.

## The two objects

### `MatroidRef<'a, 'ir>` — the one-word, provenance-safe O(1) facade

```rust
#[derive(Clone, Copy)]
pub struct MatroidRef<'a, 'ir> { incidence: &'a IncidenceIndex<'ir> }
```

**Updated P3.4:** the facade holds **only** `&IncidenceIndex` and reaches the `Ir` through
`IncidenceIndex::ir()`. There is no separate `&Ir`, so an index can never be paired with a *foreign*
`Ir` — that invalid state is **unrepresentable** at the type level, not runtime-checked (no
`debug_assert`, no pointer compare, no `SnapshotMismatch`). One thin reference, hence exactly **one
machine word**, `Copy`, constructible with zero allocations regardless of graph size, cannot outlive
the borrowed snapshot, never serialized. Proven structurally (`size_of == size_of::<usize>()`, `Copy`,
a `compile_fail` doctest that the old two-argument `new(&ir, &idx)` no longer compiles) and empirically
(0 allocations to build the facade on both a 4-node graph and a 511-point Steiner system) in
`tests/matroid_facade.rs`.

The honest boundary: the **facade** is O(1); a rank / basis **computation** over it is Θ(ground
set) — it must touch every hyperarc. `rank_computation_is_not_o1_but_facade_is` documents exactly
that line so the O(1) claim is never over-extended to the computation.

### `GraphicMatroidWorkspace` — caller-owned DSU scratch

A reusable union-find (parent / rank / generation / scratch, all `Vec<u32>`), with path-halving
`find`, union by rank, and a **generation-stamped lazy reset**: each query bumps a generation
counter and lazily re-initialises a cell the first time it is touched, so a warm query does **not**
pay an O(N) clear. Reuse across queries is deterministic (`workspace_reuse_is_deterministic…`), and
the retained footprint is **12 bytes per node** (three `u32` per node), independent of the arc
count.

## The ground set: rank-2 hyperarcs only

The graphic matroid is defined on edges of a graph. In the hypergraph Ir, an "edge" is a hyperarc
that resolves to **exactly two** node incidences. The module enforces this at the boundary:

- a hyperarc with ≠ 2 node incidences → `MatroidError::WrongCardinality { node_incidences }`;
- a reference to a non-node declaration → `MatroidError::DanglingTarget { slot }`;
- a hyperarc whose two incidences are the same node → `MatroidError::SelfLoop`;
- a non-finite weight in a weighted basis → `MatroidError::NonFiniteWeight`;
- more nodes than fit a `u32` DSU coordinate → `MatroidError::Overflow`.

A multiway hyperarc is thus **never** silently truncated to its first two members — it is a typed
error the caller must handle. (`multiway_hyperarc_is_rejected`, `self_loop_is_rejected`,
`dangling_non_node_target_is_rejected`, `non_finite_weight_is_rejected`.)

Node identity is resolved through `decl_to_node` (the canonical DeclId → NodeId map), **not** by
treating a raw DeclId as a dense offset — which matters whenever node DeclIds are interleaved with
non-node declarations (`rank_correct_with_offset_node_decl_ids`).

## Operations

| operation | returns | complexity | oracle |
|---|---|---|---|
| `rank2_spanning_forest` | basis (`Vec<HyperArcId>`), size = rank | Θ(A·α) | independent DSU + `connected_components` |
| `rank2_rank` | rank = **N − C** | Θ(A·α) | `N − connected_components().count()` |
| `rank2_weighted_basis(minimize)` | min/max total weight + basis | Θ(A·log A) | independent Kruskal |
| `IncrementalRank2::offer` | independent? (append-only) | Θ(α) amortised | full rebuild |

The rank is `N − C` where `C` is the number of connected components over the node set (isolated
nodes included). The greedy algorithm on a weighted graphic matroid **is** Kruskal, so
`rank2_weighted_basis` returns the minimum- (or maximum-) weight spanning forest.

## What a rank-2 basis does and does not preserve

A spanning forest preserves **incidence-graph connectivity** — nothing more. It does **not**
preserve shortest-path cost, hyperedge cardinality, sign/weight as a propagation operator, BᵀB, or
message passing. It must never be substituted for the canonical hypergraph in a computation that
depends on any of those. This is a deliberate, documented semantic boundary, not an oversight.

## Verification strategy

Nothing in the matroid certifies itself. Every oracle in `matroid_vv.rs` is written from scratch in
the test file (a textbook recursive-find union-find, exhaustive subset enumeration, an independent
Kruskal) or is the engine's own `connected_components` — so the rank formula is triangulated three
independent ways, and the matroid's notion of independence is proven equal to graph acyclicity on
**every** subset of a small cyclic instance. The independence axioms (I1 empty-independent, I2
hereditary, I3 augmentation) are checked exhaustively there too.

Fault injection: nine source mutants — facade-owns-state, wrong-endpoint-slot, arc/node DSU-domain
overlap, cardinality-check defeated, greedy-sort reversed, union-omitted, reset-omitted,
raw-DeclId-as-offset, incremental-accepts-a-cycle — were each injected and shown to make a **named**
test fail, then reverted with the source hash verified back to the pristine baseline.

## The implicit incidence/Levi graphic matroid (added P3.4)

`matroid/levi.rs` adds a second matroid over the *same* objects: the graphic matroid of the implicit
bipartite incidence (Levi) graph. Ground elements are exact incidence coordinates `(HyperArcId,
slot)`; the implicit graph has one vertex per node and one per hyperarc, and one edge per incidence
coordinate. Nothing is materialised — the `Ir` is iterated in place and the **same**
`GraphicMatroidWorkspace` DSU is reused over the combined coordinate space (node `n` → `n`, hyperarc
`a` → `N + a`). The vertex universe is **all** N nodes and **all** A hyperarcs, so an isolated node
and a zero-incidence hyperarc are singleton components, and `rank = basis = (N + A) − C`. A non-node
or dangling target is a typed `DanglingTarget` (never silently skipped); duplicate/parallel
coordinates stay distinct ground elements (the second is dependent, not deduplicated). Operations:
`levi_rank`, `levi_components`, `levi_spanning_basis`, `levi_independent`, `IncrementalLevi`. Like the
rank-2 basis, a Levi basis is a **connectivity certificate only**. V&V (`tests/matroid_levi_vv.rs`)
reconciles it with the direct-Ir engine via `C_levi = C_node + Z` (Z = zero-incidence hyperarcs) and
validates STS(1023) (rank 175273) and random-1e6 (rank 532010); see
`reports/2026-09-03-matroid-navigation-p34.md`.
