//! Source-first P-graph workload suite (F.11–F.16, corrective 2026-08-30).
//!
//! Every valid workload is real HyMeKo compiled through the parser + lowering
//! pipeline: `compile_to_lowered` for the version-controlled golden files,
//! `compile_sources` for the generated ones. No test here builds an `Ir` or a
//! `LoweredPGraph` directly. The oracle is computed from parameters by formula
//! (in `workload`), so a wrong edge list cannot agree with a wrong expectation.

mod workload;

use std::path::PathBuf;

use workload::{
    Built, WorkloadError, WorkloadFamily, WorkloadScale, assert_expected_structure, build,
    lower_generated, normalize_newlines, scale_of, source_fingerprint,
};

/// Build, cross-check (inside `build`), lower through the in-memory parser path,
/// and assert the lowered graph matches the formula oracle.
fn build_and_lower(family: WorkloadFamily) -> Built {
    let b = build(family).unwrap_or_else(|e| panic!("{family:?} builds: {e:?}"));
    let g = lower_generated(&b).unwrap_or_else(|e| panic!("{family:?} lowers: {e:?}"));
    assert_expected_structure(&g, &b.oracle);
    b
}

/// One golden per family.
fn golden_table() -> Vec<(WorkloadFamily, &'static str)> {
    use WorkloadFamily as F;
    vec![
        (F::Fano, "steiner_fano_7.hymeko"),
        (F::Steiner { m: 4 }, "steiner_sts15.hymeko"),
        (
            F::Sunflower {
                core: 2,
                petal: 2,
                petals: 3,
            },
            "sunflower_small.hymeko",
        ),
        (F::MycielskiC5, "mycielski_c5.hymeko"),
        (F::MycielskiGrotzsch, "mycielski_grotzsch.hymeko"),
        (
            F::SpineLeaf {
                spines: 2,
                leaves: 3,
            },
            "network_spine_leaf_small.hymeko",
        ),
        (F::RobotChain { joints: 4 }, "robot_chain_small.hymeko"),
        (
            F::RandomUniform {
                nodes: 8,
                edges: 6,
                card: 3,
                seed: 7,
            },
            "random_uniform_small.hymeko",
        ),
        (
            F::RandomDegree {
                nodes: 8,
                edges: 6,
                min_card: 2,
                max_card: 4,
                seed: 7,
            },
            "random_degree_small.hymeko",
        ),
        (
            F::RandomHub {
                nodes: 10,
                edges: 6,
                hubs: 2,
                hub_ratio_num: 1,
                hub_ratio_den: 2,
                card: 3,
                seed: 7,
            },
            "random_hub_small.hymeko",
        ),
        (
            F::Vlan {
                switches: 4,
                hosts_per_switch: 4,
                vlans: 4,
                power_domains: 2,
            },
            "network_vlan_failure_domain_small.hymeko",
        ),
        (
            F::ServiceDependency {
                services: 3,
                replicas: 2,
                shared_groups: 4,
                dep_card: 2,
                seed: 7,
            },
            "network_service_dependency_small.hymeko",
        ),
        (
            F::Humanoid {
                arm_joints: 2,
                leg_joints: 2,
                sensors_per_joint: 1,
                actuator_redundancy: 1,
            },
            "robot_humanoid_small.hymeko",
        ),
        (
            F::Hexapod {
                legs: 6,
                joints_per_leg: 2,
                sensors_per_joint: 1,
                actuator_redundancy: 1,
            },
            "robot_hexapod_small.hymeko",
        ),
        (F::ChickenPhysiology, "biomed_chicken_system_small.hymeko"),
        (
            F::ChickenFlockContact {
                birds: 12,
                coops: 3,
                feeders: 2,
                drinkers: 2,
                group: 4,
                windows: 3,
                seed: 7,
            },
            "biomed_chicken_flock_small.hymeko",
        ),
    ]
}

fn workload_golden(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/workloads")
        .join(rel)
}

// ── every family → parse → lower → oracle ──────────────────────────────────

#[test]
fn every_family_lowers_to_its_formula_oracle() {
    for (family, _) in golden_table() {
        build_and_lower(family);
    }
}

