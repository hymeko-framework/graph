//! Analytical checks for the bounded batch contract; no alternate learner or clocks.
use hymeko_graph::{
    SignedGraph,
    neural_gas::{BatchGas, BatchGasConfig, GasError},
    traversal::{BfsScratch, Csr, bfs_distances},
};

#[path = "direct_ir_common/mod.rs"]
mod source_fixture;

fn graph(n: u32, pairs: &[(u32, u32)]) -> SignedGraph {
    SignedGraph {
        n_nodes: n,
        edges: pairs.to_vec(),
        signs: vec![1; pairs.len()],
    }
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-10, "{a} != {b}");
}

fn same(a: &BatchGas, b: &BatchGas) {
    assert_eq!(a.positions(), b.positions());
    assert_eq!(a.errors(), b.errors());
    assert_eq!(a.graph().n_nodes, b.graph().n_nodes);
    assert_eq!(a.graph().edges, b.graph().edges);
    assert_eq!(a.graph().signs, b.graph().signs);
}

fn pair(config: BatchGasConfig) -> BatchGas {
    BatchGas::new(&[[0., 0., 0.], [4., 0., 0.]], &graph(2, &[(0, 1)]), config).unwrap()
}

#[test]
fn hand_calculated_separate_normalizers_euclidean_error_and_midpoint_growth() {
    let config = BatchGasConfig {
        max_nodes: 3,
        ..Default::default()
    };
    let mut model = pair(config);
    let receipt = model.step(&[[0., 0., 0.], [2., 0., 0.]]).unwrap();
    let denominator = 2. + config.epsilon;
    // Both points select prototype 0, including the distance tie at x=2.
    near(model.positions()[0][0], 1. / denominator);
    near(model.positions()[1][0], 4. - 0.6 / denominator);
    near(model.positions()[2][0], 2. + 0.2 / denominator);
    // alpha*(0+2) * delta * rho, not alpha*(0+4).
    assert_eq!(model.errors(), &[0.25, 0., 0.125]);
    assert_eq!(model.graph().edges, vec![(0, 2), (1, 2)]);
    assert_eq!(receipt.old_to_new, vec![Some(0), Some(1)]);
    assert_eq!(receipt.insertion_parents, vec![[0, 1]]);
    assert_eq!(
        (
            receipt.evaluated_pairs,
            receipt.nodes_before,
            receipt.nodes_after,
            receipt.removed,
            receipt.inserted,
            receipt.edges_after
        ),
        (4, 2, 3, 0, 1, 2)
    );
}

#[test]
fn neighbors_come_from_the_old_graph_then_winner_edges_replace_it() {
    let config = BatchGasConfig {
        growth_quantile: 1.,
        ..Default::default()
    };
    let mut model = BatchGas::new(
        &[[0., 0., 0.], [4., 0., 0.], [10., 0., 0.]],
        &graph(3, &[(0, 2)]),
        config,
    )
    .unwrap();
    let receipt = model.step(&[[9., 0., 0.]]).unwrap();
    // Winner is old 2, second is old 1; old 1 was NOT a pre-batch neighbor.
    assert_eq!(model.positions()[0], [4., 0., 0.]);
    near(model.positions()[1][0], 10. - 0.5 / (1. + config.epsilon));
    assert_eq!(receipt.old_to_new, vec![None, Some(0), Some(1)]);
    assert_eq!(model.graph().edges, vec![(0, 1)]);
}

#[test]
fn synchronous_updates_use_the_pre_batch_positions() {
    let config = BatchGasConfig {
        growth_quantile: 1.,
        ..Default::default()
    };
    let mut model = pair(config);
    model.step(&[[4., 0., 0.], [0., 0., 0.]]).unwrap();
    near(model.positions()[0][0], 0.4 / (1. + config.epsilon));
    near(model.positions()[1][0], 4. - 0.4 / (1. + config.epsilon));
    assert_eq!(model.errors(), &[0., 0.]);
}

