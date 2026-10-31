//! Independent XOR-span oracle on the seven nonzero columns of GF(2)^3.
use hymeko::common::ids::HyperArcId;
use hymeko::ir::{incidence::build_incidence_index, ir::Ir};
use hymeko_graph::matroid::{
    BinaryMatroidWorkspace, BinaryReconstructionOptions, binary_column, binary_weighted_basis,
};
use hymeko_graph::{MatroidError, MatroidRef};
use std::collections::BTreeSet;
#[path = "informed_common/mod.rs"]
mod fixture;

fn model(columns: &[u8]) -> Ir {
    let mut ir = Ir::default();
    fixture::edge_decl(&mut ir); // canonical DeclId and NodeId deliberately differ
    let width = columns
        .iter()
        .map(|v| u8::BITS - v.leading_zeros())
        .max()
        .unwrap_or(0);
    let nodes: Vec<_> = (0..width).map(|_| fixture::node_decl(&mut ir)).collect();
    for &column in columns {
        fixture::push_arc(
            &mut ir,
            (0..width as usize)
                .filter(|i| column & (1 << i) != 0)
                .map(|i| fixture::plus_ref(nodes[i], None))
                .collect(),
        );
    }
    ir
}
fn span_rank(columns: &[u8], subset: usize) -> usize {
    let mut span = BTreeSet::from([0u8]);
    for (i, &column) in columns.iter().enumerate() {
        if subset & (1 << i) != 0 {
            let added: Vec<_> = span.iter().map(|x| x ^ column).collect();
            span.extend(added);
        }
    }
    span.len().ilog2() as usize
}

#[test]
fn every_subset_rank_and_reconstruction_match_exhaustive_xor_span() {
    let columns = [1, 2, 3, 4, 5, 6, 7];
    let ir = model(&columns);
    let index = build_incidence_index(&ir).unwrap();
    let view = MatroidRef::new(&index);
    let mut ws = BinaryMatroidWorkspace::default();
    for subset in 0usize..128 {
        let ground: Vec<_> = (0..7)
            .filter(|i| subset & (1 << i) != 0)
            .map(|i| (HyperArcId::new(i), 1.0))
            .collect();
        let result = binary_weighted_basis(view, &ground, &mut ws).unwrap();
        assert_eq!(result.elements.len(), span_rank(&columns, subset));
        for coordinate in &result.coordinates {
            assert_eq!(
                coordinate
                    .support
                    .iter()
                    .fold(0, |x, a| x ^ columns[a.raw()]),
                columns[coordinate.arc.raw()]
            );
            assert!(
                coordinate
                    .support
                    .iter()
                    .all(|a| result.elements.contains(a))
            );
        }
    }
}