#[test]
fn generator_matches_golden_fixtures() {
    for (family, file) in golden_table() {
        let b = build(family).unwrap();
        let golden = std::fs::read_to_string(workload_golden(file))
            .unwrap_or_else(|e| panic!("read golden {file}: {e}"));
        assert_eq!(
            normalize_newlines(&b.source),
            normalize_newlines(&golden),
            "generated source for {family:?} must equal golden {file}"
        );
    }
}

#[test]
fn golden_fixtures_lower_through_the_filesystem_path() {
    for (family, file) in golden_table() {
        let expected = build(family).unwrap().oracle;
        let g = hymeko_pgraph::compile_to_lowered(&workload_golden(file))
            .unwrap_or_else(|e| panic!("golden {file} lowers: {e:?}"));
        assert_expected_structure(&g, &expected);
    }
}

// ── known-answer formulas ──────────────────────────────────────────────────

#[test]
fn steiner_formula_matches() {
    for (m, v, b) in [
        (3u32, 7usize, 7usize),
        (4, 15, 35),
        (5, 31, 155),
        (7, 127, 2667),
    ] {
        let built = build(WorkloadFamily::Steiner { m }).unwrap();
        assert_eq!(built.oracle.materials, v, "STS m={m}");
        assert_eq!(built.oracle.units, b, "STS m={m} blocks");
        assert_eq!(built.oracle.incidences, 3 * b);
        assert_eq!(built.max_cardinality, 3);
    }
}

#[test]
fn steiner_covers_every_pair_exactly_once() {
    for m in [3u32, 4, 5] {
        let v = build(WorkloadFamily::Steiner { m })
            .unwrap()
            .oracle
            .materials;
        let mut pair = std::collections::HashMap::<(usize, usize), usize>::new();
        for a in 1..=v {
            for bb in (a + 1)..=v {
                let c = a ^ bb;
                if c > bb {
                    for &(x, y) in &[(a, bb), (a, c), (bb, c)] {
                        *pair.entry((x.min(y), x.max(y))).or_default() += 1;
                    }
                }
            }
        }
        assert_eq!(pair.len(), v * (v - 1) / 2, "all pairs, m={m}");
        assert!(pair.values().all(|&c| c == 1), "each pair once, m={m}");
    }
}

#[test]
fn mycielski_formula_matches() {
    let mut n = 5usize;
    let mut m = 5usize;
    for it in 0..4u32 {
        let b = build(WorkloadFamily::Mycielski { iterations: it }).unwrap();
        assert_eq!(b.oracle.materials, n, "iter {it} vertices");
        assert_eq!(b.oracle.units, m, "iter {it} edges");
        assert_eq!(b.oracle.incidences, 2 * m);
        assert_eq!(b.max_cardinality, 2);
        let (nn, mm) = (2 * n + 1, 3 * m + n);
        n = nn;
        m = mm;
    }
}

fn mycielski_edges_probe(iterations: u32) -> (usize, Vec<Vec<usize>>) {
    let mut n = 5usize;
    let mut edges: Vec<(usize, usize)> = vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)];
    for _ in 0..iterations {
        let apex = 2 * n;
        let mut next = Vec::new();
        for &(a, b) in &edges {
            next.push((a, b));
            next.push((n + a, b));
            next.push((n + b, a));
        }
        for i in 0..n {
            next.push((n + i, apex));
        }
        edges = next;
        n = 2 * n + 1;
    }
    (n, edges.into_iter().map(|(a, b)| vec![a, b]).collect())
}

#[test]
fn grotzsch_is_triangle_free() {
    let verts = build(WorkloadFamily::Mycielski { iterations: 1 })
        .unwrap()
        .oracle
        .materials;
    let (_n, edges) = mycielski_edges_probe(1);
    let eset: std::collections::HashSet<(usize, usize)> = edges
        .iter()
        .map(|e| (e[0].min(e[1]), e[0].max(e[1])))
        .collect();
    let mut triangles = 0;
    for a in 0..verts {
        for bb in (a + 1)..verts {
            if !eset.contains(&(a, bb)) {
                continue;
            }
            for c in (bb + 1)..verts {
                if eset.contains(&(a, c)) && eset.contains(&(bb, c)) {
                    triangles += 1;
                }
            }
        }
    }
    assert_eq!(triangles, 0, "Grötzsch is triangle-free");
}

// ── random families ────────────────────────────────────────────────────────

