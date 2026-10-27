//! Deterministic warm-buffer contract, not a timing benchmark.
#[path = "pagerank_support/mod.rs"]
mod support;
use hymeko::ir::incidence::{IncidenceDomain, PreparedIr};
use hymeko::ir::ir::{IncidenceSign, IrBuilder, PortDirection, ValueR};
use hymeko::util::measurement::{AllocationSnapshot, CountingAllocator};
use hymeko_graph::{
    PageRank, PageRankConfig, PageRankModel, PageRankSolver, PageRankWorkspace, RankFlow,
    RankWeights,
};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

struct AllocationFixture;

#[test]
fn warm_automatic_selection_reuses_probe_basis_and_continuation_buffers() {
    let ir = AllocationFixture::hub(17);
    let rank = PageRank::build(&ir, PageRankModel::default()).unwrap();
    let solver = PageRankSolver::Auto(hymeko_graph::PageRankAutoConfig {
        restart: 4,
        ..Default::default()
    });
    let mut workspace = PageRankWorkspace::new();
    let first = rank
        .run_with_solver(PageRankConfig::default(), &mut workspace, None, solver)
        .unwrap();
    let selection = first.automatic.unwrap();
    assert!(selection.gmres_iterations > 0);
    assert!(first.progress.converged);
    let retained = workspace.retained_bytes();
    let before = AllocationSnapshot::current_thread();
    let result = rank
        .run_with_solver(PageRankConfig::default(), &mut workspace, None, solver)
        .unwrap();
    let delta = AllocationSnapshot::current_thread().since(before);
    assert_eq!(result.automatic.unwrap(), selection);
    assert!(result.progress.converged);
    assert_eq!((delta.allocations, delta.bytes), (0, 0));
    assert_eq!(workspace.retained_bytes(), retained);
}

impl AllocationFixture {
    fn hub(seed: u64) -> hymeko::ir::ir::Ir {
        use hymeko::generators::GraphGenerator;
        support::Fixture {
            nodes: 128,
            arcs: 192,
            arity: 32,
            relabel_seed: Some(seed),
            ..support::Fixture::named("hub32_1m")
        }
        .generate()
        .unwrap()
    }
}

#[test]
fn warm_gmres_reuses_basis_rhs_and_output_for_structured_and_directed_models() {
    let ir = AllocationFixture::hub(17);
    let mut directed = support::Case::unsigned(4, &[&[0, 1], &[1, 2], &[2, 0]]);
    for row in &mut directed.arcs {
        row[0].atom.direction = PortDirection::In;
        row[1].atom.direction = PortDirection::Out;
    }
    let directed_ir = directed.build().unwrap();
    let mut workspace = PageRankWorkspace::new();
    for (input, flow) in [
        (&ir, RankFlow::Undirected),
        (&directed_ir, RankFlow::InToOut),
    ] {
        let rank = PageRank::build(
            input,
            PageRankModel {
                flow,
                ..Default::default()
            },
        )
        .unwrap();
        let solver = PageRankSolver::Gmres { restart: 20 };
        let config = PageRankConfig::default();
        rank.run_with_solver(config, &mut workspace, None, solver)
            .unwrap();
        let retained = workspace.retained_bytes();
        let before = AllocationSnapshot::current_thread();
        let result = rank
            .run_with_solver(config, &mut workspace, None, solver)
            .unwrap();
        let delta = AllocationSnapshot::current_thread().since(before);
        assert!(result.progress.converged);
        assert!(result.progress.iterations > 1);
        assert_eq!((delta.allocations, delta.bytes), (0, 0));
        assert_eq!(workspace.retained_bytes(), retained);
    }
}

