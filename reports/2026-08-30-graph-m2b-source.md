# M2b-graph: `hymeko_pgraph::meta_resolve` IR reads migrated to the accessor API

**Written:** 2026-08-30 01:38 CEST · **Authorisation:** `APPROVED-GRAPH-EDIT: m2b-pgraph-meta-resolve-accessors`
**Base HEAD:** `a125275d5c6f7e22ed97fc29ae45caeabf4783f5` · **Scope:** `hymeko_pgraph/src/{meta_resolve.rs,lib.rs}` + tests/examples/docs

Migrated every raw IR-storage read in the P-graph lowerer to the storage-%
independent accessor API of core `15223bc`, made malformed IR a **typed** error
instead of a silent flatten (`GRAPH-DEFECT-01`) or a panic (`GRAPH-DEFECT-02`),
and kept valid output byte-identical. Added a **source-first** workload suite
(F.11–F.16): every workload is real HyMeKo compiled through the parser + lowering
pipeline.

## Governance & preflight

`graph` has no `CLAUDE.md`; core's applies as the minimum. A plan bundle was on
disk before the first source edit (`docs/plans/2026-08-30-graph-m2b-source/`;
`plan.pdf` waived). HEAD `a125275`, clean, local == remote. Manifests held
byte-identical: `Cargo.toml` `095e27db…`, `Cargo.lock` `c2492761…`.

## 1. Production diff & the eight raw sites → 0

| File | ± |
|---|---|
| `hymeko_pgraph/src/meta_resolve.rs` | +318 −79 |
| `hymeko_pgraph/src/lib.rs` | +8 −9 (re-export `IrReadError`) |

A fresh `rg` re-inventory confirmed the eight sites (all in `meta_resolve.rs`),
then the migration:

| # | line (was) | raw read removed | now |
|---|---|---|---|
| 1 | 317 | `(0..ir.decl_nodes.len()).map(DeclId::new)` | `ir.decls()` |
| 2 | 321 | `ir.decl_nodes[d.0].kind` | `ir.kind_of` / `decls()` node |
| 3 | 325 | `ir.decl_nodes[d.0].name` | `ir.decl_node(d).name` |
| 4 | 331 | `as_node(d)+ir.nodes[n.0].bases` | `as_node`→`node`→back-link |
| 5 | 332 | `as_edge(d)+ir.edges[e.0].bases` | `as_edge`→`edge`→back-link |
| 6 | 373 | `ir.edges[eid.0].arcs` | `edge(eid)` |
| 7 | 376 | `ir.arcs[aid.0].refs` | `arc(aid)` + `in_edge` ownership |
| 8 | 405 | `&ir.decl_nodes[d.0]` (find_decl) | `ir.decls()` |

**After: `grep` for `ir.decl_nodes[` / `ir.nodes[` / `ir.edges[` / `ir.arcs[` /
`decl_nodes.len()` in `hymeko_pgraph/src` → 0.** `edge_cost` (an `Option`-based
read outside the eight) was also made fallible so a dangling edge declaration is
a typed error and a genuinely-absent cost stays `Ok(None)` (default 1.0).

## 2. Pre-change defect evidence (characterisation)

A temporary probe (not committed) built minimal malformed IRs and ran
`lower_resolved` against the **original** code, proving:

| fault | original behaviour |
|---|---|
| missing node mapping | **flattened** — material silently dropped (`GRAPH-DEFECT-01`) |
| mapped-missing node record | **panic** on `ir.nodes[999]` (`GRAPH-DEFECT-02`) |
| dangling arc id | **panic** on `ir.arcs[999]` (`GRAPH-DEFECT-02`) |
| dangling base target | **panic** via the direct kind index (`GRAPH-DEFECT-02`) |
| node back-link mismatch | **ignored** — wrong record read anyway (map-only) |

The permanent tests below assert the corrected typed behaviour; the red→green
transition is the difference between the two.

## 3. Error taxonomy

`IrReadError` (`Debug + Copy + Clone + Eq + PartialEq + Error`, **ids/`DeclKind`
only**, no program text), wrapped by `MetaResolveError::Ir(#[from] IrReadError)`
(so `?` propagates and `source()` exposes it), allocates nothing on the valid
path. Eleven distinguished variants: `DanglingDeclaration`, `MissingNodeMapping`,
`MissingEdgeMapping`, `MissingNodeRecord`, `MissingEdgeRecord`,
`NodeBacklinkMismatch`, `EdgeBacklinkMismatch`, `DanglingArcRecord`,
`ArcOwnershipMismatch` (`ArcRec::in_edge`), `DanglingBaseTarget`,
`DanglingIncidenceTarget`. No two collapsed into one `None`/`MalformedIr`.

The helper chain (`bases_of`, `isa_reaches`, `unit_arc_refs`, `unit_arc_targets`,
`edge_cost`, `decl_name`) is fallible; `classify_materials`/`classify_units`/
`build_incidence`/`find_decl`/`lower_resolved` thread `?`. `bases_of` and
`unit_arc_refs` use the checked-composite contract (mapping → record → back-link),
decomposed so each inconsistency is its own variant.