#[test]
fn random_uniform_is_deterministic_in_the_seed() {
    let spec = WorkloadFamily::RandomUniform {
        nodes: 100,
        edges: 50,
        card: 4,
        seed: 123,
    };
    let a = build(spec).unwrap();
    let b = build(spec).unwrap();
    assert_eq!(a.source, b.source);
    assert_eq!(a.oracle.incidences, 200);
}

/// Strip the first line (the root-name line) so the comparison is of the
/// topology body, not of the seed-encoding root name.
fn body(src: &str) -> String {
    src.split_once('\n')
        .map(|(_, rest)| rest)
        .unwrap_or("")
        .to_string()
}

#[test]
fn random_seed_change_changes_topology_not_counts() {
    let mk = |seed| WorkloadFamily::RandomUniform {
        nodes: 100,
        edges: 50,
        card: 4,
        seed,
    };
    let a = build(mk(1)).unwrap();
    let b = build(mk(2)).unwrap();
    // Topology (not just the root name) must differ.
    assert_ne!(
        body(&a.source),
        body(&b.source),
        "seed must change the topology"
    );
    assert_eq!(a.oracle, b.oracle);
}

#[test]
fn five_random_seeds_all_lower_with_the_same_cardinality() {
    for seed in [1u64, 2, 3, 4, 5] {
        let b = build_and_lower(WorkloadFamily::RandomUniform {
            nodes: 200,
            edges: 120,
            card: 5,
            seed,
        });
        assert_eq!(b.oracle.incidences, 600);
    }
}

#[test]
fn random_hub_skews_incidence_toward_hubs() {
    let b = build(WorkloadFamily::RandomHub {
        nodes: 200,
        edges: 400,
        hubs: 4,
        hub_ratio_num: 1,
        hub_ratio_den: 1,
        card: 3,
        seed: 9,
    })
    .unwrap();
    let g = lower_generated(&b).unwrap();
    let mut deg = std::collections::HashMap::<String, usize>::new();
    for &u in &g.units {
        for m in g.inputs(u).iter().chain(g.outputs(u).iter()) {
            *deg.entry(g.decl_to_name[m].clone()).or_default() += 1;
        }
    }
    let avg = b.oracle.incidences as f64 / b.oracle.materials as f64;
    let hub_deg: usize = (0..4)
        .map(|h| deg.get(&format!("m{h}")).copied().unwrap_or(0))
        .sum();
    assert!(
        hub_deg as f64 / 4.0 > avg * 3.0,
        "hub degree {} vs average {avg:.1}",
        hub_deg / 4
    );
}

// ── network ────────────────────────────────────────────────────────────────

#[test]
fn vlan_membership_and_failure_domains_are_exact() {
    let b = build_and_lower(WorkloadFamily::Vlan {
        switches: 8,
        hosts_per_switch: 4,
        vlans: 4,
        power_domains: 2,
    });
    assert_eq!(b.oracle.materials, 40);
    assert_eq!(b.oracle.units, 6);
    assert_eq!(b.oracle.incidences, 40);
    assert!(b.max_cardinality >= 3);
}

#[test]
fn service_dependency_counts_are_exact() {
    let b = build_and_lower(WorkloadFamily::ServiceDependency {
        services: 5,
        replicas: 3,
        shared_groups: 6,
        dep_card: 3,
        seed: 2,
    });
    assert_eq!(b.oracle.materials, 21);
    assert_eq!(b.oracle.units, 15);
    assert_eq!(b.oracle.incidences, 60);
}

// ── robotics ───────────────────────────────────────────────────────────────

#[test]
fn humanoid_is_left_right_symmetric_and_exact() {
    let b = build_and_lower(WorkloadFamily::Humanoid {
        arm_joints: 3,
        leg_joints: 2,
        sensors_per_joint: 2,
        actuator_redundancy: 2,
    });
    assert_eq!(b.oracle.materials, 63);
    assert_eq!(b.oracle.units, 22);
    for part in [
        "left_arm_joint0",
        "right_arm_joint0",
        "left_leg_joint0",
        "right_leg_joint0",
    ] {
        assert!(b.source.contains(part), "missing {part}");
    }
}

