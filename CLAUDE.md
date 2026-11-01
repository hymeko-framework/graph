@AGENTS.md
@../../hymeko/docs/agent-contract/global.md

# Claude Code — Operating Contract (`graph` repository)

**This repository is the CANONICAL owner of the HyMeKo graph layer.** Decided 2026-08-30, after an
audit found a crate named `hymeko_graph` living in **both** `graph/hymeko_graph` and
`tensor/hymeko_graph` — 40 shared paths, 16 byte-identical, 2 formatting-only, 22 with real token
deltas, drifting in both directions by date. One owner split across two repositories is not a
naming coincidence; it is a defect that makes every claim about graph behaviour ambiguous.

## Ownership

| repository | owns |
|---|---|
| **`graph`** (this one) | `hymeko_graph` — generic signed-graph and hypergraph algorithms, incidence, cycle and walk enumeration, CSR primitives. `hymeko_pgraph` — P-graph and Friedler, **solely**. |
| `core` | the backend-independent compute IR (`hymeko_core::ir`) and the WGPU backend (`hymeko_wgpu`), which is hardware-tested. |
| `tensor` | numerical primitives: Clifford algebra, spline kernels, tensor arenas, vision rasterisation. |
| `nagare` | model prototypes, trainers, studies, application orchestration. |

**`tensor` may CONSUME `hymeko_graph`; it may not vendor it.** A second copy under any name
re-creates the defect this contract exists to close. Friedler belongs to `hymeko_pgraph` and must
never be reintroduced into the generic graph crate.

**This repository owns no compute IR and no backend.** If graph work appears to need one, that is a
signal to consume `hymeko_core`, not to grow a second one here — the mistake a 2026-08-30 session
came within one message of making, because twelve of fourteen local checkouts were stale stubs and
`core` looked empty.

## What binds here in addition to the shared contract

1. **One production entry point per capability.** The V&V architecture block below is authoritative
   and is byte-identical to the copies in the workspace root, `nagare` and `tensor`.
2. **Capture an independent oracle BEFORE deleting an owner.** A retired implementation's behaviour
   must have an executable owner in a `publish = false` V&V package before its source is removed.
   Deletion first and reconstruction later is not available.
3. **No compatibility surfaces in production.** Specifically forbidden: `Legacy*` / `Recorded*`
   type or variant names, `_under` suffixes, mode booleans that select between an old and a new
   arithmetic, and any runtime selector whose only purpose is to reproduce a retired result. A
   retired result lives in V&V, or it does not live.
4. **Dependency direction is one-way and acyclic.** `graph` consumes the canonical
   `hymeko_core::ir` from `core`; Core does not depend on Graph. See `CORE.YAML`
   and the workspace manifest. Adding a different dependency is a core-level decision.

## Known state, 2026-08-30

`hymeko/graph` had no tracked `CLAUDE.md`, `AGENTS.md` or `CORE.YAML` before this commit — it was
being made canonical while carrying no operating contract and no core protection at all.

<!-- BEGIN SHARED V&V ARCHITECTURE -->
### V&V architecture: one production entry point, many downstream checks

Verification asks whether the implementation satisfies its mathematical,
structural, numerical, state, and performance contracts. Validation asks
whether that verified implementation reproduces the intended real use case,
dataset protocol, artifact, or scientific claim. A completed feature needs both
where both apply; passing verification alone does not validate a paper claim,
and matching one artifact does not verify the implementation.

**Production topology.** Each capability has exactly one authoritative
production entry point and one implementation owner. The framework may be a
hierarchy of multiple cohesive crates/modules/libraries; "one library" means
one directed ownership graph presented as one framework, not one monolithic
source file. Orthogonal variation is expressed through typed configuration,
traits, or sealed enums behind that entry point. Do not create a parallel model,
trainer, optimizer, numerical kernel, runner, or study-named production module
in `examples/`, `experiments/`, tests, benches, or a paper panel.

**Tests are clients.** Normal tests construct inputs and typed configuration,
then call existing production functions/methods. They may compare bits, inspect
neutral read-only state, calculate hashes/statistics, perform finite
differences, and assert invariants. Test helpers may own fixture builders,
observers, comparators, deterministic generators, and artifact parsers. They
must not introduce new production behavior or silently recreate the model,
training loop, optimizer, backward pass, or orchestration being tested. If a
test requires a new generally useful operation, that is a missing production
owner: design it in the framework first, then test it through the canonical
entry point.

**Independent-oracle exception.** Avoid tautological tests: an equivalence test
must not obtain its expected value by calling the same composite it checks. A
private test-only oracle is therefore permitted only when it is:

1. a frozen implementation captured from a known-working example/experiment
   before retirement, or a minimal closed-form/high-precision reference;
2. recorded with its origin, source hash, arithmetic contract, and known scope;
3. non-public, `publish = false` where packaged, and unreachable from every
   production dependency path;
4. used only to produce expected observations, never selected as a runtime
   implementation; and
5. frozen rather than developed into another framework.

Do not write a fresh alternative model or trainer after the original has been
deleted and call it an oracle. Capture the working reference first. Retaining a
retired body in a private V&V package is provenance, not permission to retain a
`Legacy*` production arm.

**Separate test packages.** When the V&V surface is substantial, use downstream
workspace packages instead of putting engines back into examples:

- a `publish = false` contract/equivalence package for fast deterministic,
  synthetic, bit-parity, adjoint, finite-difference, and optimizer-lockstep
  verification;
- a `publish = false` reproduction package for datasets, paper configurations,
  historical artifacts, and explicitly invoked real-data validation.

Dependency direction is one-way: V&V packages depend on the production
framework; production never depends on V&V packages. Their tests compile under
the workspace gate. Data-dependent validations may be `#[ignore]`, but an
explicit run must fail clearly when required data is absent rather than
silently skip. Tests never read an NFS path implicitly.

**Examples and experiments.** Examples are thin demonstrations of the public
API, not owners of models, gradients, optimizers, or research engines.
Experiments/panels may choose configuration, data, seeds, and reporting, but
call the same production entry point. Before retiring an existing engine, map
initialization, forward, backward, optimizer state, evaluation, fixtures, and
artifact claims to permanent V&V tests; capture and hash any independent oracle;
then reconcile the test population exactly. Deletion is blocked until every
live responsibility has an executable test owner.

**One full gate.** Focused package tests are useful discriminating sub-gates,
not competing top-level runners. Each repository keeps one canonical full V&V
entry point (for example `scripts/check.sh`) that compiles/runs the applicable
contract packages, reproduction packages, integration tests, doctests,
bench-compilation checks, and repository policy gates.

<!-- END SHARED V&V ARCHITECTURE -->