#[test]
fn compaction_keeps_provenance_and_graph_is_consumed_by_existing_bfs() {
    let mut model = BatchGas::new(
        &[[0., 0., 0.], [100., 0., 0.], [10., 0., 0.], [14., 0., 0.]],
        &graph(4, &[(0, 2), (2, 3), (1, 3)]),
        BatchGasConfig::default(),
    )
    .unwrap();
    let receipt = model.step(&[[10., 0., 0.], [14., 0., 0.]]).unwrap();
    assert_eq!(receipt.old_to_new, vec![None, None, Some(0), Some(1)]);
    assert_eq!(receipt.removed, 2);
    let csr = Csr::from_graph(model.graph());
    let mut scratch = BfsScratch::with_capacity(model.graph().n_nodes);
    assert_eq!(bfs_distances(&csr, &mut scratch, 0, 5), 2);
    assert_eq!(scratch.dist, vec![0, 1]);
}

#[test]
fn growth_budget_and_strict_quantile_control_insertion() {
    let points = [[0., 0., 0.], [2., 0., 0.]];
    for (cap, quantile, expected) in [(2, 0., 2), (3, 0.85, 3), (20, 1., 2)] {
        let mut model = pair(BatchGasConfig {
            max_nodes: cap,
            growth_quantile: quantile,
            ..Default::default()
        });
        model.step(&points).unwrap();
        assert_eq!(model.positions().len(), expected);
    }
    // Equal errors imply no strict quantile exceedance, even at p=0.
    let mut model = pair(BatchGasConfig {
        growth_quantile: 0.,
        ..Default::default()
    });
    let receipt = model.step(&[[1., 0., 0.], [3., 0., 0.]]).unwrap();
    assert_eq!(receipt.inserted, 0);
    assert_eq!(model.errors(), &[0.25, 0.25]);
}

#[test]
fn budgets_and_bad_batch_inputs_leave_every_state_component_unchanged() {
    let mut model = pair(BatchGasConfig {
        max_distance_evaluations: 4,
        ..Default::default()
    });
    let original = model.clone();
    assert_eq!(
        model.step(&[[0., 0., 0.]; 3]),
        Err(GasError::BudgetExceeded {
            required: 6,
            limit: 4
        })
    );
    for points in [
        vec![],
        vec![[f64::NAN, 0., 0.]],
        vec![[f64::INFINITY, 0., 0.]],
        vec![[1_000_001., 0., 0.]],
        vec![[0., 0., 0.]; 65537],
    ] {
        assert_eq!(model.step(&points), Err(GasError::InvalidPoints));
        same(&original, &model);
    }
    assert!(
        model.step(&[[0., 0., 0.]; 2]).is_ok(),
        "budget equality is admitted"
    );
}

#[test]
fn invalid_configuration_is_rejected_before_state_allocation() {
    let invalid: [fn(&mut BatchGasConfig); 12] = [
        |c| c.winner_rate = f64::NAN,
        |c| c.neighbour_rate = -0.1,
        |c| c.winner_rate = 1.,
        |c| c.epsilon = 0.,
        |c| c.epsilon = f64::INFINITY,
        |c| c.growth_discount = 1.1,
        |c| c.error_discount = -1.,
        |c| c.growth_quantile = 1.1,
        |c| c.max_nodes = 1,
        |c| c.max_nodes = 4097,
        |c| c.max_distance_evaluations = 0,
        |c| c.max_distance_evaluations = 16_777_217,
    ];
    for change in invalid {
        let mut config = BatchGasConfig::default();
        change(&mut config);
        assert_eq!(
            BatchGas::new(&[[0.; 3]; 2], &graph(2, &[]), config).unwrap_err(),
            GasError::InvalidConfig
        );
    }
    assert!(
        GasError::InvalidConfig
            .to_string()
            .contains("InvalidConfig")
    );
}

