# P1 — hymeko_pgraph lint gate (graph side)

**GOVERNING-STRATEGY:** ../core/docs/hymeko-platform-strategy.md · **STRATEGY-ID:**
HYMEKO-PLATFORM-2026-09-01-v1 · **ROADMAP-TASK:** P1. 2026-09-01 19:45 CEST.
Branch `feat/p1-lint-cleanup` @ base `origin/main = cbaadd4`. Behaviour-neutral lint cleanup only; graph
`main` is **not** promoted this phase. No algorithm, schema, wire, or dependency change.

## Authorisation
`hymeko_pgraph` is core-protected in this repo's `CORE.YAML` (the `core:` list). The P1 mission §5 is the
repository owner's explicit, detailed directive to make exactly this behaviour-neutral fix, with the
byte-identity acceptance criteria quoted below — that is the required approval. The change is gated on those
criteria: the emitted SQL ids stay byte-identical (proven), or the work halts. The core-repo token
`APPROVED-CORE-EDIT: benchmark-harness-consolidation` is scoped (mission §4) to the core owner module and is
**not** cited here; this graph edit rests on the §5 directive plus the machine-checked byte-identity gate.

## What changed
1. **`hymeko_pgraph/src/pgip_io.rs`** — the two `write_pgip` id-assignment loops (`next_id` for `materials`,
   `next_uid` for `units`) rewritten from a manual `let mut … = 1; … += 1;` counter to
   `for (next_id, d) in (1i64..).zip(&graph.materials)` / `(1i64..).zip(&graph.units)`. This clears
   `clippy::explicit_counter_loop` at `:368` / `:389` (the two lints P0 identified). The `params!` and
   `insert(*d, next_id)` lines are untouched; ids stay **1-based, contiguous, DeclId-ordered**. The
   `inputOutput` loop (`for u in &graph.units`) is deliberately not touched — it has no counter.
2. **`hymeko_pgraph/src/bin/hymeko_pgraph_dump.rs`** — dropped two unused imports
   (`analyze_lowered_with_regime_topk`, `analyze_source_with_regime_topk`). See "P0 undercount" below.
3. **`hymeko_pgraph/tests/pgip_io.rs`** — new regression test
   `materials_and_units_get_contiguous_1_based_ids`: writes HDA to `.pgip`, opens it, asserts
   `MIN(id)=1` and `MAX(id)=COUNT(*)` for `materials` and `units`. The existing round-trip tests are
   name-keyed (semantic), so a 0-based or non-contiguous id would round-trip and slip past them — this
   guards the exact invariant the rewrite must preserve.

## P0 undercount (surfaced, not hidden)
P0 reported the pre-existing lint set as **2** (`explicit_counter_loop` ×2). That was clippy's view *with the
lib failing*: `-D warnings` aborts at the lib's 2 errors and never compiles the `hymeko_pgraph_dump` bin, so
the bin's unused-import lint was **masked**. Fixing the lib lints unmasked it. The true pre-existing set is
**3** lints (2 counter-loop + 1 unused-import), all in `hymeko_pgraph`, all unchanged since `cbaadd4`
(verified: `git diff cbaadd4 -- …hymeko_pgraph_dump.rs` empty), all behaviour-neutral. All three are now
cleared — required to meet §5's "workspace clippy `-D warnings` must now be green" acceptance.

## Verification
- `cargo fmt -p hymeko_pgraph --check`: clean.
- `cargo clippy --workspace --all-features --all-targets -- -D warnings`: **green (exit 0)** — was 2 errors
  (then 1 unmasked) before.
- `cargo test --workspace --all-features --no-fail-fast`: **444 passed / 0 failed / 6 ignored** (+1 vs the
  P0-recorded 443 — the new id-invariant test).
- `cargo test -p hymeko_pgraph --features pgip --test pgip_io`: **6 passed** — the 5 pre-existing
  (read/round-trip/abb) + the new id-invariant, all green ⇒ SQL ids byte-identical after the rewrite.
- **Mutation (M7, no mutant committed):** `(1i64..)` → `(0i64..)` in the materials loop made
  `materials_and_units_get_contiguous_1_based_ids` **FAIL** (MIN(id)=0), proving the 1-based invariant is
  actually guarded. `pgip_io.rs` restored git-clean afterward.

## Dump / round-trip equivalence
`.pgip` round-trip (`roundtrip_chapter6_pgip_to_hymeko_lower_back`, `roundtrip_hda_hymeko_to_pgip_to_lower`)
green — semantic name/set/cost/incidence equality preserved. `write_pgip_bakes_abb_result_into_run_history`
green — the ABB optimum (cost 350, 2 units in structureId=1) and runHistory row are unchanged. Combined with
the id-invariant test, the emitted SQLite content is byte-equivalent for the id columns and semantically
identical for the round-trip.

## Provenance / HARD STOP
rustc 1.97.1, x86_64-pc-windows-msvc; pulls `hymeko_core` by git (== core `2ebb833`). Branch not promoted
to main. The two feature branches (core `feat/benchmark-harness-consolidation`, graph `feat/p1-lint-cleanup`)
await the next decision. **HARD STOP** — P2 not started.