#[test]
fn hexapod_leg_count_changes_the_structure() {
    let six = build(WorkloadFamily::Hexapod {
        legs: 6,
        joints_per_leg: 2,
        sensors_per_joint: 1,
        actuator_redundancy: 1,
    })
    .unwrap();
    let seven = build(WorkloadFamily::Hexapod {
        legs: 7,
        joints_per_leg: 2,
        sensors_per_joint: 1,
        actuator_redundancy: 1,
    })
    .unwrap();
    assert_eq!(seven.oracle.materials - six.oracle.materials, 8);
    assert_eq!(seven.oracle.units - six.oracle.units, 4);
    assert!(seven.source.contains("leg6_joint0"));
    assert!(!six.source.contains("leg6_joint0"));
}

// ── biomed ─────────────────────────────────────────────────────────────────

#[test]
fn chicken_physiology_has_named_systems_and_multiway_processes() {
    let b = build_and_lower(WorkloadFamily::ChickenPhysiology);
    for organ in ["crop", "gizzard", "lung", "heart", "liver", "keel"] {
        assert!(b.source.contains(organ), "missing organ {organ}");
    }
    assert!(b.max_cardinality > 2);
    assert_eq!(b.oracle.units, 5);
}

#[test]
fn chicken_flock_shared_resources_are_multiway_and_seeded() {
    let mk = |seed| WorkloadFamily::ChickenFlockContact {
        birds: 40,
        coops: 4,
        feeders: 4,
        drinkers: 4,
        group: 6,
        windows: 10,
        seed,
    };
    let b = build_and_lower(mk(3));
    assert_eq!(b.oracle.materials, 52);
    assert_eq!(b.oracle.units, 22);
    assert_eq!(b.oracle.incidences, 192);
    assert!(b.max_cardinality > 2);
    let other = build(mk(99)).unwrap();
    assert_ne!(b.source, other.source);
    assert_eq!(b.oracle, other.oracle);
}

// ── metamorphic (whole-snapshot; NOT canonical-IR mutation — F.17 stays out) ─

#[test]
fn metamorphic_steiner_rank_grows_by_formula() {
    let a = build(WorkloadFamily::Steiner { m: 4 }).unwrap().oracle;
    let b = build(WorkloadFamily::Steiner { m: 5 }).unwrap().oracle;
    assert_eq!(a.materials, 15);
    assert_eq!(b.materials, 31);
    assert_eq!(b.units, 31 * 30 / 6);
}

#[test]
fn metamorphic_sunflower_petal_sweep() {
    let base = build(WorkloadFamily::Sunflower {
        core: 3,
        petal: 4,
        petals: 10,
    })
    .unwrap()
    .oracle;
    let more = build(WorkloadFamily::Sunflower {
        core: 3,
        petal: 4,
        petals: 11,
    })
    .unwrap()
    .oracle;
    assert_eq!(more.materials - base.materials, 4);
    assert_eq!(more.units - base.units, 1);
    assert_eq!(more.incidences - base.incidences, 7);
}

#[test]
fn metamorphic_vlan_add_one_vlan() {
    let a = build(WorkloadFamily::Vlan {
        switches: 8,
        hosts_per_switch: 4,
        vlans: 4,
        power_domains: 2,
    })
    .unwrap()
    .oracle;
    let b = build(WorkloadFamily::Vlan {
        switches: 8,
        hosts_per_switch: 4,
        vlans: 5,
        power_domains: 2,
    })
    .unwrap()
    .oracle;
    assert_eq!(a.materials, b.materials);
    assert_eq!(a.incidences, b.incidences);
    assert_eq!(b.units - a.units, 1);
}

// ── parameter validation & checked-arithmetic boundary (D) ─────────────────

#[test]
fn degenerate_parameters_are_typed_errors_not_panics() {
    use WorkloadFamily as F;
    assert!(matches!(
        build(F::Steiner { m: 2 }),
        Err(WorkloadError::InvalidParameter(msg)) if msg.contains("steiner rank")
    ));
    assert!(matches!(
        build(F::Sunflower {
            core: 0,
            petal: 0,
            petals: 0
        }),
        Err(WorkloadError::InvalidParameter(_))
    ));
    assert!(matches!(
        build(F::RandomUniform {
            nodes: 3,
            edges: 2,
            card: 5,
            seed: 0
        }),
        Err(WorkloadError::InvalidParameter(_))
    ));
    assert!(matches!(
        build(F::Vlan {
            switches: 2,
            hosts_per_switch: 1,
            vlans: 2,
            power_domains: 1
        }),
        Err(WorkloadError::InvalidParameter(_))
    ));
}

