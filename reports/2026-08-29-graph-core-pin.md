# graph → core dependency pin + pre-migration audit

**Written:** 2026-08-29 21:01 CEST · **Authorisation:** `APPROVED-DEPENDENCY-UPDATE: graph-core-pin-to-15223bc`
**Base HEAD:** `ac341e0ae8a988b64437a29af6ccb5c93641cbc5` · **Scope:** `Cargo.lock` + docs/report; **no production source**

Dependency-only bump of the `hymeko_core` git revision the `graph` workspace
resolves (`3bdebf89` → `15223bc`), plus a read-only inventory of the direct
IR-storage accesses in `hymeko_pgraph` that a later phase will migrate. **The
graph source is not migrated here.**

## Governance & preflight

`graph` has no `CLAUDE.md`/`CORE.YAML`/`tools.yaml`; core's `CLAUDE.md` rules
apply as the minimum. A plan bundle was written **before** the lockfile edit at
`docs/plans/2026-08-29-graph-core-pin/` (`plan.tex`/`plan.tikz`/`plan.mmd`;
`plan.pdf` waived under the standing no-LaTeX waiver).

| Check | Result |
|---|---|
| HEAD == expected `ac341e0` | yes |
| local == origin/main | yes (`ls-remote` via IP rewrite + host-key alias, read-only) |
| worktree clean | yes |
| workspace members | `hymeko_graph`, `hymeko_pgraph` |
| depends on core | `hymeko_pgraph` only |
| old core pin | `3bdebf89…` |
| parser pin | `e28a2051…` |
| `hymeko_core` lock entries | 1 |
| `Cargo.toml` sha256 | `095e27db…` |
| `Cargo.lock` sha256 (before) | `ff8e934f…` |

Read core M2a (accessor API), M2a.1 (accessors inline to the same code as raw
indexing), M2b (total-read vs strict-consumer split).

## B. Baseline at the old pin (`3bdebf89`)

| Gate | Result |
|---|---|
| `cargo fmt --check` | **fails** — pre-existing (see Informational) |
| `cargo clippy --workspace --all-features --all-targets -D warnings` | **fails** — 10 pre-existing errors in `hymeko_graph` |
| `cargo check --workspace --all-features` | pass |
| `cargo build --workspace --all-features` | pass |
| `cargo test --workspace --all-features --no-fail-fast` | **325 passed, 0 failed, 2 ignored** |
| doctests | **2 passed, 2 ignored** (`hymeko_graph` 0+1, `hymeko_pgraph` 2+1) |

**Per-crate real tests** — `hymeko_graph`: 109 lib + 58 integration = 167;
`hymeko_pgraph`: 45 lib + 111 integration = 156; total 323 + 2 in trailing
targets = 325.

`cargo tree -d`: duplicates are all third-party (`aho-corasick`, `either`,
`hashbrown`, `lalrpop-util`, `regex-automata`); **one `hymeko_core`** at
`3bdebf89`. 105 unique packages.

### Regression oracle

The `hymeko_pgraph_dump` binary produces deterministic P-graph structure output,
exercising the full pipeline including `meta_resolve`. Baseline hashes over two
committed fixtures × three algorithms:

| fixture | msg | ssg | abb |
|---|---|---|---|
| `methanol_synthesis` | `108b530d…` | `4ae54381…` | `5343cee4…` |
| `hda` | `5561967b…` | `bbcc5d1c…` | `517a2442…` |

This is a real, byte-exact oracle — not a fabricated performance claim.

## C. Coverage baseline

`cargo-llvm-cov 0.9.0` (present), lcov normalized to repo-relative paths.

| Scope | baseline |
|---|---|
| workspace production lines | 7730/10016 = **77.18 %** |
| `hymeko_pgraph/src` | 2844/3824 = 74.37 % |
| `hymeko_graph/src` | 4886/6192 = 78.91 % |
| `meta_resolve.rs` | 201/218 = **92.20 %** |

All eight audit sites are already covered (hits 45–1037), so the future
migration's touched lines are exercised today. Uncovered `meta_resolve.rs` lines
(241–243, 290–293, 303–305, 333, 350, 357, 363, 371, 388, 408) are error/edge
branches, pre-existing. No source was changed for coverage.

## D. Dependency bump

```bash
cargo update -p hymeko_core --precise 15223bcd762806b69909ea73f7526586c2d2ded6
```

As in the sibling network pin, this also moved `parser` (`e28a2051 → 284c490d`).
That unrelated drift was **not accepted**:

```bash
cargo update -p parser --precise e28a205147c48273980e63aa3338ca286974a6ed
```

### Lockfile audit

The **entire** `Cargo.lock` diff is one line:

```diff
-source = "git+ssh://…/core.git?branch=main#3bdebf89fbb864e0e0b109f43d8e5f7e9c108726"
+source = "git+ssh://…/core.git?branch=main#15223bcd762806b69909ea73f7526586c2d2ded6"
```

