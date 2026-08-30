# Graph workload & V&V corrective — source-first parametric workloads

**Written:** 2026-08-30 02:20 CEST · **Authorisation:** `APPROVED-GRAPH-EDIT: m2b-source-workload-vv-corrective`
**Base HEAD:** `a5ec3e7c062aa43e004a48454eb3e2f2c55ceea0` · **Scope:** tests / workloads / benchmark / docs only

A test- and workload-correction phase. The working M2b `meta_resolve` production
migration is **not** re-designed: `meta_resolve.rs` is byte-identical to `a5ec3e7`
except one `#[cfg(test)]` coverage module (+29 lines); `lib.rs`, `Cargo.toml`,
`Cargo.lock`, `hymeko_graph`, core and parser are untouched. The 6 P-graph dump
hashes and every old fixture cardinality are unchanged.

## 1. Pre-change audit — all eight findings verified TRUE

| # | Finding | Verification | Fix |
|---|---|---|---|
| 1 | `WorkloadSpec::scale` unused by the generator | 0 refs to `.scale` | `scale` removed; `scale_of(incidences)` computes the label |
| 2 | `WorkloadSpec::seed` unused | 0 refs to `.seed` | seed is a per-variant field only on the stochastic families |
| 3 | "large" cases were ~thousands of incidences | sunflower petals=400 → 6400 inc | real large cases at $10^5$–$10^6$ now run; old ones reclassified |
| 4 | oracle common-mode with the edge list | `incidences = edges…sum()`, `units = edges.len()` | oracle computed by formula; edges cross-checked against it |
| 5 | size guard ran after the edge list was built | `est` computed after `structure()` | `guard_size` runs before any edge `Vec` is built |
| 6 | unchecked `usize` mul in sunflower/spine-leaf | `core + petals*petal`, `spines*leaves` | `cadd`/`cmul` (checked) → typed `ArithmeticOverflow` |
| 7 | fingerprint used `DefaultHasher` | line 72 of the old bench | FNV-1a-64 with a frozen known-answer vector |
| 8 | 99.64 % file coverage ≠ 100 % touched-line | line 473 uncovered | 100 % (282/282) via a `#[cfg(test)]` helper test |

## 2. Retracted claim

The previous report's "all families' small … lowered" was true only of the six
families then implemented. This is corrected additively (the old report keeps its
text; the family coverage is the new `GRAPH-WL-*` rows). No number in the old
report was silently rewritten.

## 3. Family inventory — before / after

| Category | before (6) | added (11) |
|---|---|---|
| mathematical | Fano, MycielskiC5, MycielskiGrotzsch, Sunflower | **Steiner{m}** (parametric STS), **Mycielski{iterations}** |
| random | — | **RandomUniform**, **RandomDegree**, **RandomHub** (SplitMix64) |
| network | SpineLeaf | **Vlan** (failure-domain), **ServiceDependency** |
| robotics | RobotChain | **Humanoid**, **Hexapod** |
| biomedical | — | **ChickenPhysiology**, **ChickenFlockContact** |

## 4. Generator architecture

`tests/workload/{mod,families,rng,fnv}.rs`: one generator per family, used by the
suite, the goldens, the scaling tests and the benchmark example (no separate
benchmark generator). Each family maps explicit parameters to a `Structure`
(vertices + signed edges + semantic names) **and, independently, to an
`ExpectedStructure` oracle by formula**; `assemble` cross-checks the built edges
against the oracle (F.11–F.16, E). Every valid workload's primary form is HyMeKo
source compiled through parser → `ModuleStore` → `Ir` → `meta_resolve`. Direct
`Ir` construction is confined to the malformed-IR negatives.

**Safety (D):** counts, byte estimate and max cardinality are checked-arithmetic
before allocation; `usize::MAX`-adjacent parameters return `ArithmeticOverflow` or
`TooLarge` without allocating (tested). `SplitMix64` and `FNV-1a-64` are documented,
dependency-free, with frozen KAT vectors.

## 5. Parameters & formulas

