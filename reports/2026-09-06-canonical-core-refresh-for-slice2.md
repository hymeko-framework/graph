# Canonical Core refresh for Nagare Slice 2

Created-at: 2026-09-06 14:48 +02:00

## Outcome

Graph `main` was first fetched and fast-forwarded to the current remote tip
`2b619a39c9982f47db7cbe49bba6dd8c7f9b4579`. Its only Core dependency still
resolved `92383b38`; this boundary moves it to canonical Core
`fc3eda9b73e4f171aada5c79751af481759c068e`. The lock update changed exactly
that one package source. No transitive dependency moved.

The first focused build exposed `CanonicalCoreApiDrift`: Tier 2 added
`RefAtomR::direction`, while 35 Graph/P-Graph construction sites still used
pre-field literals. The complete census was 27 in `hymeko_graph` and 8 in
`hymeko_pgraph`; all are fixtures or fixture construction inside tests. Each
now states `direction: Default::default()`, which is exactly
`PortDirection::Unspecified`. No sign, target, annotation, weight, algorithm,
or production graph result changed.

## Evidence

| Gate | Result |
|---|---|
| Core lock diff | one source line, `92383b38 -> fc3eda9b` |
| `RefAtomR` census | 35 literals, 35 explicit default directions |
| focused direct-IR test | 6 passed, 0 failed |
| `cargo test --locked --workspace --all-targets --no-fail-fast` | 552 passed, 0 failed, 19 ignored |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo fmt --all -- --check` | exit 0 |
| `scripts/check_governance.sh HEAD` | 8 checks, 0 failures |
| `git diff --check` | exit 0 |

The full test command includes the 232.72 s `perf_matrix` test. It was allowed
to finish without a concurrent Cargo chain. This is a correctness gate, not a
new performance claim.

## Scope and authority

The user explicitly required one current Core identity and authorised carrying
the canonical integration through. No new worktree was created. The change is
limited to the dependency pair, the test-fixture API migration, this plan, and
this report. The historical P2-C nine-repetition campaign was not launched.

## Next boundary

After commit and verified push, Nagare must move both Graph declarations
together to this Graph tip, certify exactly one Core and one Graph instance,
and only then implement signed HSiKAN `Ir -> ExecutionPlan -> CPU/WGPU`.