- changed lines: the `hymeko_core` `source` line only;
- checksum lines: none — git deps carry no `checksum` field;
- `parser` / others: unchanged;
- one `hymeko_core` entry; `cargo tree -d` byte-identical but for the rev string;
- 105 packages before and after; `Cargo.toml` byte-identical; no `src`/`tests`/
  `examples`/fixture change.

A `cargo clean -p hymeko_core` + rebuild confirmed the new revision actually
compiles (`Compiling hymeko_core …#15223bcd`), not a stale artifact.

## E. Regression at the new pin (`15223bc`)

| Gate | old pin | new pin |
|---|---|---|
| check / build | pass | pass |
| tests | 325 / 2 ign | **325 / 2 ign** |
| doctests | 2 / 2 ign | **2 / 2 ign** |
| clippy | 10 errs (hymeko_graph) | **identical** (signature diff empty) |
| fmt | fails (pre-existing) | **identical** (source untouched) |
| `cargo tree -d` | — | identical but for rev string |
| packages | 105 | **105** |
| workspace coverage | 77.18 % | **77.18 %** |
| `meta_resolve.rs` coverage | 92.20 % | **92.20 %** |

**Regression oracle: all 6 hashes MATCH** — the P-graph dump output is byte-%
identical across the bump. No model output, ordering, hash or error semantics
changed, as required.

No performance benchmark: production code is unchanged, so there is nothing to
attribute a timing to. No improvement is claimed from a pin.

## F. Pre-migration audit (read-only)

A full `rg` sweep over `hymeko_pgraph/src` for `decl_nodes`, `.nodes[]`/`.edges[]`/
`.arcs[]`, `decl_to_*`, `.0`, `raw()`, `Id::new`. Every `.0` was classified
individually. The many `decl_to_name` hits across `builder`/`cli`/`dump`/
`lowering`/`pgip_io`/`regime` are the **pgraph's own** `BTreeMap<DeclId, String>`
field, **not** `ir.decl_to_*` — not IR reads. Line 286's `.0` is the float `1.0`.

**Every real IR-storage read is in `meta_resolve.rs`.** The old "7 site" estimate
undercounts by one: there are **8**.

| # | line | current read | contract | strict/tolerant | needed accessor | error model | mapping-only vs checked | order | hot-path | needed test | migration risk |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 317 | `(0..ir.decl_nodes.len()).map(DeclId::new)` | dense id range | bound only | `ir.decls()` | none | n/a | ascending, semantically neutral | one scan per resolve | dense-iter equivalence | low |
| 2 | 321 | `ir.decl_nodes[d.0].kind` | kind lookup | **panics** on bad id | `kind_of(d) -> Option` | dangling decl → typed | mapping-only | n/a | per-decl, hot in scans | kind-of-missing | low |
| 3 | 325 | `ir.decl_nodes[d.0].name` | name lookup | **panics** on bad id | `decl_node(d).map(\|n\| n.name)` | dangling decl → typed | mapping-only | n/a | per-decl | name-of-missing | low |
| 4 | 331 | `as_node(d).map(\|n\| ir.nodes[n.0].bases)` | node bases | **tolerant** (`unwrap_or &[]`) + panics on `n.0` | `node_for_decl(d)` | missing-mapping / missing-record / backlink → typed | **checked-composite** | bases order preserved | per-decl in `isa_reaches` | node-bases malformed | medium |
| 5 | 332 | `as_edge(d).map(\|e\| ir.edges[e.0].bases)` | edge bases | tolerant + panics | `edge_for_decl(d)` | same | checked-composite | bases order | per-decl | edge-bases malformed | medium |
| 6 | 373 | `ir.edges[eid.0].arcs` (after `as_edge`) | edge arcs | `as_edge` None→empty; **panics** on `eid.0` | `edge_for_decl` / `edge(eid)` | missing-edge / dangling-arc → typed | checked-composite | **arcs order semantic** | per-unit | edge-record malformed | medium |
| 7 | 376 | `ir.arcs[aid.0].refs` | arc refs | **panics** on dangling `aid` | `arc(aid)` | dangling-arc → typed | mapping-only (arc has no back-link) | **refs order semantic** | per-arc, incidence build | dangling-arc | medium-high |
| 8 | 405 | `&ir.decl_nodes[d.0]` (in `find_decl` scan) | decl node | **panics** on bad id | `ir.decls()` / `decl_node` | dangling decl → typed | mapping-only | scan order | archetype scan | dense-iter equivalence | low |

### The seven questions the brief asked

1. **Strict, or legitimate partial IR?** `meta_resolve` is a **strict consumer**:
   its public entries `compile_to_lowered` / `lower_resolved` return
   `Result<LoweredPGraph, MetaResolveError>`, and it already raises typed errors
   for domain faults (`ConflictingRole`, `NonMaterialIo`, `NeutralRef`,
   `MissingArchetype`, `AmbiguousArchetype`). It has no legitimate partial-IR
   input; a compiled program is expected well-formed. The only legitimate absence
   is a declaration with **no bases** (an archetype root) — which must stay an
   empty result, not an error.
