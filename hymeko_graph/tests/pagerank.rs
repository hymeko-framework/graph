//! Independent numerical validation of the canonical PageRank owner.
//! DenseOracle is a small, test-only linear-system oracle derived directly from
//! P(u,v)=sum_e a(u,e)/d(u)*b(v,e)/delta(e), Zhou et al. NIPS 2006 section 4,
//! and r=(1-alpha)p+alpha P^T r. It never calls the production transition or
//! iteration kernel; no benchmark/runtime can select it as an implementation.

#[path = "pagerank_support/mod.rs"]
mod support;
use hymeko::common::ids::{DeclId, NodeId};
use hymeko::generators::{GraphGenerator, SyntheticGenerator};
use hymeko::ir::incidence::{IncidenceDomain, PreparedIr, build_incidence_index};
use hymeko::ir::ir::{IncidenceSign, Ir, IrBuilder, PortDirection, ReferenceDraft, ValueR};
use hymeko_graph::rand_lcg::Lcg;
use hymeko_graph::{
    PageRank, PageRankConfig, PageRankError, PageRankModel, PageRankWorkspace, RankFlow,
    RankWeights, TraversalWorkspace, connected_components,
};
use support::{Case, Fixture, Membership};

#[test]
fn parallel_workers_preserve_canonical_steps_for_all_routes_and_weights() {
    let mut case = Case::unsigned(6, &[&[], &[0, 1, 1], &[1, 2], &[2, 3, 4], &[4], &[5]]);
    for (row, references) in case.arcs.iter_mut().enumerate() {
        for (slot, reference) in references.iter_mut().enumerate() {
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
            reference.atom.weights = Some(vec![ValueR::Num(if slot % 2 == 0 {
                0.0
            } else {
                (row + 1) as f64
            })]);
        }
    }
    let ir = case.build().unwrap();
    let personalization = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    for flow in [
        RankFlow::Undirected,
        RankFlow::MinusToPlus,
        RankFlow::InToOut,
    ] {
        for weights in [RankWeights::Unit, RankWeights::Component(0)] {
            let rank = PageRank::build(&ir, PageRankModel { flow, weights }).unwrap();
            assert!(rank.plan_stats().is_none());
            let mut workspace = PageRankWorkspace::new();
            let expected: Vec<_> = [1, 7, 200]
                .into_iter()
                .map(|cap| {
                    let config = PageRankConfig {
                        max_iterations: cap,
                        ..Default::default()
                    };
                    let result = rank
                        .run(config, &mut workspace, Some(&personalization))
                        .unwrap();
                    (
                        config,
                        result
                            .scores
                            .iter()
                            .map(|v| v.to_bits())
                            .collect::<Vec<_>>(),
                        result.progress,
                    )
                })
                .collect();
            for workers in [2, 4] {
                workspace.set_workers(workers).unwrap();
                assert_eq!(workspace.worker_count(), workers);
                for (config, scores, progress) in &expected {
                    let result = rank
                        .run(*config, &mut workspace, Some(&personalization))
                        .unwrap();
                    assert_eq!(
                        result
                            .scores
                            .iter()
                            .map(|v| v.to_bits())
                            .collect::<Vec<_>>(),
                        *scores
                    );
                    assert_eq!(result.progress, *progress);
                }
            }
            workspace.set_workers(1).unwrap();
            assert_eq!(workspace.worker_count(), 1);
            assert_eq!(
                workspace.set_workers(0),
                Err(PageRankError::InvalidWorkers { workers: 0 })
            );
            assert_eq!(workspace.worker_count(), 1);
        }
    }
}

struct DenseOracle;
impl DenseOracle {
    fn rates(reference: &ReferenceDraft, model: PageRankModel) -> (f64, f64) {
        let weight = match model.weights {
            RankWeights::Unit => 1.0,
            RankWeights::Component(index) => match &reference.atom.weights {
                None => 1.0,
                Some(values) => match values[index] {
                    ValueR::Num(v) => v,
                    _ => panic!("numeric oracle input"),
                },
            },
        };
        let (source, destination) = match model.flow {
            RankFlow::Undirected => (true, true),
            RankFlow::MinusToPlus => (
                reference.sign == IncidenceSign::Minus,
                reference.sign == IncidenceSign::Plus,
            ),
            RankFlow::InToOut => (
                reference.atom.direction == PortDirection::In,
                reference.atom.direction == PortDirection::Out,
            ),
        };
        (
            if source { weight } else { 0.0 },
            if destination { weight } else { 0.0 },
        )
    }
    fn solve(case: &Case, model: PageRankModel, alpha: f64, teleport: &[f64]) -> Vec<f64> {
        let n = case.nodes;
        if n == 0 {
            return vec![];
        }
        let mut transition = vec![vec![0.0; n]; n];
        let mut degree = vec![0.0; n];
        for arc in &case.arcs {
            let destinations: f64 = arc.iter().map(|r| Self::rates(r, model).1).sum();
            if destinations > 0.0 {
                for r in arc {
                    degree[r.target().raw()] += Self::rates(r, model).0;
                }
            }
        }
        for arc in &case.arcs {
            let destinations: f64 = arc.iter().map(|r| Self::rates(r, model).1).sum();
            if destinations == 0.0 {
                continue;
            }
            for source in arc {
                let u = source.target().raw();
                if degree[u] == 0.0 {
                    continue;
                }
                for destination in arc {
                    transition[u][destination.target().raw()] += Self::rates(source, model).0
                        / degree[u]
                        * Self::rates(destination, model).1
                        / destinations;
                }
            }
        }
        let p_sum: f64 = teleport.iter().sum();
        let p: Vec<_> = teleport.iter().map(|v| v / p_sum).collect();
        for (row, &degree) in transition.iter_mut().zip(&degree) {
            if degree == 0.0 {
                row.copy_from_slice(&p);
            }
        }
        let mut system = vec![vec![0.0; n + 1]; n];
        for (v, row) in system.iter_mut().enumerate() {
            for (u, transition) in transition.iter().enumerate() {
                row[u] = if u == v { 1.0 } else { 0.0 } - alpha * transition[v];
            }
            row[n] = (1.0 - alpha) * p[v];
        }
        // Partial-pivoted Gaussian elimination, independent of power iteration.
        for column in 0..n {
            let pivot = (column..n)
                .max_by(|&a, &b| system[a][column].abs().total_cmp(&system[b][column].abs()))
                .unwrap();
            system.swap(pivot, column);
            let divisor = system[column][column];
            assert!(divisor.abs() > 1e-14);
            for value in &mut system[column][column..] {
                *value /= divisor;
            }
            let pivot_values = system[column][column..].to_vec();
            for (index, row) in system.iter_mut().enumerate() {
                if index == column {
                    continue;
                }
                let factor = row[column];
                for (entry, &pivot) in row[column..].iter_mut().zip(&pivot_values) {
                    *entry -= factor * pivot;
                }
            }
        }
        system.into_iter().map(|row| row[n]).collect()
    }
}

