# Graph — verification requirements

**Established:** 2026-08-29 (graph core-pin phase) · Machine-readable status in `traceability.csv`.

This file scopes the `graph` workspace's V&V for the `hymeko_core` accessor-API
dependency pin and the read-only audit that prepares the later `meta_resolve.rs`
source migration. It follows the taxonomy the core repo established, so a reader
who knows core's requirements reads these the same way.

## Taxonomy

| Kind | Meaning | Counts toward satisfaction? |
|---|---|---|
| **NORMATIVE** | the system *shall* do this | yes, when its subject exists |
| **NOT_IMPLEMENTED** | an accepted requirement whose subject does not exist yet | no |
| **BLOCKED** | cannot even be expressed with what exists | no |
| **KNOWN_DEFECT** | a recorded defect | no — counted as defects |
| **FIXED** | a defect closed with a named regression | no — leaves the open count |
| **INFORMATIONAL** | a recorded fact, not a demand | no |

`NOT_IMPLEMENTED` and `BLOCKED` never count as satisfied.

## This phase (dependency-pin) — normative

`GRAPH-PIN-01 … 12` are all **PASS**. They assert the pin is exactly `15223bc`,
the manifests are byte-identical but for the one `hymeko_core` source line, no
unrelated dependency drifted, there is one core version, and every observable —
test/doctest counts, the P-graph dump oracle, coverage, `cargo tree -d`, package
count — is unchanged. A dependency pin that changed a model output, an ordering, a
hash or an error semantics would be rejected; none did.

## Next phase (source migration) — not implemented

`GRAPH-MIG-01 … 04` are **NOT_IMPLEMENTED**: this phase does not touch production
source. They are the scope of the future `meta_resolve.rs` migration — the eight
raw IR-storage reads, typed malformed-IR propagation, checked-composite base
resolution, and making `isa_reaches` iterative. The audit in
`reports/2026-08-29-graph-core-pin.md` is their design input.

## Known defects (found by the audit, not repaired here)

`GRAPH-DEFECT-01` — `bases_of` / `unit_arc_refs` **flatten** a missing node/edge
mapping to empty, so a malformed IR reads as "no bases" / "no incidence" rather
than an error, in a consumer that is otherwise strict.

`GRAPH-DEFECT-02` — the direct record indexes **panic** on a dangling record/arc
id instead of returning a typed fault.

Both are OPEN. They are the reason the migration is worth doing; they are recorded,
not fixed, because no source change is authorised in this phase.

## Informational

`GRAPH-INFO-01` — `cargo fmt --check` and `clippy -D warnings` fail at baseline in
`hymeko_graph` (and some `pgraph` formatting), pre-existing and identical before
and after the pin. `hymeko_graph` must not be modified this phase, so these are out
of scope and are not treated as regressions.

`GRAPH-INFO-02` — `hymeko_graph` has no `hymeko_core` dependency and no IR usage;
its `.0` / `.nodes[]` accesses are its own graph types. It is left unchanged.

`GRAPH-INFO-03` — no new core accessor is required for the migration; the
`15223bc` API (`decls`, `kind_of`, `decl_node`, `node_for_decl`, `edge_for_decl`,
`edge`, `arc`, `as_edge`) covers all eight sites.

## Rates

Denominator is normative requirements only; NOT_IMPLEMENTED and BLOCKED never
count as satisfied.

| Measure | Definition | Value |
|---|---|---|
| Evidence coverage | rows with cited evidence ÷ all rows | 20 / 20 = **100 %** |
| Implemented normative satisfaction | normative PASS ÷ normative rows whose subject exists | 12 / 12 = **100 %** |
| End-state normative satisfaction | normative PASS ÷ all normative rows | 12 / 16 = **75.0 %** |
| Open known defects | rows of kind KNOWN_DEFECT still OPEN | **2** |
| Fixed defects | KNOWN_DEFECT closed with a regression | **0** |

The end-state figure is deliberately below 100 %: `GRAPH-MIG-01…04` are the
un-built migration, and the two open defects are what it will close. A rising
end-state number without that migration would mean requirements were dropped, not
met.

---

## M2b-graph source migration — 2026-08-30 (`m2b-pgraph-meta-resolve-accessors`)

The `meta_resolve.rs` migration is done; the two open defects are closed.

- `GRAPH-MIG-01…04` move **NOT_IMPLEMENTED → PASS**: eight raw reads → 0, typed
  `IrReadError` (11 variants) replacing flatten/panic, checked-composite base
  resolution, iterative `isa_reaches`.