2. **Backlink coherence at declaration→record?** **Yes.** `bases_of` reads the
   record's `bases`; a record belonging to another declaration would give wrong
   bases and mis-classify a material's role. `node_for_decl`/`edge_for_decl`
   (checked-composite) are the right tools, as in the network migration.
3. **Refs traversal order semantic?** **Yes.** `unit_arc_refs` yields arcs in
   `edge.arcs` order, then refs in `arc.refs` order; this is the material
   incidence order that defines P-graph unit structure. The migration must
   preserve it exactly (the oracle would catch a reorder).
4. **Depth limit / recursion / silent flattening?** `isa_reaches` (the `<isa>`
   ancestry test) is **recursive** with a `BTreeSet` visited set — cycle-safe, but
   stack-depth-bounded by inheritance depth (no fixed cap like the network's 64,
   and no infinite-loop risk). `bases_of` and `unit_arc_refs` **silently flatten**
   a missing mapping to empty. No fixed numeric depth cap exists.
5. **Which faults are flattened or panicked today?** *Flattened to empty:* a Node/
   Edge declaration with a missing record mapping (`bases_of` `unwrap_or &[]`), and
   an edge with a missing mapping (`unit_arc_refs` `as_edge` None → empty).
   *Panicked:* a bad `DeclId` in `decl_kind`/`decl_name`/`find_decl`, and a
   dangling record/arc id in `ir.edges[eid.0]` / `ir.arcs[aid.0]`. This split is
   `GRAPH-DEFECT-01` / `-02`.
6. **New core accessor needed?** **No.** `decls`, `kind_of`, `decl_node`,
   `node_for_decl`, `edge_for_decl`, `edge`, `arc`, `as_edge` in `15223bc` cover
   all eight sites. Documented, not implemented.
7. **Is `hymeko_graph` independent of core IR?** **Yes.** It has no `hymeko_core`
   dependency and no `use hymeko`; its `.0` / `.nodes[]` accesses are its own graph
   types. It is left unchanged.

## Acceptance

| Criterion | Result |
|---|---|
| lockfile diff = only core revision | ✅ one line |
| gates green or baseline failure identical | ✅ check/build/tests/doctests green; fmt+clippy identical pre-existing |
| no production source changed | ✅ only `Cargo.lock` tracked |
| regression oracle byte-identical | ✅ 6/6 MATCH |
| no unrelated dependency drift | ✅ parser reverted |
| one `hymeko_core` version | ✅ |
| `Cargo.toml`/`Cargo.lock` core-only | ✅ toml byte-identical, lock one line |

## Files touched

| File | change |
|---|---|
| `Cargo.lock` | 1 line — `hymeko_core` revision |
| `docs/plans/2026-08-29-graph-core-pin/{plan.tex,tikz,mmd}` | new plan bundle |
| `docs/verification/graph/{requirements.md,traceability.csv}` | new V&V |
| `reports/2026-08-29-graph-core-pin.md` | this report |
| `reports/coverage/graph-core-pin-{baseline,after}.lcov` | coverage artifacts (repo-relative) |

`Cargo.toml`, all `src/`/`tests/`/`examples/`/`benchmarks/`/`data/` — unchanged.

## Worktree & preserved untracked files

Before commit, the only tracked change is `Cargo.lock`; the rest are new
`docs/`/`reports/` artifacts. No pre-existing untracked file was touched (the
repo had a clean worktree at entry).

## Own errors, retracted assumptions, unproven claims

- The "7 site" estimate in the brief was **wrong by one** — the actual inventory
  is **8** raw reads (node and edge bases are two sites, plus the two dense-scan
  indexes at 317/405 and the three record indexes). Recorded rather than glossed.
- `cargo metadata`'s raw `"name":` count (2399) is **not** a package count — it
  counts every name field; the meaningful figure is 105 unique packages via
  `cargo tree`. I do not report 2399 as a dependency count.
- No performance claim is made: production code did not change, so any timing
  would be diagnostic noise, and I did not measure one.
- The fmt/clippy baseline failures are in `hymeko_graph`, which I must not modify;
  I did not attempt to "fix" them and they are identical before and after.

## Entry conditions for the next graph source migration

1. This dependency commit on graph `main` (local == remote verified).
2. A plan bundle on disk **before** the first `meta_resolve.rs` edit.
3. The contract decision recorded per site (table above): checked-composite for
   the base/record reads (2, 4, 5, 6), mapping-only for the arc/decl reads (7, 8),
   `decls()` for the dense scans (1, 8).
4. A typed IR-read error model (dangling declaration, missing node/edge mapping,
   mapped record missing, backlink mismatch, dangling arc) replacing the current
   flatten-or-panic split (`GRAPH-DEFECT-01`/`-02`).
5. The 6 dump-oracle hashes are the regression oracle; a migration that changes
   them must justify it against a stated requirement.
6. `isa_reaches` may be made iterative (lower priority — already cycle-safe).
7. `hymeko_graph` stays untouched; no new core accessor is added.

## HARD STOP

The graph source migration is **not** done — this is the dependency-pin and
audit phase only. No `meta_resolve.rs` edit, no new core accessor, no next-repo
bump.
