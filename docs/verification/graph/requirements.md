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
