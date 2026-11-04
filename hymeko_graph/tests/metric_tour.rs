//! Correctness-only client: independent Floyd-Warshall and exhaustive tour oracle.
use hymeko::common::ids::NodeId;
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::Ir;
use hymeko_graph::direct_ir::{BergeTour, Christofides, TourError, TourLimits};
#[path = "informed_common/mod.rs"]
mod fixture;

struct Oracle {
    n: usize,
    distances: Vec<f64>,
}
impl Oracle {
    fn build(ir: &Ir, costs: &[f64]) -> Self {
        let index = build_incidence_index(ir).unwrap();
        let n = ir.nodes.len();
        let mut d = vec![f64::INFINITY; n * n];
        for i in 0..n {
            d[i * n + i] = 0.0;
        }
        for (a, arc) in ir.arcs.iter().enumerate() {
            let members: Vec<_> = (0..arc.refs.len())
                .map(|s| {
                    ir.as_node(
                        index
                            .resolve(hymeko::common::ids::HyperArcId::new(a), s as u32)
                            .target,
                    )
                    .unwrap()
                    .raw()
                })
                .collect();
            for &u in &members {
                for &v in &members {
                    d[u * n + v] = d[u * n + v].min(costs[a]);
                }
            }
        }
        for k in 0..n {
            for i in 0..n {
                for j in 0..n {
                    d[i * n + j] = d[i * n + j].min(d[i * n + k] + d[k * n + j]);
                }
            }
        }
        Self { n, distances: d }
    }
    fn optimum(&self) -> f64 {
        fn visit(o: &Oracle, at: usize, mask: usize, cost: f64, best: &mut f64) {
            if mask == (1 << o.n) - 1 {
                *best = best.min(cost + o.distances[at * o.n]);
                return;
            }
            for v in 1..o.n {
                if mask & (1 << v) == 0 {
                    visit(
                        o,
                        v,
                        mask | (1 << v),
                        cost + o.distances[at * o.n + v],
                        best,
                    );
                }
            }
        }
        let mut best = f64::INFINITY;
        visit(self, 0, 1, 0.0, &mut best);
        best
    }
    fn verify(&self, ir: &Ir, costs: &[f64], tour: &BergeTour) {
        assert_eq!(tour.order.len(), self.n + 1);
        assert_eq!(tour.order.first(), tour.order.last());
        let mut nodes: Vec<_> = tour.order[..self.n].iter().map(|v| v.raw()).collect();
        nodes.sort_unstable();
        assert_eq!(nodes, (0..self.n).collect::<Vec<_>>());
        let index = build_incidence_index(ir).unwrap();
        let mut last = tour.walk.start;
        let mut total = 0.0;
        let mut visited = vec![false; self.n];
        visited[last.raw()] = true;
        for step in &tour.walk.steps {
            assert_eq!(last, step.from.node);
            assert_eq!(
                index.resolve(step.arc, step.from.slot).target,
                ir.nodes[last.raw()].decl
            );
            assert_eq!(
                index.resolve(step.arc, step.to.slot).target,
                ir.nodes[step.to.node.raw()].decl
            );
            last = step.to.node;
            visited[last.raw()] = true;
            total += costs[step.arc.raw()];
        }
        assert_eq!(last, tour.walk.start);
        assert!(visited.into_iter().all(|v| v));
        assert_eq!(total, tour.total_cost);
        let metric_total: f64 = tour
            .order
            .windows(2)
            .map(|p| self.distances[p[0].raw() * self.n + p[1].raw()])
            .sum();
        assert_eq!(metric_total, total);
        let optimum = self.optimum();
        assert!(
            total >= optimum && total <= 1.5 * optimum,
            "cost {total}, optimum {optimum}"
        );
    }
}

#[test]
fn christofides_matches_exact_small_tsp_bound_and_real_incidences() {
    for n in 2..=8 {
        for arity in 2..=n.min(4) {
            let ir = fixture::window_ir(n, arity, false, fixture::SignMode::Alternating);
            let costs: Vec<_> = (0..ir.arcs.len())
                .map(|a| 0.5 + ((a * 7 + n) % 5) as f64 * 0.5)
                .collect();
            let index = build_incidence_index(&ir).unwrap();
            let tour = Christofides::solve(&index, &costs, TourLimits::default()).unwrap();
            Oracle::build(&ir, &costs).verify(&ir, &costs, &tour);
        }
    }
}

#[test]
fn transport_hyperarc_and_zero_cost_metric_are_supported() {
    let ir = fixture::map_grid_with_transport_ir(4, 2, &Default::default(), 4, 0.5);
    let costs: Vec<_> = ir
        .arcs
        .iter()
        .map(|a| fixture::first_num_weight_pub(&ir, &a.refs[0]))
        .collect();
    let index = build_incidence_index(&ir).unwrap();
    for costs in [costs, vec![0.0; ir.arcs.len()]] {
        let a = Christofides::solve(&index, &costs, TourLimits::default()).unwrap();
        let b = Christofides::solve(&index, &costs, TourLimits::default()).unwrap();
        assert_eq!(a.order, b.order);
        assert_eq!(a.walk, b.walk);
        Oracle::build(&ir, &costs).verify(&ir, &costs, &a);
    }
}

#[test]
fn empty_singleton_disconnected_and_invalid_costs_are_explicit() {
    let empty = Ir::default();
    assert_eq!(
        Christofides::solve(
            &build_incidence_index(&empty).unwrap(),
            &[],
            TourLimits::default()
        )
        .unwrap_err(),
        TourError::Empty
    );
    let mut ir = Ir::default();
    fixture::node_decl(&mut ir);
    let costs = vec![1.0; ir.arcs.len()];
    let tour = Christofides::solve(
        &build_incidence_index(&ir).unwrap(),
        &costs,
        TourLimits::default(),
    )
    .unwrap();
    assert_eq!(tour.order, vec![NodeId::new(0); 2]);
    assert!(tour.walk.steps.is_empty());
    let ir = fixture::multi_component_ir(2, 2, 2);
    let index = build_incidence_index(&ir).unwrap();
    assert!(matches!(
        Christofides::solve(&index, &vec![1.0; ir.arcs.len()], TourLimits::default()),
        Err(TourError::Disconnected { .. })
    ));
    assert_eq!(
        Christofides::solve(&index, &[], TourLimits::default()).unwrap_err(),
        TourError::CostDomain
    );
    for bad in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            Christofides::solve(&index, &vec![bad; ir.arcs.len()], TourLimits::default()),
            Err(TourError::InvalidCost { .. })
        ));
    }
}

#[test]
fn every_resource_budget_rejects_instead_of_changing_algorithm() {
    let ir = fixture::window_ir(4, 4, false, fixture::SignMode::Alternating);
    let index = build_incidence_index(&ir).unwrap();
    let costs = vec![1.0; ir.arcs.len()];
    for (limits, resource) in [
        (
            TourLimits {
                max_nodes: 3,
                ..Default::default()
            },
            "metric nodes",
        ),
        (
            TourLimits {
                max_matching_states: 2,
                ..Default::default()
            },
            "matching states",
        ),
        (
            TourLimits {
                max_walk_steps: 1,
                ..Default::default()
            },
            "expanded walk steps",
        ),
    ] {
        assert_eq!(
            Christofides::solve(&index, &costs, limits).unwrap_err(),
            TourError::Budget { resource }
        );
    }
}