#[test]
fn parallel_solve_reuses_numeric_capacity_and_lazy_reverse_index() {
    let ir = AllocationFixture::hub(17);
    let rank = PageRank::build(&ir, PageRankModel::default()).unwrap();
    let before_index = rank.retained_bytes();
    let mut workspace = PageRankWorkspace::new();
    workspace.set_workers(4).unwrap();
    rank.run(PageRankConfig::default(), &mut workspace, None)
        .unwrap();
    let retained = (rank.retained_bytes(), workspace.retained_bytes());
    assert!(retained.0 > before_index);
    for _ in 0..3 {
        workspace.set_workers(4).unwrap();
        assert!(
            rank.run(PageRankConfig::default(), &mut workspace, None)
                .unwrap()
                .progress
                .converged
        );
        assert_eq!(
            (rank.retained_bytes(), workspace.retained_bytes()),
            retained
        );
    }
    let empty = IrBuilder::build(support::Case::unsigned(0, &[])).unwrap();
    let rank = PageRank::build(&empty, PageRankModel::default()).unwrap();
    assert!(
        rank.run(PageRankConfig::default(), &mut workspace, None)
            .unwrap()
            .scores
            .is_empty()
    );
}

#[test]
fn first_encounter_range_solve_reuses_all_warm_buffers() {
    for seed in [17, 29, 43] {
        let ir = AllocationFixture::hub(seed);
        let rank = PageRank::build(&ir, PageRankModel::default()).unwrap();
        assert_eq!(
            rank.plan_stats().unwrap().ordering,
            hymeko::ir::incidence::IncidenceOrder::FirstEncounter
        );
        let mut workspace = PageRankWorkspace::new();
        rank.run(PageRankConfig::default(), &mut workspace, None)
            .unwrap();
        let before = AllocationSnapshot::current_thread();
        let result = rank
            .run(PageRankConfig::default(), &mut workspace, None)
            .unwrap();
        let delta = AllocationSnapshot::current_thread().since(before);
        assert!(result.progress.converged);
        assert_eq!((delta.allocations, delta.bytes), (0, 0));
    }
}

#[test]
fn virtual_projection_stays_compact_for_all_rank_policies_and_warm_solves() {
    let rows: Vec<Vec<usize>> = (0..16).map(|i| (i..i + 32).collect()).collect();
    let borrowed: Vec<&[usize]> = rows.iter().map(Vec::as_slice).collect();
    let mut case = support::Case::unsigned(64, &borrowed);
    for row in &mut case.arcs {
        for (slot, reference) in row.iter_mut().enumerate() {
            reference.sign = if slot == 0 {
                IncidenceSign::Minus
            } else {
                IncidenceSign::Plus
            };
            reference.atom.direction = if slot == 0 {
                PortDirection::In
            } else {
                PortDirection::Out
            };
            reference.atom.weights = Some(vec![ValueR::Num(1.0 + (slot % 3) as f64)]);
        }
    }
    let prepared = PreparedIr::new(case.build().unwrap()).unwrap();
    let projection = prepared
        .incidences()
        .prepare_projection(IncidenceDomain::Nodes)
        .unwrap();
    let compact_bytes = projection.retained_bytes();
    assert!(compact_bytes < (16 + 1 + 16 * 32) * size_of::<usize>());
    for flow in [
        RankFlow::Undirected,
        RankFlow::MinusToPlus,
        RankFlow::InToOut,
    ] {
        for weights in [RankWeights::Unit, RankWeights::Component(0)] {
            let rank =
                PageRank::from_projection(projection.clone(), PageRankModel { flow, weights })
                    .unwrap();
            let mut workspace = PageRankWorkspace::new();
            rank.run(PageRankConfig::default(), &mut workspace, None)
                .unwrap();
            let before = AllocationSnapshot::current_thread();
            let result = rank
                .run(PageRankConfig::default(), &mut workspace, None)
                .unwrap();
            let delta = AllocationSnapshot::current_thread().since(before);
            assert!(result.progress.converged);
            assert_eq!(delta.allocations, 0);
            assert_eq!(delta.bytes, 0);
            assert_eq!(
                projection.retained_bytes(),
                compact_bytes,
                "rank must not request expanded targets"
            );
        }
    }
}