| Family | oracle |
|---|---|
| Steiner{m} | $v=2^m-1$, $b=v(v-1)/6$, inc $=3b$, blocks $\{a,b,a\oplus b\}$ |
| Mycielski{it} | $(n_0,m_0)=(5,5)$; $n'=2n+1$, $m'=3m+n$; 2-uniform |
| RandomUniform | mat $=$ nodes, units $=$ edges, inc $=$ edges·card |
| RandomDegree | cardinality cycles $[\min,\max]$; inc $= \sum$ cardinalities |
| RandomHub | inc $=$ edges·card; incidence concentrates on hubs |
| Vlan | mat $=$ switches$+$hosts, units $=$ vlans$+$domains, inc $=$ hosts$+$switches |
| ServiceDependency | mat $=$ instances$+$shared, units $=$ instances, inc $=$ units·(1+dep_card) |
| Humanoid/Hexapod | per_joint $=2+$sensors$+$actuators; units $=2\cdot$joints$+2$ |
| ChickenPhysiology | 14 organs, 5 processes, 20 incidences |
| ChickenFlockContact | inc $=$ (shared$+3\cdot$birds)$+$windows·group |

## 6. Golden fixtures

16 goldens under `tests/data/workloads/` (one per family). The 5 pre-existing
(`steiner_fano_7`, `sunflower_small`, `mycielski_c5`, `network_spine_leaf_small`,
`robot_chain_small`) are **byte-identical to `a5ec3e7`** (verified by `git diff`) —
the new generator emits the same source. 11 new goldens added, each byte-identical
to the generator (F.12) and lowering through the filesystem `compile_to_lowered`
path (F.11). A kept `#[ignore]` `regenerate_golden_fixtures` re-emits them.

## 7. Parser-path evidence

`generator_matches_golden_fixtures`, `golden_fixtures_lower_through_the_filesystem_path`
and `every_family_lowers_to_its_formula_oracle` together prove every family reaches
`meta_resolve` through the real parser on both the filesystem and in-memory paths,
asserting the exact oracle (never `is_ok()`).

## 8. Test matrix & counts

`workloads_suite.rs`: **31 tests + 2 ignored** (stress, golden-regenerate), plus
the SplitMix64 and FNV KAT tests. Coverage per family includes: known-answer
formula, structural formula (Steiner pair-coverage, Mycielski/Grötzsch
triangle-free, hub skew, VLAN membership, humanoid symmetry, hexapod per-leg,
flock multiway), determinism, golden byte-identity, both parser paths, exact
cardinality, metamorphic (Steiner rank, sunflower petal sweep, VLAN add-one),
parameter validation, checked-arithmetic boundary, scale classification,
fingerprint stability. `meta_resolve_ir_faults.rs` (26) and the malformed
negatives are unchanged. **Workspace: 383 tests + 2 doctests pass.**

## 9. Fault injection — five generator mutants, each caught

| mutant | caught by |
|---|---|
| drop a Steiner block | `steiner_formula_matches` |
| ignore the random seed | `random_seed_change_changes_topology_not_counts` |
| drop a hexapod leg | `hexapod_leg_count_changes_the_structure` |
| delete a VLAN membership | `vlan_membership_and_failure_domains_are_exact` |
| flatten a chicken multiway edge | `chicken_physiology_has_named_systems_and_multiway_processes` |

Own error found here: the seed mutant initially *escaped* because the test
compared full sources and the root name encodes the seed; the test was
strengthened to compare the topology body, after which the mutant is caught. All
mutants restored; `families.rs` verified against sha256 `744b3428…`; no `MUT`
marker remains.

## 10. Coverage

`meta_resolve.rs` **92.20 % (m2b baseline) → 99.64 % (m2b-source) → 100.00 %**
(282/282) this phase, via one `#[cfg(test)]` unit test for `bases_of`'s defensive
`DanglingDeclaration` arm — no exclusion, no production behaviour change.
Workspace production 77.42 % → 77.49 %. Test-support (generator) coverage is
**UNMEASURED**: `cargo llvm-cov` instruments workspace `src`, not `tests/`; the
generator is instead pinned by the formula cross-check, the KAT vectors and the
five caught mutants.

