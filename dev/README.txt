Native Graph development linkage -- 2026-10-28

Run from the Graph workspace:
  cargo dev-test --test pagerank
  cargo dev-test --test informed_search_vv --test metric_tour
  cargo dev-test --test direct_ir_vv --test matroid_levi_vv
  cargo dev-run --example render_fixture_gallery -- target/dev-gallery.json

The aliases select the dev-dll profile and a publish=false downstream client.
The library in dylib/ only re-exports hymeko_graph. Cargo compiles the original
test/example source paths. The pinned Core dependency has exactly the same
identity in the DLL and its clients; no new IR, conversion or algorithm exists.

The manifest registers 27 existing correctness targets, including PageRank,
Berge/A*, tours, direct-IR traversal, topology, matroids and cycle search.
Use cargo metadata --no-deps to inspect the exact list, and select the relevant
targets for each change. All 27 can be compiled without running them using:
  cargo dev-test --tests --no-run
Existing tools: render_fixture_gallery and cycle_stats. Their own workload and
measurement gates still apply; linking a tool is not permission to run a study.
Other suites, library-private unit tests, custom-global-allocator tests and
benchmark/evidence executables retain their original static targets. In
particular a passing DLL test does not establish allocator-counter coverage
across a shared-library boundary. No publication benchmark changes strategy.

The profile is incremental with line-table debug data. It omits full variable
and type debugging; ordinary dev remains available for that work. Standard
default workspace members and release settings are unchanged.

Cargo supplies the matching DLL search paths. Use these Cargo aliases instead
of launching a lone copied EXE. A portable bundle additionally needs the Graph
DLL, matching Rust standard-library DLL and native runtimes. Rust DLLs require
the same compiler/dependency build and do not define a stable plugin ABI.

Smaller EXEs alone do not prove a smaller total distribution: include the shared
DLLs once when comparing a group of clients. A single small static client may
need less total storage because the linker discards unused library code.

Identical commands should return Fresh artifacts. Source, toolchain, features,
profile or dependency changes may legitimately rebuild them. The shared cache
does not make incompatible builds identical. Do not cargo clean that shared
build directory, and obtain executable paths from Cargo JSON artifacts.

CACHE IDENTITY
The aliases place intermediates under
  {cargo-cache-home}/hymeko-dev-build/{workspace-path-hash}
Cargo expands these placeholders using the actual workspace manifest. Clients
in one workspace share this cache; distinct source roots cannot reuse each
other's local package outputs. A first build for each workspace is required.
This prevents false Fresh results observed with an unqualified shared build-dir
when switching between an edited checkout and a different-source verification
export. Ordinary static/benchmark profiles are not changed by the aliases.
The first location is Cargo's cache home, not a hard-coded machine path.