## 4. Semantic decisions (E)

- **Arc/ref order** is preserved (the walk `extend`s in `edge.arcs` then
  `arc.refs` order; no `sort`/`map`/`set` on that path). **Finding:** the final
  `LoweredPGraph` is *order-canonical* — reversing arc or ref order changes
  **no** dump hash and **no** test, because the incidence is a `set` keyed/sorted
  by `DeclId`. So order preservation is a fidelity property, not output-%
  observable (see §6, mutant 3).
- **`ArcRec::in_edge` ownership** is now a typed error
  (`ArcOwnershipMismatch`) when an edge lists an arc owned by another edge.
- **Inheritance cycle** stays cycle-safe and **not** an error — decision:
  a cyclic `<isa>` is a property of the (well-formed) reference graph, not a
  storage malformation, and the visited-set walk gives the same deterministic
  "reaches no archetype" result as before. `isa_reaches` was made **iterative**
  (was recursive) so the walk has no stack-depth limit; a permanent test resolves
  inheritance depth 64/65/128/1024 and another proves the cycle terminates.

## 5. Test matrix

`meta_resolve_ir_faults.rs` — **26 tests** (hand-built IR is permitted for
malformed negatives, F.16): every declaration/record/arc/base/incidence fault
asserting the *exact* variant; valid baseline; sign-direction and multi-arc/%
multi-ref order (non-symmetric fixtures); inheritance depth ladder; cycle
termination; the domain errors (`ConflictingRole`, `NonMaterialIo`, `NeutralRef`,
`AmbiguousArchetype`); scalar cost; no-`isa` program; non-`isa` base; every
`IrReadError` `Display` arm.

`workloads_suite.rs` + `workload/mod.rs` — the **source-first** suite (F.11–F.16).
Every workload is HyMeKo compiled through `compile_to_lowered` (golden files) or
`compile_sources` (generated). The generator maps a combinatorial structure to a
P-graph and emits the source *with* an independently-derived `ExpectedStructure`
oracle (materials/units/arcs/incidences/raws/products). Families: Fano S(2,3,7),
sunflower, Mycielski C5 & Grötzsch, spine/leaf network, robot chain. Coverage:

- generator sanity per family (lowers to its oracle);
- determinism;
- **golden byte-identity** (F.12): each tiny generated source equals a
  version-controlled `tests/data/workloads/*.hymeko` golden (a kept `#[ignore]`
  `regenerate_golden_fixtures` re-emits them);
- goldens lower through the **filesystem** path;
- **run policy** (F.15): all families' small, three families' medium, and a math
  + network + robot **large** all lowered on the default `cargo test` path; stress
  behind `#[ignore]`.

Test/doctest counts: **workspace 363 passed, 0 failed**; **2 doctests** (2
ignored). No `#[ignore]` test is reported as PASS.

## 6. Fault injection (G)

Four mutants, each caught by a named test, none committed; `src/meta_resolve.rs`
restored and verified against sha256 `093b87cb…`:

| mutant | caught by |
|---|---|
| drop node back-link check | `node_backlink_mismatch_is_typed` |
| missing arc → skip (empty) | `dangling_arc_record_is_typed` |
| drop arc-ownership check | `arc_ownership_mismatch_is_typed` |
| ignore dangling incidence target | `dangling_incidence_target_is_typed` |