- `GRAPH-DEFECT-01` and `GRAPH-DEFECT-02` move **OPEN → FIXED**, each citing the
  `meta_resolve_ir_faults` regressions.
- New: `GRAPH-M2BS-01…03` (byte-identical valid output; source-first workload
  suite; <3 % perf with no new heap allocation) **PASS**; `GRAPH-M2BS-04`
  informational (order-canonical output); `GRAPH-M2BS-05` (F.17 dynamic
  source-pair oracle) **NOT_IMPLEMENTED**, deferred per the brief.

### Recomputed rates (normative only)

| Measure | dependency-pin | now |
|---|---|---|
| Implemented normative satisfaction | 12/12 = 100 % | **20/20 = 100 %** |
| End-state normative satisfaction | 12/16 = 75.0 % | **20/21 = 95.2 %** |
| Open known defects | 2 | **0** |
| Fixed defects | 0 | **2** |

The single remaining non-satisfied normative row is `GRAPH-M2BS-05` (F.17),
which the brief explicitly defers. `meta_resolve.rs` line coverage rose
92.20 % → 99.64 % (the one uncovered line justified in the report).

---

## Workload & V&V corrective — 2026-08-30 (`m2b-source-workload-vv-corrective`)

A test/workload-only phase. The M2b `meta_resolve` production migration is
unchanged (byte-identical to `a5ec3e7` except one `#[cfg(test)]` coverage module).

### Retracted / corrected claims from the previous report

The M2b-source report said "all families' small … lowered". That was true only of
the six families then implemented. The corrective, additively:

- **`GRAPH-M2BS-02` was over-broad** and is now backed by eleven parametric
  families, each its own normative row (`GRAPH-WL-*`). The old row stays PASS for
  what it covered; the family coverage is the new rows.
- **Audit findings, all verified true, then fixed:** `scale`/`seed` were unused
  (now `scale` is computed from incidence count and `seed` is semantic only on the
  stochastic families, encoded in the type); the oracle was common-mode with the
  edge list (now computed by formula and cross-checked); the size guard ran after
  allocation and used unchecked `usize` mul (now checked arithmetic before
  allocation, typed `ArithmeticOverflow`/`TooLarge`); the fingerprint used
  `DefaultHasher` (now FNV-1a-64 with a frozen vector); the previous "large" cases
  were a few thousand incidences (reclassified Small; real large cases at
  $10^5$–$10^6$ now run).
- **The 99.64 % file coverage was not 100 % touched-line.** It is now 100 %
  (282/282) via one `#[cfg(test)]` unit test for `bases_of`'s defensive arm.

### Taxonomy states

Rows now use `NORMATIVE` (status PASS/PARTIAL), `KNOWN_DEFECT` (FIXED),
`NOT_IMPLEMENTED`, `INFORMATIONAL` — the real state of each, never one blanket
PASS.

### Recomputed rates

| Measure | m2b-source | now |
|---|---|---|
| Evidence coverage (rows with cited evidence ÷ all rows) | 20/21 | **44/45 = 97.8 %** |
| Implemented normative satisfaction (normative PASS ÷ normative whose subject exists) | 20/20 = 100 % | **38/38 = 100 %** |
| End-state normative satisfaction (normative PASS ÷ all normative incl. the unbuilt F.17) | 20/21 = 95.2 % | **38/39 = 97.4 %** |
| Open known defects | 0 | **0** |
| Fixed defects | 2 | **2** |

Rows: 38 NORMATIVE (all PASS), 2 KNOWN_DEFECT (both FIXED), 4 INFORMATIONAL, 1
NOT_IMPLEMENTED (F.17, `GRAPH-M2BS-05`, deferred per the brief).

### Actually-run size classes (per family, this phase)

Every family: tiny + small on the default `cargo test` path. Medium: Steiner(m=6),
random-uniform(32000 inc), VLAN(8320 inc), humanoid, flock. Large (run once):
Steiner(m=9, 130305 inc), random-uniform(200000 inc), VLAN(262656 inc). Stress:
`#[ignore]` (Steiner m=11 ~2.1M inc, random-uniform 300000 edges) with the
documented command.

Test-support (generator) coverage is **UNMEASURED**: `cargo llvm-cov` instruments
workspace `src`, not `tests/`, so it does not stably report test-module lines; the
generator's correctness is instead pinned by the formula cross-check, the KAT
vectors and the five caught mutants.