#[test]
fn unit_preparation_from_shared_coordinates_needs_only_its_two_output_buffers() {
    let ir = support::Case::unsigned(5, &[&[], &[0, 1, 1], &[1, 2], &[2, 3]])
        .build()
        .unwrap();
    let prepared = PreparedIr::new(ir).unwrap();
    let projection = prepared
        .incidences()
        .prepare_projection(IncidenceDomain::Nodes)
        .unwrap();
    let before = AllocationSnapshot::current_thread();
    let rank = PageRank::from_projection(projection, PageRankModel::default()).unwrap();
    let delta = AllocationSnapshot::current_thread().since(before);
    assert_eq!(delta.allocations, 2);
    assert_eq!(
        delta.bytes,
        ((rank.node_count() + prepared.ir().arcs.len()) * size_of::<f64>()) as u64
    );
}

#[test]
fn cached_projection_and_prepared_signed_operator_allocate_nothing_when_warm() {
    let mut input_case = support::Case::unsigned(4, &[&[0, 1, 1], &[], &[1, 2]]);
    for reference in input_case.arcs.iter_mut().flatten() {
        reference.sign = IncidenceSign::Plus;
    }
    let ir = input_case.build().unwrap();
    let prepared = PreparedIr::new(ir).unwrap();
    let view = prepared.incidences();
    let plan = view.prepare_operator().unwrap();
    let input = vec![1.0; prepared.ir().decl_count()];
    let mut output = vec![0.0; input.len()];
    let mut arc_values = vec![0.0; prepared.ir().arcs.len()];
    let projection = view.prepare_projection(IncidenceDomain::Nodes).unwrap();
    plan.star_operator(&input, &mut output, &mut arc_values);
    let before = AllocationSnapshot::current_thread();
    let reused = view.prepare_projection(IncidenceDomain::Nodes).unwrap();
    let cloned = projection.clone();
    plan.node_to_arc(&input, &mut arc_values);
    plan.star_operator(&input, &mut output, &mut arc_values);
    let delta = AllocationSnapshot::current_thread().since(before);
    assert_eq!(delta.allocations, 0);
    assert_eq!(delta.bytes, 0);
    assert_eq!(reused.incidence_count(), cloned.incidence_count());
    assert_eq!(&output[..4], &[3.0, 8.0, 2.0, 0.0]);
}

#[test]
fn warm_solve_allocates_nothing_for_all_routes_and_weight_modes() {
    let mut input = support::Case::unsigned(4, &[&[0, 1, 2], &[2, 3]]);
    for arc in &mut input.arcs {
        for (slot, r) in arc.iter_mut().enumerate() {
            r.atom.weights = Some(vec![ValueR::Num(2.0)]);
            r.sign = if slot == 0 {
                IncidenceSign::Minus
            } else {
                IncidenceSign::Plus
            };
            r.atom.direction = if slot == 0 {
                PortDirection::In
            } else {
                PortDirection::Out
            };
        }
    }
    let ir = input.build().unwrap();
    for (flow, weights) in [
        RankFlow::Undirected,
        RankFlow::MinusToPlus,
        RankFlow::InToOut,
    ]
    .into_iter()
    .flat_map(|flow| [RankWeights::Unit, RankWeights::Component(0)].map(|weights| (flow, weights)))
    {
        let rank = PageRank::build(&ir, PageRankModel { flow, weights }).unwrap();
        let mut workspace = PageRankWorkspace::new();
        rank.run(
            PageRankConfig::default(),
            &mut workspace,
            Some(&[1.0, 2.0, 3.0, 4.0]),
        )
        .unwrap();
        let before = AllocationSnapshot::current_thread();
        let result = rank
            .run(
                PageRankConfig::default(),
                &mut workspace,
                Some(&[1.0, 2.0, 3.0, 4.0]),
            )
            .unwrap();
        let delta = AllocationSnapshot::current_thread().since(before);
        assert!(result.progress.converged);
        assert!(
            result.progress.iterations > 1,
            "exercise a real iterative solve"
        );
        assert_eq!(delta.allocations, 0);
        assert_eq!(delta.bytes, 0);
    }
}