#[test]
fn native_graph_validation_handles_public_mutable_graph_fields() {
    let mut negative = graph(2, &[(0, 1)]);
    negative.signs[0] = -1;
    let mut ragged = graph(2, &[(0, 1)]);
    ragged.signs.clear();
    for bad in [
        graph(3, &[]),
        graph(2, &[(0, 0)]),
        graph(2, &[(0, 2)]),
        graph(2, &[(0, 1), (1, 0)]),
        negative,
        ragged,
        graph(2, &vec![(0, 1); 262145]),
    ] {
        assert_eq!(
            BatchGas::new(&[[0.; 3]; 2], &bad, BatchGasConfig::default()).unwrap_err(),
            GasError::InvalidGraph
        );
    }
    assert!(
        BatchGas::new(
            &[[0.; 3]; 4097],
            &graph(4097, &[]),
            BatchGasConfig::default()
        )
        .is_err()
    );
    let normalized = BatchGas::new(
        &[[0.; 3]; 3],
        &graph(3, &[(2, 1), (1, 0)]),
        BatchGasConfig::default(),
    )
    .unwrap();
    assert_eq!(normalized.graph().edges, vec![(0, 1), (1, 2)]);
}

#[test]
fn coincident_prototypes_have_stable_winner_ties_and_stay_finite() {
    let mut model =
        BatchGas::new(&[[0.; 3]; 4], &graph(4, &[]), BatchGasConfig::default()).unwrap();
    let receipt = model.step(&[[0.; 3]; 5]).unwrap();
    assert_eq!(receipt.old_to_new, vec![Some(0), Some(1), None, None]);
    assert_eq!(receipt.inserted, 0);
    assert_eq!(model.positions(), &[[0.; 3]; 2]);
    assert_eq!(model.errors(), &[0., 0.]);
}

#[test]
fn repeated_steps_replay_and_obey_graph_invariants() {
    let mut a = pair(BatchGasConfig {
        max_nodes: 12,
        ..Default::default()
    });
    let mut b = a.clone();
    for step in 0..12 {
        let points: Vec<_> = (0..30)
            .map(|i| [i as f64 / 7., (i % 3) as f64, step as f64 / 8.])
            .collect();
        assert_eq!(a.step(&points).unwrap(), b.step(&points).unwrap());
        same(&a, &b);
        assert!((2..=12).contains(&a.positions().len()));
        assert!(a.graph().edges.windows(2).all(|w| w[0] < w[1]));
        assert!(
            a.graph()
                .edges
                .iter()
                .all(|&(u, v)| u < v && v < a.graph().n_nodes)
        );
        assert_eq!(a.graph().edges.len(), a.graph().signs.len());
        let csr = Csr::from_graph(a.graph());
        assert!((0..a.graph().n_nodes).all(|n| csr.degree(n) > 0));
    }
}

#[test]
fn translating_and_scaling_the_point_geometry_preserves_connections() {
    let initial = [[0., 0., 0.], [4., 0., 0.]];
    let transformed = initial.map(|p| p.map(|v| v * 2. + 10.));
    for seed in 0..16 {
        let mut rng = hymeko_graph::rand_lcg::Lcg::new(seed);
        let points: Vec<[f64; 3]> = (0..20)
            .map(|_| std::array::from_fn(|_| rng.next_in_range(1024) as f64 / 128. - 2.))
            .collect();
        let shifted: Vec<_> = points.iter().map(|p| p.map(|v| v * 2. + 10.)).collect();
        let mut a =
            BatchGas::new(&initial, &graph(2, &[(0, 1)]), BatchGasConfig::default()).unwrap();
        let mut b = BatchGas::new(
            &transformed,
            &graph(2, &[(0, 1)]),
            BatchGasConfig::default(),
        )
        .unwrap();
        assert_eq!(a.step(&points).unwrap(), b.step(&shifted).unwrap());
        assert_eq!(a.graph().edges, b.graph().edges);
        for (x, y) in a
            .positions()
            .iter()
            .flatten()
            .zip(b.positions().iter().flatten())
        {
            near(*y, 2. * x + 10.);
        }
        for (x, y) in a.errors().iter().zip(b.errors()) {
            near(*y, 2. * x);
        }
    }
}