struct Verification;
impl Verification {
    fn compare(case: &Case, model: PageRankModel, teleport: &[f64]) {
        let ir = case.clone().build().unwrap();
        let rank = PageRank::build(&ir, model).unwrap();
        let config = PageRankConfig {
            tolerance: 1e-12,
            ..Default::default()
        };
        let expected = DenseOracle::solve(case, model, config.damping, teleport);
        let mut workspace = PageRankWorkspace::new();
        let result = rank.run(config, &mut workspace, Some(teleport)).unwrap();
        assert!(result.progress.converged);
        assert_eq!(rank.node_count(), case.nodes);
        assert_eq!(
            rank.incidence_count(),
            case.arcs.iter().map(Vec::len).sum::<usize>()
        );
        let error: f64 = result
            .scores
            .iter()
            .zip(&expected)
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(
            error <= result.progress.error_bound_l1 + 2e-13,
            "error {error}, bound {:?}",
            result.progress
        );
        assert!(result.scores.iter().all(|v| v.is_finite() && *v >= 0.0));
        if case.nodes != 0 {
            assert!((result.scores.iter().sum::<f64>() - 1.0).abs() < 2e-13);
        }
        assert_eq!(result.score(NodeId::new(case.nodes)), None);
        let bits: Vec<_> = result.scores.iter().map(|s| s.to_bits()).collect();
        let repeated = rank.run(config, &mut workspace, Some(teleport)).unwrap();
        assert_eq!(
            bits,
            repeated
                .scores
                .iter()
                .map(|s| s.to_bits())
                .collect::<Vec<_>>()
        );
        // All independent oracle cases also exercise the shared cached Core route.
        let index = build_incidence_index(&ir).unwrap();
        let projection = index.prepare_projection(IncidenceDomain::Nodes).unwrap();
        let shared = PageRank::from_projection(projection, model).unwrap();
        let cached = shared.run(config, &mut workspace, Some(teleport)).unwrap();
        assert_eq!(
            bits,
            cached
                .scores
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn one_shared_projection_supports_independent_direction_and_weight_policies() {
    let input = DirectedCase::input(RankFlow::MinusToPlus);
    let prepared = PreparedIr::new(input.clone().build().unwrap()).unwrap();
    let projection = prepared
        .incidences()
        .prepare_projection(IncidenceDomain::Nodes)
        .unwrap();
    let personalization = [1.0, 3.0, 0.0, 0.0, 0.0];
    let config = PageRankConfig {
        tolerance: 1e-12,
        ..Default::default()
    };
    for flow in [
        RankFlow::Undirected,
        RankFlow::MinusToPlus,
        RankFlow::InToOut,
    ] {
        for weights in [RankWeights::Unit, RankWeights::Component(0)] {
            let model = PageRankModel { flow, weights };
            let rank = PageRank::from_projection(projection.clone(), model).unwrap();
            let expected = DenseOracle::solve(&input, model, config.damping, &personalization);
            let mut workspace = PageRankWorkspace::new();
            let result = rank
                .run(config, &mut workspace, Some(&personalization))
                .unwrap();
            assert!(result.progress.converged);
            assert!(
                result
                    .scores
                    .iter()
                    .zip(expected)
                    .map(|(a, b)| (a - b).abs())
                    .sum::<f64>()
                    < 2e-12
            );
        }
    }
    let declarations = prepared
        .incidences()
        .prepare_projection(IncidenceDomain::Declarations)
        .unwrap();
    assert!(matches!(
        PageRank::from_projection(declarations, PageRankModel::default()),
        Err(PageRankError::InvalidProjectionDomain)
    ));
}

/// A true directed hyperarc: two sources, two destinations and an inactive slot.
/// The unused direction axis is deliberately reversed, exposing accidental coupling.
struct DirectedCase;
impl DirectedCase {
    fn input(flow: RankFlow) -> Case {
        let mut case = Case::unsigned(5, &[&[0, 1, 2, 3, 4]]);
        let roles = [
            (IncidenceSign::Minus, PortDirection::In, 1.0),
            (IncidenceSign::Minus, PortDirection::In, 3.0),
            (IncidenceSign::Plus, PortDirection::Out, 2.0),
            (IncidenceSign::Plus, PortDirection::Out, 6.0),
            (IncidenceSign::Neutral, PortDirection::Unspecified, 100.0),
        ];
        for (reference, (sign, direction, weight)) in case.arcs[0].iter_mut().zip(roles) {
            reference.sign = if flow == RankFlow::InToOut {
                match sign {
                    IncidenceSign::Plus => IncidenceSign::Minus,
                    IncidenceSign::Minus => IncidenceSign::Plus,
                    IncidenceSign::Neutral => IncidenceSign::Neutral,
                }
            } else {
                sign
            };
            reference.atom.direction = if flow == RankFlow::MinusToPlus {
                match direction {
                    PortDirection::In => PortDirection::Out,
                    PortDirection::Out => PortDirection::In,
                    PortDirection::Unspecified => PortDirection::Unspecified,
                }
            } else {
                direction
            };
            reference.atom.weights = Some(vec![ValueR::Num(weight)]);
        }
        case
    }

    fn scores(case: &Case, model: PageRankModel) -> Vec<f64> {
        let personalization = [1.0, 3.0, 0.0, 0.0, 0.0];
        Verification::compare(case, model, &personalization);
        let ir = case.clone().build().unwrap();
        let rank = PageRank::build(&ir, model).unwrap();
        let mut workspace = PageRankWorkspace::new();
        rank.run(
            PageRankConfig::default(),
            &mut workspace,
            Some(&personalization),
        )
        .unwrap()
        .scores
        .to_vec()
    }
}

#[test]
fn directed_hyperarc_aggregates_sources_and_distributes_only_to_eligible_targets() {
    let alpha = PageRankConfig::default().damping;
    for flow in [RankFlow::MinusToPlus, RankFlow::InToOut] {
        for weights in [RankWeights::Unit, RankWeights::Component(0)] {
            let model = PageRankModel { flow, weights };
            let scores = DirectedCase::scores(&DirectedCase::input(flow), model);
            let target_fraction = if weights == RankWeights::Unit {
                0.5
            } else {
                0.25
            };
            // The sinks teleport back to the sources; their total masses are
            // 1/(1+alpha) and alpha/(1+alpha), independently of the implementation.
            let expected = [
                0.25 / (1.0 + alpha),
                0.75 / (1.0 + alpha),
                target_fraction * alpha / (1.0 + alpha),
                (1.0 - target_fraction) * alpha / (1.0 + alpha),
                0.0,
            ];
            for (actual, expected) in scores.iter().zip(expected) {
                assert!((actual - expected).abs() < 1e-10);
            }
            assert_eq!(scores[4], 0.0, "inactive slot must receive no graph mass");
        }
    }
}

#[test]
fn source_only_and_target_only_hyperarcs_do_not_steal_or_create_mass() {
    for flow in [RankFlow::MinusToPlus, RankFlow::InToOut] {
        let base = DirectedCase::input(flow);
        let mut extended = base.clone();
        extended
            .arcs
            .push(vec![base.arcs[0][0].clone(), base.arcs[0][1].clone()]);
        extended
            .arcs
            .push(vec![base.arcs[0][2].clone(), base.arcs[0][3].clone()]);
        for weights in [RankWeights::Unit, RankWeights::Component(0)] {
            let model = PageRankModel { flow, weights };
            assert_eq!(
                DirectedCase::scores(&base, model),
                DirectedCase::scores(&extended, model)
            );
        }
    }
}

#[test]
fn scores_use_node_ids_when_declarations_and_records_are_interleaved() {
    use hymeko::common::ids::SymId;
    use hymeko::ir::ir::{AnnoR, ArcPlacement, IrConstruction};
    let mut ir = Ir::default();
    let mut construction = IrConstruction::new(&mut ir);
    let owner = construction
        .append_edge(SymId::new(0), AnnoR::default(), vec![])
        .unwrap();
    let source = construction
        .append_node(SymId::new(1), AnnoR::default(), vec![])
        .unwrap();
    construction
        .append_edge(SymId::new(2), AnnoR::default(), vec![])
        .unwrap();
    let target = construction
        .append_node(SymId::new(3), AnnoR::default(), vec![])
        .unwrap();
    construction
        .append_arc(
            ArcPlacement::under_owner(owner),
            AnnoR::default(),
            vec![
                ReferenceDraft::plain(source, IncidenceSign::Minus),
                ReferenceDraft::plain(target, IncidenceSign::Plus),
            ],
        )
        .unwrap();
    assert_eq!((source.raw(), target.raw()), (1, 3));
    assert_eq!(
        (ir.as_node(source), ir.as_node(target)),
        (Some(NodeId::new(0)), Some(NodeId::new(1)))
    );
    let rank = PageRank::build(
        &ir,
        PageRankModel {
            flow: RankFlow::MinusToPlus,
            ..Default::default()
        },
    )
    .unwrap();
    let mut workspace = PageRankWorkspace::new();
    let result = rank
        .run(PageRankConfig::default(), &mut workspace, None)
        .unwrap();
    assert_eq!(result.scores.len(), 2);
    assert!((result.score(NodeId::new(0)).unwrap() - 1.0 / 2.85).abs() < 1e-10);
    assert!((result.score(NodeId::new(1)).unwrap() - (1.0 - 1.0 / 2.85)).abs() < 1e-10);
}

#[test]
fn exact_single_hyperedge_return_and_duplicate_semantics() {
    for members in [vec![0, 1], vec![0, 0, 1], vec![0, 1, 1, 1]] {
        let case = Case::unsigned(2, &[&members]);
        let ir = case.build().unwrap();
        let rank = PageRank::build(&ir, PageRankModel::default()).unwrap();
        let mut ws = PageRankWorkspace::new();
        let result = rank
            .run(PageRankConfig::default(), &mut ws, Some(&[1.0, 0.0]))
            .unwrap();
        let expected =
            0.15 + 0.85 * members.iter().filter(|&&n| n == 0).count() as f64 / members.len() as f64;
        assert!((result.score(NodeId::new(0)).unwrap() - expected).abs() < 1e-14);
        assert!(result.progress.converged);
    }
}

#[test]
fn directed_pairwise_case_is_classical_pagerank_and_ports_are_independent() {
    let mut case = Case::unsigned(2, &[&[0, 1]]);
    case.arcs[0][0].sign = IncidenceSign::Minus;
    case.arcs[0][1].sign = IncidenceSign::Plus;
    case.arcs[0][0].atom.direction = PortDirection::Out;
    case.arcs[0][1].atom.direction = PortDirection::In;
    let ir = case.clone().build().unwrap();
    for (flow, first) in [
        (RankFlow::MinusToPlus, 1.0 / 2.85),
        (RankFlow::InToOut, 1.0 - 1.0 / 2.85),
    ] {
        let model = PageRankModel {
            flow,
            ..Default::default()
        };
        let rank = PageRank::build(&ir, model).unwrap();
        let mut ws = PageRankWorkspace::new();
        let result = rank.run(PageRankConfig::default(), &mut ws, None).unwrap();
        assert!((result.scores[0] - first).abs() < 1e-10);
        Verification::compare(&case, model, &[1.0, 1.0]);
    }
}

#[test]
fn generated_weighted_directed_and_undirected_models_match_dense_solve() {
    for seed in 0..48 {
        let mut rng = Lcg::new(seed);
        let nodes = 1 + seed as usize % 8;
        let mut case = Case {
            nodes,
            arcs: vec![],
        };
        for _ in 0..(seed as usize % 7) {
            let arity = (rng.next_u64() >> 32) as usize % 10;
            let mut refs = vec![];
            for _ in 0..arity {
                let target = (rng.next_u64() >> 32) as usize % nodes;
                let mut draft = ReferenceDraft::plain(
                    DeclId::new(target),
                    [
                        IncidenceSign::Plus,
                        IncidenceSign::Minus,
                        IncidenceSign::Neutral,
                    ][(rng.next_u64() >> 32) as usize % 3],
                );
                draft.atom.direction = [
                    PortDirection::In,
                    PortDirection::Out,
                    PortDirection::Unspecified,
                ][(rng.next_u64() >> 32) as usize % 3];
                draft.atom.weights = Some(vec![
                    ValueR::Num(((rng.next_u64() >> 32) % 5) as f64),
                    ValueR::Num(2.0),
                ]);
                refs.push(draft);
            }
            case.arcs.push(refs);
        }
        let p: Vec<_> = (0..nodes).map(|i| (i + 1) as f64).collect();
        for flow in [
            RankFlow::Undirected,
            RankFlow::MinusToPlus,
            RankFlow::InToOut,
        ] {
            for weights in [
                RankWeights::Unit,
                RankWeights::Component(0),
                RankWeights::Component(1),
            ] {
                Verification::compare(&case, PageRankModel { flow, weights }, &p);
            }
        }
    }
}

#[test]
fn empty_isolated_singleton_and_no_destination_cases_keep_mass() {
    for case in [
        Case::unsigned(0, &[&[]]),
        Case::unsigned(3, &[]),
        Case::unsigned(3, &[&[], &[0], &[1, 1]]),
    ] {
        Verification::compare(&case, PageRankModel::default(), &vec![1.0; case.nodes]);
        Verification::compare(
            &case,
            PageRankModel {
                flow: RankFlow::MinusToPlus,
                ..Default::default()
            },
            &vec![1.0; case.nodes],
        );
    }
}

#[test]
fn unit_cardinalities_keep_multiplicity_and_ignore_unselected_payloads() {
    let mut case = Case::unsigned(5, &[&[], &[0, 1, 1], &[1, 2], &[2, 2, 3], &[3]]);
    for (slot, reference) in case.arcs.iter_mut().flatten().enumerate() {
        reference.sign = IncidenceSign::Minus;
        reference.atom.direction = PortDirection::Out;
        reference.atom.weights = Some(if slot % 2 == 0 {
            vec![ValueR::Num(f64::NAN)]
        } else {
            vec![]
        });
    }
    Verification::compare(&case, PageRankModel::default(), &[1.0, 2.0, 3.0, 4.0, 5.0]);
}

#[test]
fn cap_exhaustion_is_explicit_and_alpha_zero_is_exact() {
    let case = Case::unsigned(3, &[&[0, 1], &[1, 2]]);
    let ir = case.clone().build().unwrap();
    let rank = PageRank::build(&ir, PageRankModel::default()).unwrap();
    let mut ws = PageRankWorkspace::new();
    let result = rank
        .run(
            PageRankConfig {
                max_iterations: 1,
                tolerance: 1e-15,
                ..Default::default()
            },
            &mut ws,
            Some(&[1.0, 0.0, 0.0]),
        )
        .unwrap();
    assert!(!result.progress.converged);
    assert_eq!(result.progress.iterations, 1);
    let exact = DenseOracle::solve(&case, PageRankModel::default(), 0.85, &[1.0, 0.0, 0.0]);
    assert!(
        result
            .scores
            .iter()
            .zip(exact)
            .map(|(a, b)| (a - b).abs())
            .sum::<f64>()
            <= result.progress.error_bound_l1 + 1e-14
    );
    let result = rank
        .run(
            PageRankConfig {
                damping: 0.0,
                ..Default::default()
            },
            &mut ws,
            Some(&[0.0, 2.0, 0.0]),
        )
        .unwrap();
    assert_eq!(result.scores, &[0.0, 1.0, 0.0]);
    assert_eq!(result.progress.iterations, 0);
    assert!(result.progress.converged);
}

#[test]
fn controls_personalization_and_topology_fail_closed() {
    let ir = Case::unsigned(2, &[&[0, 1]]).build().unwrap();
    let rank = PageRank::build(&ir, PageRankModel::default()).unwrap();
    let mut ws = PageRankWorkspace::new();
    for alpha in [-1.0, 1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            rank.run(
                PageRankConfig {
                    damping: alpha,
                    ..Default::default()
                },
                &mut ws,
                None
            )
            .unwrap_err(),
            PageRankError::InvalidConfig
        );
    }
    for tolerance in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            rank.run(
                PageRankConfig {
                    tolerance,
                    ..Default::default()
                },
                &mut ws,
                None
            )
            .unwrap_err(),
            PageRankError::InvalidConfig
        );
    }
    assert_eq!(
        rank.run(
            PageRankConfig {
                max_iterations: 0,
                ..Default::default()
            },
            &mut ws,
            None
        )
        .unwrap_err(),
        PageRankError::InvalidConfig
    );
    for p in [
        vec![],
        vec![1.0],
        vec![0.0, 0.0],
        vec![-1.0, 2.0],
        vec![f64::NAN, 1.0],
        vec![f64::INFINITY, 1.0],
    ] {
        assert_eq!(
            rank.run(PageRankConfig::default(), &mut ws, Some(&p))
                .unwrap_err(),
            PageRankError::InvalidPersonalization
        );
    }
    let huge = rank
        .run(
            PageRankConfig::default(),
            &mut ws,
            Some(&[f64::MAX, f64::MAX]),
        )
        .unwrap();
    assert_eq!(huge.scores, &[0.5, 0.5]);
    let mut broken = ir.clone();
    broken.arcs[0].refs[0] = broken.arcs[0].refs[0].with_target(DeclId::new(2));
    assert!(matches!(
        PageRank::build(&broken, PageRankModel::default()),
        Err(PageRankError::InvalidTarget { .. })
    ));
    let mut broken = ir;
    broken.nodes[0].decl = DeclId::NONE;
    assert!(matches!(
        PageRank::build(&broken, PageRankModel::default()),
        Err(PageRankError::InvalidNode(_))
    ));
    assert!(!PageRankError::NumericalMass.to_string().is_empty());
}

