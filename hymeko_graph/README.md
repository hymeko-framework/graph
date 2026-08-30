# hymeko_graph

Generic signed-graph cycle and walk enumeration with **axiomatic
pruning**.

A pure-Rust library that lifts the cycle / walk enumeration code
out of `hymeko_py` so it can be used outside the KAN /
deep-learning pipeline.  The headline feature is the pluggable
`CyclePruner` trait: cycles that violate domain axioms are
**pruned during DFS, not after enumeration**.

> Previously `hymeko_framework_rust/hymeko_graph`, with a
> byte-identical vendored copy in `nagare`. This is the canonical
> one; see the [repository README](../README.md) for how it sits
> beside `hymeko_pgraph` and `hotaru::hypergraph`, which sound
> similar and are not.

## Not a hypergraph library

Everything here is a *pairwise* signed graph — `SignedGraph` is
`edges: Vec<(u32, u32)>`. A hypergraph reaches these algorithms
only by being star-expanded first, which is what
`BipartiteOnlyPruner` exists for. For hypergraphs as hypergraphs,
see [`hotaru::hypergraph`](http://amaterasu.local:3000/hymeko/hotaru).

## What's in the box

- `SignedGraph` + CSR adjacency builder (no `unsafe`, no Python deps)
- `CyclePruner` trait — `extend_ok` (DFS-time pre-check) +
  `emit_ok` (post-completion verification)
- Pre-built balance pruners:
  - `CartwrightHararyPruner` — balanced-only or unbalanced-only
    sign-product filter (Cartwright-Harary 1956)
  - `DavisWeakBalancePruner` — exclude all-negative triangles
    (Davis 1967)
  - `BipartiteOnlyPruner` — drop odd-length cycles (handy after
    star-expansion)

Domain-specific pruners belong on the other side of the trait.
`hymeko_pgraph::FriedlerAxiomPruner` is the worked example: it
encodes Friedler's A0/A1/A3 for P-graphs, and lived here until
the vocabulary followed it into the crate that already had it.

## Why pruning matters

A pruner consulted mid-DFS cuts a branch before it materialises,
so the enumeration costs what the *feasible* set costs rather
than what the full one does. On a bipartite graph, rejecting
same-colour steps skips every odd-length search path outright —
an immediate ~2× on top of the rayon parallelism, before any
domain constraint narrows it further.

The two-level API mirrors how Friedler's accelerated branch-and-
bound works: cheap structural checks during the search
(`extend_ok`), expensive verification once a candidate is ready
(`emit_ok`).

## Status

The migration the old status note was waiting on has happened. The
enumerator body is here — `cycle_enum.rs` and the 4,100 lines of
`topk_cycles.rs` — not in `hymeko_py/src/cycles.rs`, and it takes
the `&dyn CyclePruner` the note anticipated.

**167 tests** — 109 unit, 58 integration. The Friedler scenarios and the
two axiom examples moved to `hymeko_pgraph` along with the pruner.

## Building

```
cargo test -p hymeko_graph   # two dependencies, rayon and dashmap
cargo build --examples
cd ../benchmarks && cargo bench
```

The benchmarks are a **separate crate with its own workspace
root**, so the library never resolves their dependencies.
Criterion's closure is around a hundred and twenty crates
including a wasm-bindgen toolchain, and needing all of it to run
`cargo test` on a graph library is the wrong trade. Nothing is
lost — the bench sources are unchanged in `benchmarks/benches/` —
but running them needs crates.io, which the vendored-registry
build machines deliberately do not carry.

`proptest` was dropped from the dev-dependencies on extraction: it
was declared but no test, bench or example ever used it.

## Worked example

Writing a pruner: reject any step between two vertices of the
same colour, and emit only even-length cycles. This is the
generic shape of what `hymeko_pgraph`'s Friedler A0 does with
Material and Operating-Unit as the two colours.

```rust
use hymeko_graph::{CyclePruner, PrunerDecision, SignedGraph, enumerate_simple_cycles};

struct TwoColour { colour: Vec<bool> }

impl CyclePruner for TwoColour {
    fn extend_ok(&self, path: &[u32], next: u32) -> PrunerDecision {
        match path.last() {
            Some(&tail) if self.colour[tail as usize] == self.colour[next as usize] =>
                PrunerDecision::Reject,
            _ => PrunerDecision::Accept,
        }
    }
    fn emit_ok(&self, cycle: &[u32], _signs: &[i8]) -> PrunerDecision {
        PrunerDecision::from_bool(cycle.len().is_multiple_of(2))
    }
}

// 4-cycle 0-1-2-3 with a chord 0—2 joining two same-coloured vertices.
let g = SignedGraph::from_parts(4, &[0, 1, 2, 3, 0], &[1, 2, 3, 0, 2], &[1; 5]);
let p = TwoColour { colour: vec![false, true, false, true] };

// The chord is refused mid-DFS, so no triangle survives.
assert_eq!(enumerate_simple_cycles(&g, 3, &p).len(), 0);
// The alternating 4-cycle does.
assert_eq!(enumerate_simple_cycles(&g, 4, &p), vec![vec![0, 1, 2, 3]]);
```

For the P-graph axioms themselves, see
[`hymeko_pgraph`](../hymeko_pgraph/) and its `axiom_effect` and
`strategy_pattern` examples.

## Citations

- D. Cartwright, F. Harary, *Structural balance: a generalisation
  of Heider's theory*, Psychological Review, 63(5), 1956.
- J. A. Davis, *Clustering and structural balance in graphs*,
  Human Relations, 20(2), 1967.
- F. Heider, *Attitudes and cognitive organization*, Journal of
  Psychology, 21, 1946.