**Order-swap mutant (the brief's #3), reported honestly:** reversing arc order
*and* reversing ref order each left all six dump hashes and every test unchanged
— the P-graph output is order-canonical (§4), so an order swap is not
output-observable and no output test can catch it. I therefore substituted the
arc-ownership mutant above and record the order-independence as a finding, rather
than fabricate a test that would "pass" on any implementation.

## 7. Coverage (H)

Same `cargo-llvm-cov`, same `--workspace` denominator; lcov repo-relative.

| scope | baseline | after |
|---|---|---|
| `meta_resolve.rs` | 92.20 % | **99.64 %** (273/274) |
| workspace production | 77.18 % | **77.42 %** |

Every `IrReadError` variant construction and `Display` arm is covered. One line is
uncovered and **justified, not excluded**:

- **`meta_resolve.rs:473`** — `None => Err(DanglingDeclaration { decl })` in
  `bases_of`. `bases_of` is a *total* helper, but its only caller (`isa_reaches`)
  pre-validates every id with `kind_of` before pushing it, and the walk's root is
  always a declaration from `ir.decls()`. So `kind_of(d)` is never `None` at this
  call site; the arm guards the helper's contract for a future caller that does
  not pre-check. Reaching it would require an artificial direct call that no real
  path makes.

## 8. Regression oracle & cardinalities (F.5)

The six dump hashes (methanol_synthesis, hda × MSG/SSG/ABB) are **byte-identical**
across the migration, and the workload suite asserts materials/units/raws/%
products/incidence cardinalities for every family so an accidentally-stable hash
harness cannot hide a change.

## 9. Performance (I / F.13) — and a regression I found and fixed

The `meta_resolve_bench` example (std-only, not in the public API, no new
dependency) measures two axes with source generation, parsing (for resident) and
hashing **outside** the timed region: **resident** (pre-parsed `Ir` → `meta_%
resolve`) and **end-to-end** (source → parse → lower → `meta_resolve`). Resident
is 11–270 µs; end-to-end 107–1414 µs, so parse/lower dominates and the resident
axis isolates `meta_resolve`.

**First A/B (old vs new, resident, 6 alternating rounds) showed +8.4 % geomean,
worst +12.9 % — over the 3 % gate** (A/A null ±2.5 %). Diagnosis: the iterative
`isa_reaches` allocated a `Vec` stack **per call** on top of the `BTreeSet` the
old recursion already allocated — a **new heap allocation on the valid path**,
which the brief forbids. Fix (smallest semantic-preserving): hoist both buffers
into a reusable `IsaWalk` scratch, allocated once per classification pass and
cleared per query. Re-measured:

| workload | old (µs) | new (µs) | Δ |
|---|---|---|---|
| grotzsch | 10.30 | 10.20 | −0.97 % |
| spine_leaf_medium | 80.20 | 78.90 | −1.62 % |
| robot_chain_medium | 261.00 | 260.20 | −0.31 % |
| sunflower_medium | 128.80 | 129.90 | +0.85 % |

**Geomean −0.52 %, worst +0.85 % — under 3 %, no new heap allocation on the valid
path** (the scratch is reused; the old code allocated a fresh set per call).

## Acceptance

| Criterion | Result |
|---|---|
| production raw IR-read count | **0** |
| every malformed case → exact typed error | ✅ 26 fault tests |
| valid HyMeKo → P-graph works | ✅ source-first suite, all families |
| 6/6 dump hashes byte-identical | ✅ |
| touched-line coverage | 99.64 %; the one gap justified per-line |
| reproducible > 3 % regression | none (−0.52 % geomean after the alloc fix) |
| manifest & lockfile byte-identical | ✅ `095e27db` / `c2492761` |
| `hymeko_graph`, core, parser untouched | ✅ |
| fault-injection mutants in tree | none (restored, hash-verified) |

## 10. Own errors, retracted assumptions, remaining gaps

- **Introduced a +8 % perf regression** (per-call `Vec` in the iterative walk),
  caught by the A/B, fixed by scratch reuse. Recorded, not hidden.
- **Order-swap is not output-observable** — my initial assumption that an
  arc/ref reversal would change the dump hash was wrong; the P-graph is a set.
  Substituted an ownership mutant and documented the finding.
- **Stray formatting** reflowed `abb.rs`/`cli.rs`/`dump.rs`; reverted to HEAD
  (byte-identical) — only `meta_resolve.rs` and `lib.rs` are changed in `src/`.
- **`meta_resolve.rs:473`** stays uncovered (defensive, justified above).
- **F.17 (dynamic before/after source pairs)** is **not** built here — the brief
  defers it ("ne alkalmazz mutációt a canonical IR-en"); the generator already
  supports emitting two specs, which is the hook for that future differential
  oracle.

## Next-phase entry conditions

1. This commit on graph `main` (local == remote verified).
2. A plan bundle before any further `meta_resolve.rs` edit.
3. The six dump hashes + the workload oracles are the regression fence.
4. F.17: `before.hymeko`/`after.hymeko` source pairs (generated from a base and a
   mutated `WorkloadSpec`) become the differential oracle for the transaction /
   dynamic-update work — **not** started here.

## HARD STOP

No next-repo dependency bump, no core stable-ID/storage migration, no transaction
API, no dynamic-update implementation, no MSG/SSG/ABB optimisation, no M2c/M2d/M1
work.

### Explicit answers

- **HyMeKo source loading works:** yes — every workload compiles through the real
  parser + `ModuleStore` + lowering + `meta_resolve` pipeline (golden filesystem
  path and generated in-memory path), across six families at small/medium/large.
- **P-graph function byte-identical:** yes — 6/6 dump hashes and all fixture
  cardinalities unchanged.
- **Tests added:** 26 fault tests + 14 workload-suite tests (12 default + 2
  ignored) + the bench probe; workspace 363 pass.
- **Coverage:** `meta_resolve.rs` 92.20 % → 99.64 %; workspace 77.18 % → 77.42 %.
- **Malformed states now typed:** dangling declaration, missing node/edge mapping,
  mapped-missing node/edge record, node/edge back-link mismatch, dangling arc,
  arc-ownership mismatch, dangling base target, dangling incidence target.
- **Performance gate met:** yes — resident A/B geomean −0.52 %, worst +0.85 %,
  under 3 %, with no new heap allocation on the valid path (after fixing an
  interim +8 % regression).
