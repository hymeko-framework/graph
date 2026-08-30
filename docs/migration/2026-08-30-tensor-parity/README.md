# Two-implementation parity: `graph/hymeko_graph` vs `tensor/hymeko_graph`

**Date:** 2026-08-30 · evidence for the consolidation that makes this repository canonical.

## Result

**7496 observations per side, bit-identical**, across the whole Nagare-reachable shared surface.

| area | why it is here |
|---|---|
| `spine` | 772-token delta, the largest; on Nagare's recorded FIR path |
| `cycle_enum` | the one path where **graph is newer** (2026-08-24 vs tensor 2026-08-16), so the switch hands Nagare a body it has never executed |
| `unsigned_cycles` CSR | `build_csr`, `neighbours`, `has_edge`, directed and undirected |
| CSR bitsets | `bs_words/get/set/clear` at n = 1, 63, 64, 65, 129 — the word boundary |
| `walks_unsigned` | two lengths × two seeds |
| `signed_graph` | `from_parts`, `n_edges`, `Sign` round-trip |

Comparison is `to_bits()` per scalar, printed per coordinate — not a digest. A digest says *that*
two runs differ; it never says where.

## Negative controls — each shown to fire

| mutation, applied to one side only | differing lines |
|---|---:|
| one ulp on a single FIR coefficient | 1314 |
| one edge changed in the signed-graph fixture | 20 |
| bitset probe moved 64 → 65 (word boundary) | 6 |

Without these the identical result would be indistinguishable from a blind harness.

## What this licenses, and what it does not

**Licenses:** keeping graph's bodies for every shared file. The 22 token deltas are formatting,
documentation, API extension and arithmetic-preserving refactor — not contract divergence. In
particular the `spine` delta is an *extension*; graph's shared arithmetic needs no porting.

**Does not license:** anything about `signed_tuples` or `hyper_operator`, which exist only on the
tensor side and therefore have no parity to check. They are pure migration and need their own
tests once moved.

## Two census errors corrected here

Both came from `grep`-shaped analysis trusted without being exercised.

1. **`bs_*` is not tensor-only.** The Phase-0 scan used `^\s*pub fn bs_`, which missed graph's
   one-line form `#[inline] pub fn bs_words(...)`. Both sides have the bitset API in *both*
   `traversal.rs` and `unsigned_cycles/csr.rs`, identically re-exported. Nothing to migrate and
   nothing to deduplicate.
2. **Ranking by line count pointed at the wrong file.** Line-level diffing put `topk_cycles.rs`
   first at 604 changed lines; at token level it is 191 and `spine.rs` leads at 772. Most of the
   604 was rustfmt re-wrapping.

## Why the harness is not a workspace member

`graph/CORE.YAML` declares `depends_on: []`. A crate here that links `tensor/hymeko_graph` would
create the very edge the consolidation exists to remove. Cargo also refuses two packages of the
same name in one lockfile, which is why the harness is two sibling crates emitting one text format
diffed offline rather than a single crate linking both.

It is committed as **source, not as a build target**: reproducible, and inert once
`tensor/hymeko_graph` is retired.

## Reproducing

```sh
cp -r harness /somewhere/outside/both/repos
cd /somewhere/outside/both/repos/harness
# side_g/Cargo.toml and side_t/Cargo.toml point at the two implementations by path
for s in g t; do (cd side_$s && cargo run -q --release > ../out_$s.txt); done
diff out_g.txt out_t.txt && echo IDENTICAL
```

---

## Appendix, 2026-08-30 07:40 — the formatting-only proof, corrected

The Phase 3D.1 commit (`a2a13cd`) justified "formatting only" with a **sorted token multiset**.
That is not sufficient, and the objection is correct: an arbitrary reordering of tokens preserves a
multiset. The claim needed a durable proof and now has one.

**Method.** For every path in `a2a13cd^..a2a13cd`, materialise the pre-format blob, run the crate's
actual rustfmt (`1.9.0-stable`, `--edition 2024`, no config file — defaults), and require **byte
identity** with the post-format blob.

**Result: 19/19 byte-identical. Affected-path sets equal, 19/19.**

| group | how it had to be run | result |
|---|---|---|
| `src/lib.rs` | in a reconstructed tree, so `pub mod` children resolve | exact |
| 15 `src/` files reachable from `lib.rs` | same in-tree pass | exact |
| 3 files under `examples/` and `tests/` | standalone — each is its own crate root and is not reachable from `lib.rs` | exact |

**Two harness errors on the way, both of which produced convincing-looking wrong answers:**

1. First run reported **0/19 exact**. `rustfmt` is not on `PATH` in this environment; every
   "output" was the shell's `command not found` message. Nineteen identical failures read as a
   catastrophic result rather than as a broken invocation. Fixed by resolving the binary through
   `rustup which rustfmt`.
2. Second attempt added `--skip-children` to keep `lib.rs` from recursing — and that flag does not
   exist in rustfmt 1.9. It made all 19 fail where 18 had passed. An unrecognised option is
   indistinguishable from a real regression if only the pass count is read.

The lesson is the same one this migration keeps re-learning: a check that reports failure must be
shown to be *running* before its failure is believed, exactly as a check that reports success must
be shown to be non-vacuous.