#[test]
fn selected_weight_vector_component_is_checked_and_subnormal_weights_work() {
    let model = PageRankModel {
        weights: RankWeights::Component(0),
        ..Default::default()
    };
    for weights in [
        vec![],
        vec![ValueR::Num(-1.0)],
        vec![ValueR::Num(f64::NAN)],
        vec![ValueR::Num(f64::INFINITY)],
        vec![ValueR::List(vec![])],
    ] {
        let mut case = Case::unsigned(2, &[&[0, 1]]);
        case.arcs[0][0].atom.weights = Some(weights);
        let ir = case.build().unwrap();
        assert!(matches!(
            PageRank::build(&ir, model),
            Err(PageRankError::InvalidWeight { .. })
        ));
    }
    let mut case = Case::unsigned(2, &[&[0, 1]]);
    for r in &mut case.arcs[0] {
        r.atom.weights = Some(vec![ValueR::Num(f64::from_bits(1))]);
    }
    Verification::compare(&case, model, &[1.0, 2.0]);
    for r in &mut case.arcs[0] {
        r.atom.weights = Some(vec![ValueR::Num(f64::MAX)]);
    }
    assert!(matches!(
        PageRank::build(&case.build().unwrap(), model),
        Err(PageRankError::NumericalMass)
    ));
    Verification::compare(&Case::unsigned(2, &[&[0, 1]]), model, &[1.0, 1.0]);
}

