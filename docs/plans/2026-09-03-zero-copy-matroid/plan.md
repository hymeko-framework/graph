# Plan — P3.3 zero-copy matroid PoW + P3.2 measurement closure

**Created-at:** 2026-09-03 12:49 +02:00 · **ROADMAP-TASK:** P3.3 · **APPROVED-GRAPH-EDIT:**
zero-copy-matroid-operations-and-benchmarks · Repo `hymeko/graph`, branch
`feat/graph-matroid-navigation-p33` off P3.2 `1c6d568`. Not merged; no core/Python/Nagare/Hotaru/
production-PGraph edit. All task state under `D:\h\graph-matroid-p33` (never C:).

**Scope honesty (declared up front).** P3.3 bundles (A) closing the P3.2 measurement gaps and
(B) a zero-copy matroid PoW — together a multi-day study. This session delivers **Part B's
rank-2 graphic matroid**, its full V&V, the O(1)-facade proof, and fault injection — a
self-contained, fully-verifiable contribution. **PARTIAL** for: Part A (CPU-scaling subprocess
matrix + process-CPU-time, peak-RSS, bytes/entity, direct/scan/star/clique break-even, T/S-norm),
the incidence/Levi graphic matroid (§3.1), and the associated figures — enumerated in the report.

## §0 binding rule (respected)
Canonical `Ir` is the only semantic model; `IncidenceIndex` the only derived index. The matroid
is a **borrowed facade** `MatroidRef<'a,'ir> { ir, incidence }` (Copy, 2 words, O(1), zero-alloc,
no graph-sized loop, cannot outlive the snapshot) + caller-owned `GraphicMatroidWorkspace` (DSU
scratch: parent/rank_or_size/generation/scratch — NOT a graph model). No second IR, no copied
topology, no materialised Levi/star/clique in production, no serialization, no second caller ID
space. Complexity stated honestly: facade = O(1); rank/basis = Θ(ground set) (not O(1)); workspace
prep measured separately; basis output caller-owned.

## §1 audit (done)
P3.2 tip 1c6d568; graph main b234c487; one core/parser. No existing matroid/Kruskal/spanning
forest (`s_connect.rs` has a PRIVATE one-shot `Dsu`; `connected_components` is the differential
oracle giving C). The reusable matroid workspace is a distinct concern (append-only, generation
reset) — not a duplicate.

## §3.2 rank-2 graphic matroid (Part B, delivered)
Ground = `HyperArcId`s resolving to exactly two valid node incidences. DSU over the N nodes.
Operations: independence, rank (= N − components of the rank-2 subgraph), spanning forest,
min/max-weight basis (greedy = Kruskal), incremental append-only cycle check, fundamental cycle.
Typed `MatroidError`: WrongCardinality (multiway arc — never silently pairwise), DanglingTarget,
SelfLoop, NonFiniteWeight, UnsupportedTargetKind, SnapshotMismatch, Overflow (u32). Canonical IDs
out; output into caller buffers; no unsafe.

## §5 proofs
Axioms (empty independent, hereditary, exchange, basis-cardinality invariant, greedy=exhaustive)
on tiny instances vs an exhaustive oracle. Formula `basis = N − C` on Mycielski/map (connected →
N−1). Differential: DSU connectivity == `connected_components`; weighted basis == independent
Kruskal (total weight + selected canonical IDs + component count); incremental == full rebuild;
workspace reuse identical output; insertion-permutation invariant; duplicate incidences separate;
sign/weight payload + canonical hash unchanged before/after.

## §8 O(1) facade proof
Structural: `size_of::<MatroidRef>()` ≤ 2 pointers; constructor no loop/alloc/clone; creation does
not invoke basis prep; no topology-bearing field. Empirical sanity: flat creation time at
0/1e3/1e5/1e6 incidences (not over-interpreted).

## §10 fault injection (none committed; each caught by a named test; restored + hash-verified)
facade clones an array; wrong slot; dedup incidence; node/arc DSU domain overlap; rank-2 accepts a
3-way arc; weighted sort reversed; union omitted; reset omitted; raw-id-as-dense-offset; basis
claimed to preserve shortest paths.

## Gates / commits / stop
fmt · clippy `-D warnings` · check · build · tests · doctests. Commits: plan; (Part A closure if
reached); matroid + tests; bench + data; report + figures. Push only the feature branch; no main
merge. Rollback: branch is additive. plan.pdf unavailable (no TeX).
