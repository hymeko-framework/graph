# Consumer / ownership inventory — direct-Ir hypergraph algorithms
Created-at: 2026-09-01 01:58 CEDT

Read-only audit (Phase 1). Documentation is the index; code is the authority (§0.1).

## The canonical incidence substrate (core)
- `hymeko_core::ir::Ir` — THE canonical signed-incidence hypergraph. `pub arcs: Vec<ArcRec>`;
  `ArcRec.refs: Vec<SignedRefR>` is the sparse operator **B** row orientation; `SignedRefR::{target,sign,atom}`.
- `hymeko_core::ir::incidence::IncidenceIndex<'ir>` (promoted to core main 70b4f36 this session) — the
  reverse `(HyperArcId, ref_slot)` access permutation (Bᵀ). Borrows `&'ir Ir`. `incident_arcs(node)->&[HyperArcId]`,
  `incidences(node)->impl Iterator<IncidenceRef>`, `resolve(arc,slot)->IncidenceRef`, `ir()`, `degree`, `validate()`.
  Owner of canonical storage + incidence navigation. **This is what the direct-Ir algorithms consume.**

## Ownership table (asked-for capability -> owner), verified against code
| capability | intended owner (brief) | ACTUAL today | decision |
|---|---|---|---|
| canonical storage / incidence navigation | hymeko_core::Ir | hymeko_core::ir::{Ir, incidence} ✓ | consume as-is (zero core edit) |
| general hypergraph *topological* traversal on the canonical Ir | hymeko_graph | **absent** (see finding) | **BUILD HERE (this phase)** |
| P-graph semantics (producibility, MSG/SSG/ABB, axioms) | hymeko_pgraph | hymeko_pgraph ✓ | unchanged |
| HNN / message passing (numeric incidence operators) | Nagare | nagare::hnn (HyperGraphView numeric) ✓ | unchanged; do not write |
| domain-specific search | Hotaru | Hotaru owns a GENERAL suite (see finding) | do not write; mark reuse/migration |

## §0.1 FINDING — doc disagrees with code (reported, not silently followed)
The brief's ownership assumption is **"general hypergraph traversal -> hymeko_graph; domain-specific search -> Hotaru."**
The code disagrees:

1. **Hotaru already owns a mature GENERAL hypergraph suite**, not merely domain-specific search.
   `hotaru::hypergraph` (own signed bidirectional CSR `Hypergraph`, NOT the canonical Ir):
   - `berge::{shortest_berge_path, berge_distances, berge_components, enumerate_berge_paths,
     enumerate_berge_cycles, is_berge_acyclic}`
   - `s_connect::{SLine, s_components, s_distances, s_adjacent, max_s, s-betweenness, s-closeness}`
   - `construct::{levi, two_section, line_graph, dual}`
   - `cover::{transversals, matchings, edge covers}`, `acyclic::{GYO alpha-acyclicity, conformal, Helly}`
   - `signed::{directed_distances, directed_path, AlternatingPruner}`
   - 151 hypergraph tests; visited-node AND visited-edge discipline already present (berge.rs `edge_seen`;
     signed.rs `(edge, entry-sign)` triples). Reaches the canonical Ir ONLY via an optional, feature-gated
     (`hymeko-ir`, default off) **copy**: `Ir -> IrHypergraph::build_with -> own CSR Hypergraph` (hypergraph/ir.rs).
     Dependency-free by default.

2. **Nagare carries a second copy-based hypergraph model + a DORMANT topological traversal.**
   `nagare::hnn::traversal::HyperGraphView` is an OWNED twin-CSR built from Ir (dense-index coords). Its numeric
   side (gather/scatter/clique/HGNN) is the live HNN forward path. Its topological side — `BergeView` +
   `graph_traversal::{bfs, dfs_preorder, bfs_dense, dfs_preorder_dense}` — has **0 production call sites** and
   **1 test** (`berge_dfs_reaches_other_node`). Nagare depends on BOTH hymeko_core AND hymeko_graph (downstream).

## What is genuinely NOT duplicated (the non-overlapping primitive we build)
**Zero-copy topological traversal of the canonical `hymeko_core::ir::Ir` in place**, via `IncidenceIndex<'ir>`,
returning canonical `NodeId`/`HyperArcId`/`DeclId`. Neither Hotaru nor Nagare does this: both first COPY the Ir
into a private CSR model (Hotaru's `Hypergraph`, Nagare's `HyperGraphView`). The direct-Ir path:
- introduces NO second hypergraph model (borrows the ONE canonical Ir);
- sits at the correct layer — **upstream** of Nagare (which already depends on hymeko_graph), parallel to Hotaru;
- is the standard HyMeKo graph path the brief mandates.

The *algorithm shapes* (alternating node<->hyperedge BFS, visited-node+visited-edge) are shared textbook Berge
traversal — implemented independently here on the canonical operand, NOT copied from Hotaru's berge.rs.

## Migration / reuse items (NOT this night — Hotaru/Nagare are write-frozen this session)
- R1. Nagare's dormant `BergeView`/`graph_traversal` topological layer can be **subsumed/retired** in favour of the
  upstream hymeko_graph direct-Ir traversal (Nagare already depends on hymeko_graph). Correct-layer de-duplication.
- R2. Hotaru could gain a zero-copy `IncidenceIndex` backend (or the direct-Ir core could become a shared substrate)
  so its mature suite runs without the `Ir -> Hypergraph` copy. Cross-repo; needs its own approved edit.
- R3. `hymeko_pgraph::lowering` builds producers/consumers side tables by scanning `ir.arcs` directly; it could be
  re-expressed over `IncidenceIndex`. Non-urgent; pgraph reachability is domain-specific producibility, not generic.

## Materialization paths NOT on the standard path (kept as compat/standalone primitives, not retired this night)
- `hymeko_graph::signed_graph::SignedGraph` (pairwise `edges: Vec<(u32,u32)>`, dense u32 vertex ids) + `traversal::Csr`
  DFS/BFS — pairwise-only; a hypergraph reaches them via star-expansion. Compat/standalone; NOT the canonical path.
- `hymeko_graph::hyper_operator::SignedHyperIncidence` (uniform-arity, dense u32, f32 matvec/clique) — a NUMERIC star
  operator, subsumed on the canonical path by core `IncidenceIndex::{node_to_arc,arc_to_node,star_operator}` (ragged,
  canonical-id, lossless). Kept as a standalone f32 primitive (Nagare uses it); not a topological traversal.
- `hymeko_graph::incidence` (edge->cycle line-graph M_e) — unrelated object (KAN pipeline), not a hypergraph incidence.