#[test]
fn oversized_workloads_are_rejected_before_allocation() {
    use WorkloadFamily as F;
    let huge = build(F::Sunflower {
        core: 1,
        petal: usize::MAX / 2,
        petals: 4,
    });
    assert!(
        matches!(
            huge,
            Err(WorkloadError::ArithmeticOverflow(_)) | Err(WorkloadError::TooLarge { .. })
        ),
        "usize::MAX-adjacent must not allocate: {huge:?}"
    );
    let big = build(F::RandomUniform {
        nodes: 10,
        edges: 20_000_000,
        card: 5,
        seed: 0,
    });
    assert!(
        matches!(big, Err(WorkloadError::TooLarge { .. })),
        "{big:?}"
    );
}

// ── scale classification (L) ───────────────────────────────────────────────

#[test]
fn scale_of_classifies_by_incidence_count() {
    assert_eq!(scale_of(50), WorkloadScale::Tiny);
    assert_eq!(scale_of(500), WorkloadScale::Small);
    assert_eq!(scale_of(50_000), WorkloadScale::Medium);
    assert_eq!(scale_of(500_000), WorkloadScale::Large);
    assert_eq!(scale_of(5_000_000), WorkloadScale::Stress);
    let old_large = build(WorkloadFamily::Sunflower {
        core: 8,
        petal: 8,
        petals: 400,
    })
    .unwrap();
    assert_eq!(scale_of(old_large.oracle.incidences), WorkloadScale::Small);
}

// ── fingerprint ────────────────────────────────────────────────────────────

#[test]
fn source_fingerprint_is_stable_and_distinguishing() {
    let a = build(WorkloadFamily::Steiner { m: 4 }).unwrap();
    let b = build(WorkloadFamily::Steiner { m: 4 }).unwrap();
    assert_eq!(source_fingerprint(&a.source), source_fingerprint(&b.source));
    let c = build(WorkloadFamily::Steiner { m: 5 }).unwrap();
    assert_ne!(source_fingerprint(&a.source), source_fingerprint(&c.source));
}

// ── run policy (L / F.15) ──────────────────────────────────────────────────

#[test]
fn medium_examples_of_each_category_lower() {
    use WorkloadFamily as F;
    build_and_lower(F::Steiner { m: 6 });
    build_and_lower(F::RandomUniform {
        nodes: 4000,
        edges: 8000,
        card: 4,
        seed: 1,
    });
    build_and_lower(F::Vlan {
        switches: 128,
        hosts_per_switch: 64,
        vlans: 32,
        power_domains: 16,
    });
    build_and_lower(F::Humanoid {
        arm_joints: 12,
        leg_joints: 12,
        sensors_per_joint: 4,
        actuator_redundancy: 3,
    });
    build_and_lower(F::ChickenFlockContact {
        birds: 2000,
        coops: 16,
        feeders: 32,
        drinkers: 32,
        group: 12,
        windows: 400,
        seed: 1,
    });
}

#[test]
fn large_math_random_and_network_examples_lower() {
    use WorkloadFamily as F;
    build_and_lower(F::Steiner { m: 9 }); // 511 pts, 43435 blocks, 130305 inc → large
    build_and_lower(F::RandomUniform {
        nodes: 20_000,
        edges: 40_000,
        card: 5,
        seed: 1,
    }); // 200000 inc
    build_and_lower(F::Vlan {
        switches: 512,
        hosts_per_switch: 512,
        vlans: 128,
        power_domains: 64,
    }); // 262656 inc
}

#[test]
#[ignore = "stress; run: cargo test -p hymeko_pgraph --test workloads_suite stress -- --ignored"]
fn stress_examples_lower() {
    use WorkloadFamily as F;
    build_and_lower(F::Steiner { m: 11 });
    build_and_lower(F::RandomUniform {
        nodes: 100_000,
        edges: 300_000,
        card: 5,
        seed: 1,
    });
}

#[test]
#[ignore = "writes goldens; run: cargo test -p hymeko_pgraph --test workloads_suite regenerate_golden_fixtures -- --ignored"]
fn regenerate_golden_fixtures() {
    for (family, file) in golden_table() {
        let b = build(family).unwrap();
        std::fs::write(workload_golden(file), b.source.as_bytes())
            .unwrap_or_else(|e| panic!("write golden {file}: {e}"));
    }
}
