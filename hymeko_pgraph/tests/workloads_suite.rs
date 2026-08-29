//! Source-first P-graph workload suite (F.11–F.16).
//!
//! Every workload is a real HyMeKo description compiled through the parser and
//! lowering pipeline (`compile_to_lowered` for the golden files,
//! `compile_sources` for the generated ones). No test here builds an `Ir` or a
//! `LoweredPGraph` directly.

mod workload;

use std::path::PathBuf;

use workload::{
    GeneratedHymeko, WorkloadFamily, WorkloadScale, WorkloadSpec, assert_expected_structure,
    generate_hymeko, lower_generated, normalize_newlines,
};

fn spec(family: WorkloadFamily) -> WorkloadSpec {
    WorkloadSpec {
        family,
        scale: WorkloadScale::Tiny,
        seed: 0,
    }
}

fn build_wl(family: WorkloadFamily) -> GeneratedHymeko {
    generate_hymeko(&spec(family)).expect("generation succeeds")
}

// ── generator sanity: each family lowers with its oracle ───────────────────

#[test]
fn fano_generated_lowers_to_its_oracle() {
    let g = build_wl(WorkloadFamily::Fano);
    assert_eq!(g.expected.materials, 7);
    assert_eq!(g.expected.units, 7);
    assert_eq!(g.expected.incidences, 21);
    let lowered = lower_generated(&g).expect("Fano lowers");
    assert_expected_structure(&lowered, &g.expected);
}

#[test]
fn mycielski_c5_generated_lowers_to_its_oracle() {
    let g = build_wl(WorkloadFamily::MycielskiC5);
    assert_eq!(g.expected.materials, 5);
    assert_eq!(g.expected.units, 5);
    assert_eq!(g.expected.incidences, 10);
    let lowered = lower_generated(&g).expect("C5 lowers");
    assert_expected_structure(&lowered, &g.expected);
}

#[test]
fn mycielski_grotzsch_generated_lowers_to_its_oracle() {
    let g = build_wl(WorkloadFamily::MycielskiGrotzsch);
    // Grötzsch: 11 vertices, 20 edges, all size 2.
    assert_eq!(g.expected.materials, 11);
    assert_eq!(g.expected.units, 20);
    assert_eq!(g.expected.incidences, 40);
    let lowered = lower_generated(&g).expect("Grötzsch lowers");
    assert_expected_structure(&lowered, &g.expected);
}

#[test]
fn sunflower_generated_lowers_to_its_oracle() {
    let family = WorkloadFamily::Sunflower {
        core: 2,
        petal: 2,
        petals: 3,
    };
    let g = build_wl(family);
    // n = 2 + 3*2 = 8 ; edges = 3 ; each size 4 ; incidences = 12.
    assert_eq!(g.expected.materials, 8);
    assert_eq!(g.expected.units, 3);
    assert_eq!(g.expected.incidences, 12);
    let lowered = lower_generated(&g).expect("sunflower lowers");
    assert_expected_structure(&lowered, &g.expected);
}

#[test]
fn spine_leaf_generated_lowers_to_its_oracle() {
    let family = WorkloadFamily::SpineLeaf {
        spines: 2,
        leaves: 3,
    };
    let g = build_wl(family);
    // n = 5 ; edges = 2*3 = 6 ; each size 2 ; incidences = 12.
    assert_eq!(g.expected.materials, 5);
    assert_eq!(g.expected.units, 6);
    assert_eq!(g.expected.incidences, 12);
    let lowered = lower_generated(&g).expect("spine/leaf lowers");
    assert_expected_structure(&lowered, &g.expected);
}

#[test]
fn robot_chain_generated_lowers_to_its_oracle() {
    let family = WorkloadFamily::RobotChain { joints: 4 };
    let g = build_wl(family);
    // n = 5 ; edges = 4 ; each size 2 ; incidences = 8.
    assert_eq!(g.expected.materials, 5);
    assert_eq!(g.expected.units, 4);
    assert_eq!(g.expected.incidences, 8);
    let lowered = lower_generated(&g).expect("robot chain lowers");
    assert_expected_structure(&lowered, &g.expected);
}

// ── determinism ─────────────────────────────────────────────────────────

#[test]
fn generation_is_deterministic() {
    for family in [
        WorkloadFamily::Fano,
        WorkloadFamily::MycielskiGrotzsch,
        WorkloadFamily::SpineLeaf {
            spines: 3,
            leaves: 3,
        },
    ] {
        let a = build_wl(family);
        let b = build_wl(family);
        assert_eq!(a.source, b.source, "{family:?} source is deterministic");
        assert_eq!(a.expected, b.expected);
    }
}

// ── golden byte-identity (F.12) ────────────────────────────────────────────

fn workload_golden(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/workloads")
        .join(rel)
}

/// Each tiny generated source is byte-identical to a version-controlled,
/// human-readable golden fixture (modulo newline style), and the golden itself
/// lowers to the same oracle through the filesystem `compile_to_lowered` path.
#[test]
fn generator_matches_golden_fixtures() {
    let cases = [
        (WorkloadFamily::Fano, "steiner_fano_7.hymeko"),
        (WorkloadFamily::MycielskiC5, "mycielski_c5.hymeko"),
        (
            WorkloadFamily::Sunflower {
                core: 2,
                petal: 2,
                petals: 3,
            },
            "sunflower_small.hymeko",
        ),
        (
            WorkloadFamily::SpineLeaf {
                spines: 2,
                leaves: 3,
            },
            "network_spine_leaf_small.hymeko",
        ),
        (
            WorkloadFamily::RobotChain { joints: 4 },
            "robot_chain_small.hymeko",
        ),
    ];
    for (family, file) in cases {
        let g = build_wl(family);
        let golden = std::fs::read_to_string(workload_golden(file))
            .unwrap_or_else(|e| panic!("read golden {file}: {e}"));
        assert_eq!(
            normalize_newlines(&g.source),
            normalize_newlines(&golden),
            "generated source for {family:?} must equal golden {file}"
        );
    }
}