#[test]
fn core_compiled_seed_can_feed_the_operator_without_transformation() {
    use hymeko::ir::ir::ValueR;
    let world = source_fixture::compile(
        "cloud {} seeds { a { position [0.0,0.0,0.0]; } b { position [4.0,0.0,0.0]; } @neighbors { (~ a, ~ b); } }",
    );
    let ir = world.ir();
    let seed_ids = [world.node("a"), world.node("b")].map(|id| ir.nodes[id.raw()].decl);
    // Fixture reading only: one position-valued child per explicitly chosen seed.
    // This is not an independent semantic query or inheritance implementation.
    let positions: Vec<[f64; 3]> = seed_ids
        .iter()
        .map(|id| {
            let attribute = ir.decl_children(*id).next().unwrap();
            let Some(ValueR::List(values)) = &ir.decl_nodes[attribute.raw()].anno.value else {
                panic!("numeric fixture position");
            };
            std::array::from_fn(|axis| match values[axis] {
                ValueR::Num(v) => v,
                _ => panic!("numeric fixture coordinate"),
            })
        })
        .collect();
    let arc = &ir.arcs[world.edge_arc("neighbors").raw()];
    let endpoints: Vec<_> = arc
        .refs
        .iter()
        .map(|r| seed_ids.iter().position(|d| *d == r.target()).unwrap() as u32)
        .collect();
    let mut model = BatchGas::new(
        &positions,
        &graph(2, &[(endpoints[0], endpoints[1])]),
        BatchGasConfig {
            max_nodes: 3,
            ..Default::default()
        },
    )
    .unwrap();
    let receipt = model.step(&[[0., 0., 0.], [2., 0., 0.]]).unwrap();
    assert_eq!(receipt.insertion_parents, vec![[0, 1]]);
    let csr = Csr::from_graph(model.graph());
    let mut scratch = BfsScratch::with_capacity(3);
    assert_eq!(bfs_distances(&csr, &mut scratch, 0, 4), 3);
    assert_eq!(scratch.dist, vec![0, 2, 1]);
    // The authored seed IR is retained and unchanged; runtime prototype state is separate.
    assert_eq!(
        ir.arcs[world.edge_arc("neighbors").raw()].refs[0].target(),
        seed_ids[0]
    );
}

#[test]
fn linear_quantile_and_maximum_error_ties_grow_in_a_stable_order() {
    let coordinates: Vec<_> = [0., 4., 8., 12.].map(|x| [x, 0., 0.]).into();
    let config = BatchGasConfig {
        growth_quantile: 0.5,
        max_nodes: 6,
        ..Default::default()
    };
    let mut model =
        BatchGas::new(&coordinates, &graph(4, &[(0, 1), (1, 2), (2, 3)]), config).unwrap();
    let receipt = model
        .step(&[[1., 0., 0.], [3., 0., 0.], [8., 0., 0.], [12., 0., 0.]])
        .unwrap();
    // Before growth: errors [.25,.25,0,0], interpolated median .125, so two insertions.
    // The second parent selection sees the first insertion and discounted parents.
    assert_eq!(receipt.insertion_parents, vec![[0, 1], [0, 4]]);
    assert_eq!(receipt.nodes_after, 6);
    near(model.errors()[5], 0.0625);
}

#[test]
fn separated_samples_can_remove_a_bridge_without_inventing_reachability() {
    let coordinates = [[0., 0., 0.], [1., 0., 0.], [10., 0., 0.], [11., 0., 0.]];
    let mut model = BatchGas::new(
        &coordinates,
        &graph(4, &[(0, 1), (1, 2), (2, 3)]),
        BatchGasConfig {
            growth_quantile: 1.,
            ..Default::default()
        },
    )
    .unwrap();
    model.step(&coordinates).unwrap();
    let components =
        hymeko_graph::traversal::count_connected_components(&Csr::from_graph(model.graph()));
    assert_eq!(components, 2);
    assert_eq!(model.graph().edges, vec![(0, 1), (2, 3)]);
}