#[test]
fn permutation_and_global_positive_weight_scaling_preserve_the_ranking() {
    let mut case = Case::unsigned(4, &[&[0, 1, 1], &[1, 2], &[2, 3, 0]]);
    for (e, arc) in case.arcs.iter_mut().enumerate() {
        for (slot, r) in arc.iter_mut().enumerate() {
            r.atom.weights = Some(vec![ValueR::Num((e + slot + 1) as f64)]);
        }
    }
    let model = PageRankModel {
        weights: RankWeights::Component(0),
        ..Default::default()
    };
    let mut changed = case.clone();
    changed.arcs.reverse();
    for arc in &mut changed.arcs {
        arc.reverse();
        for r in arc {
            r.atom.target = DeclId::new(3 - r.target().raw());
            if let Some(weights) = &mut r.atom.weights
                && let ValueR::Num(w) = &mut weights[0]
            {
                *w *= 7.0;
            }
        }
    }
    let mut ws = PageRankWorkspace::new();
    let mut results = vec![];
    for (case, p) in [
        (case, [1.0, 2.0, 3.0, 4.0]),
        (changed, [4.0, 3.0, 2.0, 1.0]),
    ] {
        let ir = case.build().unwrap();
        let model = PageRank::build(&ir, model).unwrap();
        results.push(
            model
                .run(PageRankConfig::default(), &mut ws, Some(&p))
                .unwrap()
                .scores
                .to_vec(),
        );
    }
    for (a, b) in results[0].iter().zip(results[1].iter().rev()) {
        assert!((a - b).abs() < 1e-12);
    }
}