## 11. Size classes actually run

Every family tiny + small on the default path. Medium (run): Steiner(m=6),
random-uniform(32 000 inc), VLAN(8 320 inc), humanoid, flock. **Large (run once
this phase):** Steiner(m=9, 130 305 inc), random-uniform(200 000 inc),
VLAN(262 656 inc) — the `large_math_random_and_network_examples_lower` test is on
the default path and passed. Stress is `#[ignore]` with a documented command; not
counted as PASS.

## 12. Benchmark survey (release, medians; topologies never averaged)

Control (original four resident cells, reproduced): sunflower_medium 125.9 µs,
spine_leaf_medium 82.3 µs, robot_chain_medium 259.5 µs, grotzsch 10.1 µs resident —
consistent with the m2b-source numbers, proving the harness refactor did not change
the measurement. Survey highlights (resident µs · ns/incidence):
STS15 21.2·202, STS31 128.2·276, STS127 3238.6·405, mycielski_iter5 2225.1·471,
random_uniform(20000 inc) 8893.3·445, VLAN 1279.9·606, humanoid 127.6·406,
hexapod 70.8·354, chicken_flock 780.4·307. Source generation, parsing (resident)
and hashing are outside every timed region; the fingerprint is FNV-1a-64 with a
byte length on every line.

## 13. Amaterasu

Not run this phase — a test/workload correction needs no second-host campaign, and
no production code changed. The stress `#[ignore]` cases carry documented commands
for later cross-host use.

## 14. V&V rates

| Measure | now |
|---|---|
| Evidence coverage | 44/45 = 97.8 % |
| Implemented normative satisfaction | 38/38 = 100 % |
| End-state normative satisfaction | 38/39 = 97.4 % |
| Open / fixed defects | 0 / 2 |

38 NORMATIVE (all PASS), 2 KNOWN_DEFECT (FIXED), 4 INFORMATIONAL, 1
NOT_IMPLEMENTED (F.17, deferred). Nineteen new `GRAPH-WL-*` normative rows, one per
family/infrastructure requirement — not one blanket PASS.

## 15. Remaining NOT_IMPLEMENTED

**F.17** (dynamic before/after source-pair differential oracle) stays
NOT_IMPLEMENTED, deferred per the brief. The generator already supports emitting
two independent specs, which is the hook for that future transaction /
dynamic-update oracle.

## 16. Own errors & unproven claims

- The seed mutant escaped a first, too-weak test (root-name confound); fixed and
  recorded above.
- The flock membership incidence formula was wrong on first write (omitted the
  shared-resource vertices); caught by `assemble`'s cross-check during bring-up and
  corrected before any golden or test was committed.
- Test-support coverage is reported **UNMEASURED**, not estimated.
- The biomedical families are **structural benchmarks, explicitly not validated
  biological models** — stated in the source docs, the family docs and here.

## Files touched

| File | change |
|---|---|
| `hymeko_pgraph/src/meta_resolve.rs` | +29 (one `#[cfg(test)]` coverage module; production behaviour unchanged) |
| `hymeko_pgraph/tests/workload/{mod,families,rng,fnv}.rs` | rewritten / new generator |
| `hymeko_pgraph/tests/workloads_suite.rs` | rewritten suite |
| `hymeko_pgraph/examples/meta_resolve_bench.rs` | rewritten survey |
| `hymeko_pgraph/tests/data/workloads/*.hymeko` | 5 unchanged + 11 new goldens |
| `docs/plans/2026-08-30-graph-workload-corrective/*` | plan bundle |
| `docs/verification/graph/{requirements.md,traceability.csv}` | corrective + 19 rows |
| `reports/2026-08-30-graph-workload-vv-corrective.md` | this report |
| `reports/coverage/graph-workload-vv-after.lcov` | coverage artifact |

`Cargo.toml`, `Cargo.lock`, `lib.rs`, other `src`, `hymeko_graph`, core, parser —
unchanged.

## HARD STOP

No F.17/dynamic-mutation work, no stable ID, no transaction API, no M2c/M2d/M1, no
next consumer repository, no production P-graph optimisation.