/// Regenerate the version-controlled golden fixtures from the generator.
///
/// Ignored by default (it writes into the source tree). Run with
/// `cargo test -p hymeko_pgraph --test workloads_suite regenerate_golden_fixtures -- --ignored`
/// after an intentional generator change, then review the diff and commit.
#[test]
#[ignore = "writes golden fixtures; run explicitly to regenerate"]
fn regenerate_golden_fixtures() {
    let cases = [
        (WorkloadFamily::Fano, "steiner_fano_7.hymeko"),
        (WorkloadFamily::MycielskiC5, "mycielski_c5.hymeko"),
        (
            WorkloadFamily::Sunflower {
                core: 2,
                petal: 2,
                petals: 3,
            },
            "sunflower_small.hymeko",
        ),
        (
            WorkloadFamily::SpineLeaf {
                spines: 2,
                leaves: 3,
            },
            "network_spine_leaf_small.hymeko",
        ),
        (
            WorkloadFamily::RobotChain { joints: 4 },
            "robot_chain_small.hymeko",
        ),
    ];
    for (family, file) in cases {
        let g = build_wl(family);
        std::fs::write(workload_golden(file), g.source.as_bytes())
            .unwrap_or_else(|e| panic!("write golden {file}: {e}"));
    }
}

// ── golden fixtures via the filesystem parser path (F.11) ──────────────────

/// The golden files compile through the *filesystem* `compile_to_lowered` path
/// (resolving `@"meta_pgraph.hymeko"` beside them) and lower to the same oracle
/// as the generator. This is the human-readable, version-controlled entry.
#[test]
fn golden_fixtures_lower_through_the_filesystem_path() {
    let cases = [
        (WorkloadFamily::Fano, "steiner_fano_7.hymeko"),
        (WorkloadFamily::MycielskiC5, "mycielski_c5.hymeko"),
        (
            WorkloadFamily::Sunflower {
                core: 2,
                petal: 2,
                petals: 3,
            },
            "sunflower_small.hymeko",
        ),
        (
            WorkloadFamily::SpineLeaf {
                spines: 2,
                leaves: 3,
            },
            "network_spine_leaf_small.hymeko",
        ),
        (
            WorkloadFamily::RobotChain { joints: 4 },
            "robot_chain_small.hymeko",
        ),
    ];
    for (family, file) in cases {
        let expected = build_wl(family).expected;
        let g = hymeko_pgraph::compile_to_lowered(&workload_golden(file))
            .unwrap_or_else(|e| panic!("golden {file} lowers: {e:?}"));
        assert_expected_structure(&g, &expected);
    }
}

// ── run policy (F.15): small always; medium; a math/network/robot large ────
//
// These run on the default `cargo test` path (they are in-memory and bounded).
// The stress sizes live behind `#[ignore]` below.

fn run_scale(family: WorkloadFamily) {
    let g = build_wl(family);
    let lowered = lower_generated(&g).unwrap_or_else(|e| panic!("{family:?} lowers: {e:?}"));
    assert_expected_structure(&lowered, &g.expected);
}

#[test]
fn small_examples_of_every_family_lower() {
    run_scale(WorkloadFamily::Fano);
    run_scale(WorkloadFamily::MycielskiC5);
    run_scale(WorkloadFamily::MycielskiGrotzsch);
    run_scale(WorkloadFamily::Sunflower {
        core: 2,
        petal: 3,
        petals: 4,
    });
    run_scale(WorkloadFamily::SpineLeaf {
        spines: 3,
        leaves: 4,
    });
    run_scale(WorkloadFamily::RobotChain { joints: 8 });
}

#[test]
fn medium_examples_of_three_families_lower() {
    // A mathematical, a network and a robotics medium workload.
    run_scale(WorkloadFamily::Sunflower {
        core: 4,
        petal: 4,
        petals: 40,
    }); // 164 materials, 40 units
    run_scale(WorkloadFamily::SpineLeaf {
        spines: 8,
        leaves: 16,
    }); // 24 materials, 128 units
    run_scale(WorkloadFamily::RobotChain { joints: 200 }); // 201 materials, 200 units
}

#[test]
fn large_math_network_and_robot_examples_lower() {
    // One mathematical large, one network large, one robotics large (F.15).
    run_scale(WorkloadFamily::Sunflower {
        core: 8,
        petal: 8,
        petals: 400,
    }); // 3208 materials, 400 units
    run_scale(WorkloadFamily::SpineLeaf {
        spines: 32,
        leaves: 64,
    }); // 96 materials, 2048 units
    run_scale(WorkloadFamily::RobotChain { joints: 4000 }); // 4001 materials, 4000 units
}

#[test]
#[ignore = "stress scale; run explicitly"]
fn stress_examples_lower() {
    run_scale(WorkloadFamily::SpineLeaf {
        spines: 128,
        leaves: 256,
    }); // 32768 units
    run_scale(WorkloadFamily::RobotChain { joints: 50_000 });
}