#[test]
fn shared_streaming_fixture_uses_core_and_is_seeded_and_bounded() {
    let mut fixture = Fixture {
        nodes: 16,
        arcs: 12,
        arity: 5,
        seed: 7,
        relabel_seed: None,
        membership: Membership::Affine,
    };
    let a = fixture.generate().unwrap();
    let b = fixture.build().unwrap();
    let refs = |ir: &Ir| {
        ir.arcs
            .iter()
            .map(|arc| arc.refs.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(refs(&a), refs(&b));
    assert_eq!(a.nodes.len(), 16);
    assert_eq!(a.arcs.len(), 12);
    for arc in &a.arcs {
        let mut targets: Vec<_> = arc.refs.iter().map(|r| r.target().raw()).collect();
        targets.sort_unstable();
        targets.dedup();
        assert_eq!(targets.len(), 5);
    }
    assert_eq!(fixture.seed(), 7);
    fixture.reseed(8);
    assert_ne!(refs(&a), refs(&fixture.generate().unwrap()));
    let disjoint = Fixture {
        nodes: 15,
        arcs: 3,
        arity: 5,
        membership: Membership::Disjoint,
        ..fixture
    }
    .generate()
    .unwrap();
    assert_eq!(
        disjoint
            .arcs
            .iter()
            .flat_map(|a| &a.refs)
            .map(|r| r.target().raw())
            .collect::<Vec<_>>(),
        (0..15).collect::<Vec<_>>()
    );
    for name in ["rank2_1m", "rank32_1m", "rank1000_1m", "disjoint_1m"] {
        let f = Fixture::named(name);
        assert_eq!(f.arcs * f.arity, 1_000_000);
    }
    let rank = PageRank::build(&a, PageRankModel::default()).unwrap();
    assert!(rank.retained_bytes() > 0);
    let mut ws = PageRankWorkspace::new();
    rank.run(PageRankConfig::default(), &mut ws, None).unwrap();
    assert!(ws.retained_bytes() > 0);
    let empty = Ir::default();
    assert!(
        PageRank::build(&empty, PageRankModel::default())
            .unwrap()
            .run(PageRankConfig::default(), &mut ws, None)
            .unwrap()
            .scores
            .is_empty()
    );
}

#[test]
fn diverse_fixture_strategies_preserve_shape_distinctness_and_known_topology() {
    let base = Fixture {
        nodes: 32,
        arcs: 24,
        arity: 4,
        seed: 19,
        relabel_seed: None,
        membership: Membership::Affine,
    };
    for fixture in [
        Fixture {
            arity: 8,
            membership: Membership::Mixed(&[2, 3, 8]),
            ..base
        },
        Fixture {
            membership: Membership::Hub,
            ..base
        },
        Fixture {
            nodes: 40,
            arcs: 32,
            membership: Membership::Communities {
                groups: 4,
                active: 32,
            },
            ..base
        },
        Fixture {
            arcs: 31,
            arity: 2,
            membership: Membership::Chain,
            ..base
        },
    ] {
        let ir = fixture.generate().unwrap();
        let repeated = fixture.generate().unwrap();
        let mut degrees = vec![0; fixture.nodes];
        let mut rows = Vec::new();
        for (ordinal, arc) in ir.arcs.iter().enumerate() {
            let members: Vec<_> = arc
                .refs
                .iter()
                .map(|r| ir.as_node(r.target()).unwrap().raw())
                .collect();
            let mut distinct = members.clone();
            distinct.sort_unstable();
            distinct.dedup();
            assert_eq!(members.len(), distinct.len());
            assert_eq!(arc.refs, repeated.arcs[ordinal].refs);
            for &node in &members {
                degrees[node] += 1;
            }
            rows.push(members);
        }
        assert_eq!(
            rows.iter().map(Vec::len).sum::<usize>(),
            fixture.incidence_count()
        );
        assert!(!fixture.membership.label().is_empty());
        match fixture.membership {
            Membership::Mixed(widths) => {
                for (ordinal, row) in rows.iter().enumerate() {
                    assert_eq!(row.len(), widths[ordinal % widths.len()]);
                }
            }
            Membership::Hub => {
                assert_eq!(degrees[0], fixture.arcs);
                assert!(
                    degrees[1..]
                        .iter()
                        .all(|&degree| degree < fixture.arcs && degree > 0)
                );
            }
            Membership::Communities { groups, active } => {
                assert!(degrees[active..].iter().all(|&degree| degree == 0));
                assert!(degrees[..active].iter().all(|&degree| degree > 0));
                let index = build_incidence_index(&ir).unwrap();
                let partition =
                    connected_components(&index, &mut TraversalWorkspace::new()).unwrap();
                assert_eq!(partition.count() as usize, groups + fixture.nodes - active);
            }
            Membership::Chain => {
                for (ordinal, row) in rows.iter().enumerate() {
                    assert_eq!(row, &[ordinal, ordinal + 1]);
                }
            }
            _ => unreachable!(),
        }
        let slices: Vec<_> = rows.iter().map(Vec::as_slice).collect();
        Verification::compare(
            &Case::unsigned(fixture.nodes, &slices),
            PageRankModel::default(),
            &vec![1.0; fixture.nodes],
        );
    }
}

#[test]
fn diverse_catalog_dimensions_are_exact_and_invalid_strategies_fail_before_population() {
    for (name, nodes, arcs, incidences) in [
        ("steiner255", 255, 10_795, 32_385),
        ("complete32_3", 32, 4_960, 14_880),
        ("sunflower1024", 32_769, 1_024, 33_792),
        ("mycielski7", 767, 22_196, 44_392),
        ("mixed_1m", 65_536, 31_250, 1_000_000),
        ("hub32_1m", 65_536, 31_250, 1_000_000),
        ("components8_1m", 65_536, 31_250, 1_000_000),
        ("chain65k", 65_536, 65_535, 131_070),
        ("scale_small", 8_192, 4_096, 131_072),
        ("scale_large", 262_144, 125_000, 4_000_000),
    ] {
        let fixture = Fixture::named(name);
        assert_eq!(
            (fixture.nodes, fixture.arcs, fixture.incidence_count()),
            (nodes, arcs, incidences)
        );
    }
    let base = Fixture {
        nodes: 16,
        arcs: 10,
        arity: 4,
        seed: 1,
        relabel_seed: None,
        membership: Membership::Affine,
    };
    for membership in [
        Membership::Mixed(&[]),
        Membership::Mixed(&[5]),
        Membership::Communities {
            groups: 0,
            active: 8,
        },
        Membership::Chain,
    ] {
        assert!(
            std::panic::catch_unwind(|| Fixture { membership, ..base }.incidence_count()).is_err()
        );
    }
}

#[test]
fn structural_range_plan_preserves_dense_solution_and_certifies_convergence() {
    use support::{Fixture, Membership};
    let fixtures = [
        Fixture {
            nodes: 128,
            arcs: 192,
            arity: 32,
            seed: 19,
            relabel_seed: None,
            membership: Membership::Hub,
        },
        Fixture {
            nodes: 128,
            arcs: 192,
            arity: 16,
            seed: 19,
            relabel_seed: None,
            membership: Membership::Communities {
                groups: 4,
                active: 64,
            },
        },
    ];
    for fixture in fixtures.into_iter().flat_map(|fixture| {
        [None, Some(17), Some(29), Some(43)].map(move |relabel_seed| Fixture {
            relabel_seed,
            ..fixture
        })
    }) {
        let ir = fixture.generate().unwrap();
        let case = Case {
            nodes: ir.nodes.len(),
            arcs: ir
                .arcs
                .iter()
                .map(|arc| {
                    arc.refs
                        .iter()
                        .map(|r| {
                            ReferenceDraft::plain(
                                DeclId::new(ir.as_node(r.target()).unwrap().raw()),
                                IncidenceSign::Neutral,
                            )
                        })
                        .collect()
                })
                .collect(),
        };
        let rank = PageRank::build(&ir, PageRankModel::default()).unwrap();
        let plan = rank
            .plan_stats()
            .expect("structured fixture must activate range execution");
        if fixture.relabel_seed.is_some() {
            assert_eq!(
                plan.ordering,
                hymeko::ir::incidence::IncidenceOrder::FirstEncounter
            );
            assert!(plan.ranges < plan.input_ranges);
        }
        assert!(plan.selected_work < plan.direct_work);
        assert!(plan.range_rows > 0);
        assert!(plan.search_states <= 512);
        let p: Vec<_> = (0..case.nodes)
            .map(|i| {
                if i % 3 == 0 {
                    1e-14
                } else {
                    (i % 7 + 1) as f64
                }
            })
            .collect();
        Verification::compare(&case, PageRankModel::default(), &p);
        let reference = DenseOracle::solve(&case, PageRankModel::default(), 0.85, &p);
        let mut parallel = PageRankWorkspace::new();
        let mut parallel_bits = None;
        for workers in [2, 4] {
            parallel.set_workers(workers).unwrap();
            let result = rank
                .run(PageRankConfig::default(), &mut parallel, Some(&p))
                .unwrap();
            assert!(result.progress.converged);
            assert!(
                result
                    .scores
                    .iter()
                    .zip(&reference)
                    .map(|(a, b)| (a - b).abs())
                    .sum::<f64>()
                    < 2e-10
            );
            let bits: Vec<_> = result.scores.iter().map(|v| v.to_bits()).collect();
            if let Some(expected) = &parallel_bits {
                assert_eq!(&bits, expected);
            }
            parallel_bits = Some(bits);
        }
        for cap in [1, 2, 7, 200] {
            let mut workspace = PageRankWorkspace::new();
            let result = rank
                .run(
                    PageRankConfig {
                        max_iterations: cap,
                        tolerance: 1e-12,
                        ..Default::default()
                    },
                    &mut workspace,
                    Some(&p),
                )
                .unwrap();
            assert!(result.scores.iter().all(|v| v.is_finite() && *v >= 0.0));
            let error: f64 = result
                .scores
                .iter()
                .zip(&reference)
                .map(|(a, b)| (a - b).abs())
                .sum();
            assert!(error <= result.progress.error_bound_l1 + 2e-13);
            assert!(result.progress.iterations <= cap);
        }
        let mut ws = PageRankWorkspace::new();
        let result = rank
            .run(
                PageRankConfig {
                    tolerance: 1e-12,
                    ..Default::default()
                },
                &mut ws,
                Some(&p),
            )
            .unwrap();
        let error: f64 = result
            .scores
            .iter()
            .zip(&reference)
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(error <= result.progress.error_bound_l1 + 2e-13);
        let weighted = PageRank::build(
            &ir,
            PageRankModel {
                weights: RankWeights::Component(0),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(weighted.plan_stats().is_none());
    }
}

#[test]
fn mixed_component_kernels_preserve_reversed_and_duplicate_memberships() {
    let mut rows: Vec<Vec<usize>> = (0..96)
        .map(|i| {
            let start = i % 24;
            let mut row: Vec<_> = (start..start + 32).collect();
            if i % 2 == 0 {
                row.reverse();
            }
            if i % 3 == 0 {
                row.push(start);
            }
            row
        })
        .collect();
    rows.extend([vec![64, 65], vec![65, 66], vec![66, 67], vec![67, 64]]);
    let borrowed: Vec<_> = rows.iter().map(Vec::as_slice).collect();
    let case = Case::unsigned(72, &borrowed);
    let ir = case.clone().build().unwrap();
    let rank = PageRank::build(&ir, PageRankModel::default()).unwrap();
    let plan = rank.plan_stats().unwrap();
    assert!(plan.range_rows > 0 && plan.range_rows < rows.len());
    let p: Vec<_> = (0..72)
        .map(|i| {
            if i % 4 == 0 {
                1e-30
            } else {
                (i % 11 + 1) as f64
            }
        })
        .collect();
    Verification::compare(&case, PageRankModel::default(), &p);
}

#[test]
fn regrouped_pagerank_matches_exact_rational_personalized_stationary_distribution() {
    // 64 identical 32-member hyperedges, followed by 32 isolated vertices.
    // p_i=(i+1)/2080, alpha=17/20. The formula below solves the stationary
    // equations in integers; numerator and denominator are exactly representable
    // before the one final f64 division. Independent of either iteration kernel.
    let rows = vec![(0..32).collect::<Vec<_>>(); 64];
    let borrowed: Vec<_> = rows.iter().map(Vec::as_slice).collect();
    let ir = Case::unsigned(64, &borrowed).build().unwrap();
    let rank = PageRank::build(&ir, PageRankModel::default()).unwrap();
    let stats = rank.plan_stats().unwrap();
    assert_eq!(stats.components, 33);
    assert_eq!(stats.isolated_nodes, 32);
    assert_eq!(stats.range_rows, 64);
    let p: Vec<_> = (1..=64).map(|n| n as f64).collect();
    let active: usize = (1..=32).sum();
    let dangling: usize = (33..=64).sum();
    let denominator = 20 * active + 3 * dangling;
    let mut workspace = PageRankWorkspace::new();
    let result = rank
        .run(
            PageRankConfig {
                tolerance: 1e-13,
                ..Default::default()
            },
            &mut workspace,
            Some(&p),
        )
        .unwrap();
    assert!(result.progress.converged);
    let error: f64 = result
        .scores
        .iter()
        .enumerate()
        .map(|(i, &got)| {
            let expected = if i < 32 {
                (3 * 32 * (i + 1) + 17 * active) as f64 / (32 * denominator) as f64
            } else {
                (3 * (i + 1)) as f64 / denominator as f64
            };
            (got - expected).abs()
        })
        .sum();
    assert!(
        error <= result.progress.error_bound_l1 + 4e-15,
        "error {error}; {:?}",
        result.progress
    );
}

#[test]
fn core_generated_fixture_rows_keep_the_canonical_builder_and_permutation() {
    use hymeko::generators::HypergraphGenerator;
    use support::{Fixture, Membership, VertexPermutation};
    let cube = Fixture {
        nodes: 16,
        arcs: 32,
        arity: 2,
        seed: 0,
        relabel_seed: None,
        membership: Membership::Core(HypergraphGenerator::Hypercube { dimension: 4 }),
    };
    for mut fixture in ["steiner255", "complete32_3", "sunflower1024", "mycielski7"]
        .map(Fixture::named)
        .into_iter()
        .chain([cube])
    {
        // The unpermuted topology is the metamorphic reference. Core generator
        // formulas and small independent graph properties are tested separately.
        let original = fixture.generate().unwrap();
        let expected: Vec<Vec<usize>> = original
            .arcs
            .iter()
            .map(|arc| {
                arc.refs
                    .iter()
                    .map(|r| original.as_node(r.target()).unwrap().raw())
                    .collect()
            })
            .collect();
        for seed in [None, Some(17)] {
            fixture.relabel_seed = seed;
            let permutation = VertexPermutation::new(fixture.nodes, seed);
            let ir = fixture.generate().unwrap();
            assert_eq!(
                (ir.nodes.len(), ir.edges.len(), ir.arcs.len()),
                (fixture.nodes, fixture.arcs, fixture.arcs)
            );
            assert_eq!(
                ir.arcs.iter().map(|arc| arc.refs.len()).sum::<usize>(),
                fixture.incidence_count()
            );
            for (arc, members) in ir.arcs.iter().zip(&expected) {
                assert!(ir.as_edge(arc.in_edge).is_some(), "canonical arc owner");
                assert_eq!(
                    arc.refs
                        .iter()
                        .map(|r| ir.as_node(r.target()).unwrap().raw())
                        .collect::<Vec<_>>(),
                    members
                        .iter()
                        .map(|&v| permutation.target(v))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}