#[test]
fn required_reconstruction_exchanges_reduce_paid_columns_and_keep_every_certificate_exact() {
    use hymeko_graph::matroid::binary_reconstruction_search;
    let columns = [1, 2, 4, 3, 5, 6, 7];
    let ir = model(&columns);
    let index = build_incidence_index(&ir).unwrap();
    let view = MatroidRef::new(&index);
    let ground: Vec<_> = (0..7)
        .map(|i| {
            (
                HyperArcId::new(i),
                if i == 0 {
                    0.
                } else if i < 3 {
                    1.
                } else {
                    10.
                },
            )
        })
        .collect();
    let free = [HyperArcId::new(0)];
    let available: Vec<_> = (1..6).map(HyperArcId::new).collect();
    let required = [HyperArcId::new(6)];
    let mut ws = BinaryMatroidWorkspace::default();
    let result = binary_reconstruction_search(
        view,
        &ground,
        &free,
        &available,
        &required,
        &mut ws,
        BinaryReconstructionOptions {
            beam: 4,
            max_exchanges: 100,
            ..Default::default()
        },
    )
    .unwrap();
    let paid = |b: &hymeko_graph::matroid::BinaryBasis| {
        b.coordinates
            .iter()
            .find(|c| c.arc == required[0])
            .unwrap()
            .support
            .iter()
            .filter(|a| !free.contains(a))
            .count()
    };
    assert_eq!(paid(&result.baseline), 2);
    assert_eq!(paid(&result.candidates[0]), 1);
    assert!(result.exchanges_evaluated > 0);
    for b in &result.candidates {
        for c in &b.coordinates {
            assert_eq!(
                c.support.iter().fold(0, |x, a| x ^ columns[a.raw()]),
                columns[c.arc.raw()]
            );
            assert!(c.support.iter().all(|a| b.elements.contains(a)));
        }
    }
    assert!(
        binary_reconstruction_search(
            view,
            &ground,
            &free,
            &[],
            &required,
            &mut ws,
            BinaryReconstructionOptions {
                beam: 4,
                max_exchanges: 100,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(
        binary_reconstruction_search(
            view,
            &ground,
            &free,
            &available,
            &required,
            &mut ws,
            BinaryReconstructionOptions {
                beam: 0,
                max_exchanges: 100,
                ..Default::default()
            }
        )
        .is_err()
    );
    let capped = binary_reconstruction_search(
        view,
        &ground,
        &free,
        &available,
        &required,
        &mut ws,
        BinaryReconstructionOptions {
            beam: 4,
            max_exchanges: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(capped.budget_reached);
    assert_eq!(capped.exchanges_evaluated, 1);
    let ir = model(&[1, 1, 1]);
    let index = build_incidence_index(&ir).unwrap();
    let ground = vec![
        (HyperArcId::new(0), -1e308),
        (HyperArcId::new(1), 1e308),
        (HyperArcId::new(2), 1e308),
    ];
    let result = binary_reconstruction_search(
        MatroidRef::new(&index),
        &ground,
        &[],
        &[HyperArcId::new(0), HyperArcId::new(1)],
        &[HyperArcId::new(2)],
        &mut ws,
        BinaryReconstructionOptions {
            beam: 4,
            max_exchanges: 100,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(result.candidates.iter().any(|b| b.weight == 1e308));
    assert!(result.candidates.iter().all(|b| b.weight.is_finite()));
}

#[test]
fn independence_exchange_and_weighted_optimum_are_independently_checked() {
    let columns = [1, 2, 3, 4, 5, 6, 7];
    let weights = [7.0, -2.0, 5.0, 1.0, 2.0, 8.0, 3.0];
    let ir = model(&columns);
    let index = build_incidence_index(&ir).unwrap();
    let view = MatroidRef::new(&index);
    let mut ws = BinaryMatroidWorkspace::default();
    let mut independent = vec![];
    for subset in 0usize..128 {
        let ground: Vec<_> = (0..7)
            .filter(|i| subset & (1 << i) != 0)
            .map(|i| (HyperArcId::new(i), weights[i]))
            .collect();
        let basis = binary_weighted_basis(view, &ground, &mut ws).unwrap();
        if basis.elements.len() == subset.count_ones() as usize {
            independent.push(subset);
        }
    }
    assert!(independent.contains(&0));
    for &a in &independent {
        for i in 0..7 {
            assert!(independent.contains(&(a & !(1 << i))));
        }
        for &b in &independent {
            if a.count_ones() < b.count_ones() {
                assert!(
                    (0..7).any(|i| b & !a & (1 << i) != 0 && independent.contains(&(a | (1 << i))))
                );
            }
        }
    }
    let optimum = independent
        .iter()
        .filter(|s| s.count_ones() == 3)
        .map(|s| {
            (0..7)
                .filter(|i| s & (1 << i) != 0)
                .map(|i| weights[i])
                .sum::<f64>()
        })
        .min_by(f64::total_cmp)
        .unwrap();
    let ground: Vec<_> = (0..7).map(|i| (HyperArcId::new(i), weights[i])).collect();
    assert_eq!(
        binary_weighted_basis(view, &ground, &mut ws)
            .unwrap()
            .weight,
        optimum
    );
}

#[test]
fn repeated_incidence_cancels_and_invalid_weights_are_refused() {
    let mut ir = model(&[1]);
    let node = ir.nodes[0].decl;
    fixture::push_arc(
        &mut ir,
        vec![fixture::plus_ref(node, None), fixture::plus_ref(node, None)],
    );
    fixture::push_arc(&mut ir, vec![fixture::plus_ref(node, Some(0.0))]);
    fixture::push_arc(&mut ir, vec![fixture::plus_ref(node, Some(0.5))]);
    let non_node = fixture::edge_decl(&mut ir);
    fixture::push_arc(&mut ir, vec![fixture::plus_ref(non_node, None)]);
    let index = build_incidence_index(&ir).unwrap();
    let view = MatroidRef::new(&index);
    assert!(binary_column(view, HyperArcId::new(1)).unwrap().is_empty());
    assert!(binary_column(view, HyperArcId::new(2)).unwrap().is_empty());
    assert!(matches!(
        binary_column(view, HyperArcId::new(3)),
        Err(MatroidError::NonBinaryIncidence { .. })
    ));
    assert!(matches!(
        binary_column(view, HyperArcId::new(4)),
        Err(MatroidError::DanglingTarget { .. })
    ));
}

#[test]
fn budgets_duplicate_ground_and_unknown_arcs_fail_without_poisoning_reuse() {
    let ir = model(&[1, 2, 3]);
    let index = build_incidence_index(&ir).unwrap();
    let view = MatroidRef::new(&index);
    let mut ws = BinaryMatroidWorkspace::with_limits(1, 8);
    let a = HyperArcId::new(0);
    let b = HyperArcId::new(1);
    assert!(matches!(
        binary_weighted_basis(view, &[(a, 1.), (b, 1.)], &mut ws),
        Err(MatroidError::BinaryWorkspaceBudget)
    ));
    assert_eq!(
        binary_weighted_basis(view, &[(b, 1.)], &mut ws)
            .unwrap()
            .elements,
        [b]
    );
    assert!(matches!(
        binary_weighted_basis(view, &[(a, 1.), (a, 2.)], &mut ws),
        Err(MatroidError::DuplicateGroundElement { .. })
    ));
    assert!(binary_weighted_basis(view, &[(a, f64::NAN)], &mut ws).is_err());
    assert!(binary_weighted_basis(view, &[(HyperArcId::new(99), 1.)], &mut ws).is_err());
}

#[test]
fn learned_prototypes_decode_only_exact_reconstructible_bases() {
    use hymeko_graph::matroid::binary_latent_proposals;
    let columns = [1, 2, 3, 4, 5, 6, 7];
    let ir = model(&columns);
    let index = build_incidence_index(&ir).unwrap();
    let view = MatroidRef::new(&index);
    let mut ws = BinaryMatroidWorkspace::default();
    let ground: Vec<_> = (0..7).map(|i| (HyperArcId::new(i), 1.)).collect();
    let proposal = binary_latent_proposals(view, &ground, &mut ws, 8).unwrap();
    let repeated = binary_latent_proposals(view, &ground, &mut ws, 8).unwrap();
    assert_eq!(proposal.prototypes, repeated.prototypes);
    assert_eq!(proposal.candidates, repeated.candidates);
    assert_eq!(proposal.epochs, 8);
    assert!(
        proposal
            .prototypes
            .iter()
            .all(|p| p.iter().all(|v| v.is_finite()))
    );
    assert!(
        proposal
            .prototypes
            .iter()
            .any(|p| *p != proposal.descriptors[0].1)
    );
    for basis in proposal.candidates {
        assert_eq!(basis.elements.len(), 3);
        for coordinate in basis.coordinates {
            assert_eq!(
                coordinate
                    .support
                    .iter()
                    .fold(0, |x, a| x ^ columns[a.raw()]),
                columns[coordinate.arc.raw()]
            );
        }
    }
    assert!(binary_latent_proposals(view, &ground, &mut ws, 0).is_err());
}

#[test]
fn frontier_matches_exhaustive_orders_and_certifies_every_live_basis() {
    use hymeko_graph::matroid::binary_frontier_search;
    let inputs = [1, 2, 3, 4];
    for outputs in [[1, 2, 4, 3], [4, 3, 2, 1], [1, 1, 2, 4], [7, 6, 5, 4]] {
        let columns = [inputs.as_slice(), outputs.as_slice()].concat();
        let ir = model(&columns);
        let index = build_incidence_index(&ir).unwrap();
        let view = MatroidRef::new(&index);
        let mut ws = BinaryMatroidWorkspace::default();
        let a: Vec<_> = (0..4).map(HyperArcId::new).collect();
        let b: Vec<_> = (4..8).map(HyperArcId::new).collect();
        let result = binary_frontier_search(view, &a, &b, &mut ws, 256, 1000).unwrap();
        let mut optimum = usize::MAX;
        for i in 0..4 {
            for j in 0..4 {
                for k in 0..4 {
                    for l in 0..4 {
                        let order = [i, j, k, l];
                        if order.iter().copied().collect::<BTreeSet<_>>().len() != 4 {
                            continue;
                        }
                        let mut done = 0;
                        let mut peak = 0;
                        for term in order {
                            let before = span_rank(&inputs, 15 ^ done);
                            done |= 1 << term;
                            peak = peak.max(before + span_rank(&outputs, done));
                        }
                        optimum = optimum.min(peak);
                    }
                }
            }
        }
        assert_eq!(result.peak, optimum);
        assert!(result.lower_bound <= optimum);
        assert!(!result.beam_truncated && !result.budget_reached);
        let mut done = 0;
        for step in result.steps {
            assert_eq!(
                step.input_before.elements.len(),
                span_rank(&inputs, 15 ^ done)
            );
            done |= 1 << step.term;
            assert_eq!(step.output_after.elements.len(), span_rank(&outputs, done));
            assert_eq!(step.input_rank_after, span_rank(&inputs, 15 ^ done));
            for basis in [step.input_before, step.output_after] {
                for c in basis.coordinates {
                    assert_eq!(
                        c.support.iter().fold(0, |v, a| v ^ columns[a.raw()]),
                        columns[c.arc.raw()]
                    );
                }
            }
        }
        assert_eq!(done, 15);
        let limited = binary_frontier_search(view, &a, &b, &mut ws, 1, 1).unwrap();
        assert!(limited.budget_reached);
        assert_eq!(limited.steps.len(), 4);
        assert!(binary_frontier_search(view, &a, &b[..3], &mut ws, 1, 10).is_err());
        assert!(binary_frontier_search(view, &a, &b, &mut ws, 0, 10).is_err());
    }
}

#[test]
fn frontier_counts_read_before_retirement_and_handles_repeated_forms() {
    use hymeko_graph::matroid::binary_frontier_search;
    let ir = model(&[1, 1]);
    let index = build_incidence_index(&ir).unwrap();
    let mut ws = BinaryMatroidWorkspace::default();
    let a = HyperArcId::new(0);
    let b = HyperArcId::new(1);
    let result =
        binary_frontier_search(MatroidRef::new(&index), &[a], &[b], &mut ws, 1, 10).unwrap();
    assert_eq!(result.peak, 2);
    assert_eq!(result.steps[0].input_rank_after, 0);
    let repeated =
        binary_frontier_search(MatroidRef::new(&index), &[a, a], &[b, b], &mut ws, 4, 10).unwrap();
    assert_eq!(repeated.peak, 2);
    assert!(
        binary_frontier_search(
            MatroidRef::new(&index),
            &[a],
            &[HyperArcId::new(99)],
            &mut ws,
            1,
            10
        )
        .is_err()
    );
}

#[test]
fn reconstruction_can_leave_the_active_span_and_preserves_the_incumbent_at_a_budget() {
    use hymeko_graph::matroid::binary_reconstruction_search;
    // Initial basis {1,2,4,8} uses three columns for 7. The physical product
    // 15 is outside span{1,2,4}; 8 XOR 15 gives the target with two columns.
    // No available single column is 7, so two is an independent exact optimum.
    let columns = [1, 2, 4, 8, 15, 7];
    let ir = model(&columns);
    let index = build_incidence_index(&ir).unwrap();
    let ground: Vec<_> = (0..6)
        .map(|i| {
            (
                HyperArcId::new(i),
                if i < 4 {
                    0.0
                } else if i == 4 {
                    1.0
                } else {
                    100.0
                },
            )
        })
        .collect();
    let available: Vec<_> = (0..5).map(HyperArcId::new).collect();
    let required = [HyperArcId::new(5)];
    let options = BinaryReconstructionOptions {
        target_cost: Some(2),
        max_exchanges: 128,
        max_extra_products: 1,
        ..Default::default()
    };
    let mut workspace = BinaryMatroidWorkspace::default();
    let result = binary_reconstruction_search(
        MatroidRef::new(&index),
        &ground,
        &[],
        &available,
        &required,
        &mut workspace,
        options,
    )
    .unwrap();
    assert!(result.target_reached);
    let needed = result.candidates[0]
        .coordinates
        .iter()
        .find(|c| c.arc == required[0])
        .unwrap();
    assert_eq!(needed.support.len(), 2);
    for candidate in &result.candidates {
        for coordinate in &candidate.coordinates {
            assert_eq!(
                coordinate
                    .support
                    .iter()
                    .fold(0, |v, a| v ^ columns[a.raw()]),
                columns[coordinate.arc.raw()]
            );
        }
    }
    let limited = binary_reconstruction_search(
        MatroidRef::new(&index),
        &ground,
        &[],
        &available,
        &required,
        &mut workspace,
        BinaryReconstructionOptions {
            max_states: 1,
            ..options
        },
    )
    .unwrap();
    assert!(limited.state_budget_reached);
    assert_eq!(limited.candidates[0].elements, limited.baseline.elements);
}

#[test]
fn grouped_barrier_is_sound_for_small_orderings_and_refuses_weak_groups() {
    use hymeko_graph::matroid::{binary_frontier_barrier, binary_frontier_search};
    // Three directions in GF(2)^2: any two groups span, every nonzero
    // functional activates two groups. All six orders must reach rank sum4.
    let ir = model(&[1, 2, 3, 1, 2, 3]);
    let index = build_incidence_index(&ir).unwrap();
    let view = MatroidRef::new(&index);
    let mut ws = BinaryMatroidWorkspace::default();
    let a: Vec<_> = (0..3).map(HyperArcId::new).collect();
    let b: Vec<_> = (3..6).map(HyperArcId::new).collect();
    let groups = vec![vec![0], vec![1], vec![2]];
    let proof = binary_frontier_barrier(view, &a, &b, &groups, &mut ws).unwrap();
    assert!(proof.certified);
    assert_eq!((proof.input_cover, proof.active_groups), (2, 2));
    let exact = binary_frontier_search(view, &a, &b, &mut ws, 256, 100).unwrap();
    assert_eq!(exact.peak, proof.input_rank + proof.output_rank);
    // Independent singleton directions admit a3-rank frontier, so a4-rank
    // obstruction must not be certified.
    let weak = binary_frontier_barrier(view, &a[..2], &b[..2], &groups[..2], &mut ws).unwrap();
    assert!(!weak.certified);
    let local_failure = binary_frontier_barrier(view, &a, &b, &[vec![0, 1, 2]], &mut ws).unwrap();
    assert!(local_failure.certified);
    assert!(binary_frontier_barrier(view, &a, &b, &[vec![0, 1], vec![1, 2]], &mut ws).is_err());
}

#[test]
fn quotient_completion_matches_exhaustive_paid_subsets() {
    use hymeko_graph::matroid::binary_reconstruction_search;
    let columns=[4,8,5,10,6,1,2];
    let ir=model(&columns);let index=build_incidence_index(&ir).unwrap();
    let ground:Vec<_>=(0..7).map(|i|(HyperArcId::new(i),if i<4 {1.0} else if i==4 {10.0} else {1000.0})).collect();
    let available:Vec<_>=(0..5).map(HyperArcId::new).collect();
    let required=[HyperArcId::new(5),HyperArcId::new(6)];
    let optimum=(0usize..32).filter(|s|span_rank(&columns,*s)==span_rank(&columns,*s|96))
        .map(|s|s.count_ones() as usize).min().unwrap();
    assert_eq!(optimum,3);
    for goal in [2,3] {
        let result=binary_reconstruction_search(MatroidRef::new(&index),&ground,&[],&available,&required,
            &mut BinaryMatroidWorkspace::default(),BinaryReconstructionOptions{target_cost:Some(goal),..Default::default()}).unwrap();
        assert_eq!(result.lower_bound,2);
        assert_eq!(result.target_infeasible_certified,goal<optimum);
        assert_eq!(result.target_reached,goal>=optimum);
        // A negative certificate must not end generation: the exchange stage
        // still delivers the catalogue optimum above the excluded goal.
        assert_eq!(result.exchanges_evaluated==0,goal>=optimum);
        let paid=|b:&hymeko_graph::matroid::BinaryBasis|b.coordinates.iter()
            .filter(|c|required.contains(&c.arc)).flat_map(|c|c.support.iter()).collect::<BTreeSet<_>>().len();
        assert_eq!(result.candidates.iter().map(paid).min(),Some(optimum));
        for basis in &result.candidates {
            assert_eq!(basis.weight,basis.elements.iter().map(|a|ground[a.raw()].1).sum::<f64>());
            for row in &basis.coordinates {
                assert_eq!(row.support.iter().fold(0,|v,a|v^columns[a.raw()]),columns[row.arc.raw()]);
            }
        }
    }
}

#[test]
fn certified_goal_keeps_distinct_equal_cost_reconstructions() {
    use hymeko_graph::matroid::binary_reconstruction_search;
    // 7 has no one- or two-column form over {1,2,4,9,10}; exactly two
    // three-column forms exist: 1^2^4 and 4^9^10.
    let columns = [1, 2, 4, 9, 10, 7];
    let ir = model(&columns);
    let index = build_incidence_index(&ir).unwrap();
    let ground: Vec<_> = (0..6)
        .map(|i| (HyperArcId::new(i), if i < 5 { 1.0 } else { 100.0 }))
        .collect();
    let available: Vec<_> = (0..5).map(HyperArcId::new).collect();
    let required = [HyperArcId::new(5)];
    let result = binary_reconstruction_search(
        MatroidRef::new(&index),
        &ground,
        &[],
        &available,
        &required,
        &mut BinaryMatroidWorkspace::default(),
        BinaryReconstructionOptions {
            target_cost: Some(2),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(result.target_infeasible_certified && !result.target_reached);
    let forms: BTreeSet<Vec<usize>> = result
        .candidates
        .iter()
        .map(|b| {
            let row = b.coordinates.iter().find(|c| c.arc == required[0]).unwrap();
            assert_eq!(row.support.iter().fold(0, |v, a| v ^ columns[a.raw()]), 7);
            row.support.iter().map(|a| a.raw()).collect()
        })
        .collect();
    assert_eq!(forms, BTreeSet::from([vec![0, 1, 2], vec![2, 3, 4]]));
}

#[test]
fn every_certified_small_barrier_is_below_all_exact_order_costs() {
    use hymeko_graph::matroid::binary_frontier_barrier;
    let triples: Vec<_> = (1..=7)
        .flat_map(|a| (a + 1..=7).flat_map(move |b| (b + 1..=7).map(move |c| [a, b, c])))
        .collect();
    let groups = vec![vec![0], vec![1], vec![2]];
    let mut certified = 0;
    for inputs in &triples {
        for outputs in &triples {
            let columns = [inputs.as_slice(), outputs.as_slice()].concat();
            let ir = model(&columns);
            let index = build_incidence_index(&ir).unwrap();
            let mut ws = BinaryMatroidWorkspace::default();
            let proof = binary_frontier_barrier(
                MatroidRef::new(&index),
                &(0..3).map(HyperArcId::new).collect::<Vec<_>>(),
                &(3..6).map(HyperArcId::new).collect::<Vec<_>>(),
                &groups,
                &mut ws,
            )
            .unwrap();
            if !proof.certified {
                continue;
            }
            certified += 1;
            for order in [
                [0, 1, 2],
                [0, 2, 1],
                [1, 0, 2],
                [1, 2, 0],
                [2, 0, 1],
                [2, 1, 0],
            ] {
                let mut done = 0;
                let mut peak = 0;
                for term in order {
                    let before = span_rank(inputs, 7 ^ done);
                    done |= 1 << term;
                    peak = peak.max(before + span_rank(outputs, done));
                }
                assert!(peak >= proof.input_rank + proof.output_rank);
            }
        }
    }
    assert!(certified > 0);
}
