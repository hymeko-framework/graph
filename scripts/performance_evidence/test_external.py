"""Correctness-only checks of native library adapters, without timers."""
import unittest
import numpy as np

from external import BACKENDS, CanonicalCase, ExternalCell


class _ExactTourOracle:
    """Private bounded Held-Karp reference, never a selectable runtime backend.

    Independent recurrence on a Floyd-Warshall metric of the exported memberships.
    Scope: at most 16 vertices, dyadic weights; expected cost only, outside timing.
    """
    @staticmethod
    def cost(case):
        import functools
        import math
        n=case["nodes"]
        if not 1 <= n <= 16: raise ValueError("exact oracle limited to 16 vertices")
        d=[[0.0 if u==v else math.inf for v in range(n)] for u in range(n)]
        for edge,cost in zip(case["memberships"],case["costs"]):
            for u in edge:
                for v in edge: d[u][v]=min(d[u][v],cost)
        for k in range(n):
            for u in range(n):
                for v in range(n): d[u][v]=min(d[u][v],d[u][k]+d[k][v])
        @functools.lru_cache(None)
        def visit(at,remaining):
            if not remaining: return d[at][0]
            best=math.inf
            bits=remaining
            while bits:
                bit=bits & -bits; bits-=bit; v=bit.bit_length()-1
                best=min(best,d[at][v]+visit(v,remaining^bit))
            return best
        return visit(0,((1<<n)-1)^1)

    @staticmethod
    def bounds(case):
        """Independent metric MST / maximum one-tree lower bounds, n <= 64.

        Removing one vertex from a Hamiltonian cycle leaves a spanning path;
        its cost bounds the remaining MST plus that vertex's two cheapest legs.
        This returns a lower bound, never an exact optimum for large inputs.
        """
        import networkx as nx
        import itertools
        n=case["nodes"]
        if not 3 <= n <= 64: raise ValueError("one-tree reference scope is 3..64")
        graph=nx.Graph();graph.add_nodes_from(range(n))
        for edge,cost in zip(case["memberships"],case["costs"]):
            for u,v in itertools.combinations(edge,2):
                if not graph.has_edge(u,v) or cost<graph[u][v]["weight"]:graph.add_edge(u,v,weight=cost)
        distances=dict(nx.all_pairs_dijkstra_path_length(graph,weight="weight"))
        if any(len(row)!=n for row in distances.values()):raise ValueError("disconnected tour domain")
        metric=nx.Graph();metric.add_nodes_from(range(n))
        metric.add_weighted_edges_from((u,v,distances[u][v]) for u in range(n) for v in range(u+1,n))
        mst=nx.minimum_spanning_tree(metric).size(weight="weight")
        lower=mst
        for root in range(n):
            rest=metric.subgraph(v for v in range(n) if v!=root)
            two=sum(sorted(distances[root][v] for v in range(n) if v!=root)[:2])
            lower=max(lower,nx.minimum_spanning_tree(rest).size(weight="weight")+two)
        return {"tour_lower_bound":lower,"tour_mst_cost":mst,"tour_bound_kind":"max_metric_MST_one_tree"}


class ExternalContract(unittest.TestCase):
    def test_explicit_zero_heuristic_preserves_cheap_long_distance_transport(self):
        from routing import RoutingBackend
        data=dict(family="named_geometry",nodes=3,arcs=3,incidences=6,memberships=[[0,1],[1,2],[0,2]],
            costs=[0.5,0.5,3.0],components=1,labels=[0]*3,reachable=[0,1,2],start=0,goal=2,
            shortest_hops=2,shortest_cost=1.0,positions=[[0,0],[1000,0],[1,0]],heuristic="zero")
        for name in BACKENDS:
            adapter=RoutingBackend(CanonicalCase(data,"0"*64),name)
            result=adapter.query("astarpath");adapter.verify(result,"astarpath")
            self.assertEqual(result.cost,1.0)

    def test_larger_tour_verification_uses_bounds_without_claiming_exact_optimum(self):
        from routing import RoutingBackend
        data=dict(family="bounded",nodes=4,arcs=3,incidences=7,memberships=[[0,1,2],[2,3],[0,3]],
            costs=[0.5,2.0,1.0],components=1,labels=[0]*4,reachable=list(range(4)),start=0,goal=3,
            shortest_hops=1,shortest_cost=1.0,positions=[[0,0]]*4,heuristic="zero")
        data.update(_ExactTourOracle.bounds(data))
        optimum=_ExactTourOracle.cost(data)
        self.assertLessEqual(data["tour_lower_bound"],optimum)
        self.assertLessEqual(optimum,2*data["tour_mst_cost"])
        for name in BACKENDS:
            adapter=RoutingBackend(CanonicalCase(data,"0"*64),name)
            adapter.verify(adapter.query("tour"),"tour")
            adapter=RoutingBackend(CanonicalCase(data,"0"*64),name)
            adapter.verify(adapter.query("astarpath"),"astarpath")
        with self.assertRaises(ValueError):_ExactTourOracle.bounds({"nodes":65})
    def test_native_unit_routes_return_complete_incidence_witnesses(self):
        from routing import RoutingBackend
        data=dict(family="tiny",nodes=4,arcs=2,incidences=5,memberships=[[0,1,2],[2,3]],
            costs=[1.0,1.0],components=1,labels=[0]*4,reachable=list(range(4)),
            start=0,goal=3,shortest_hops=2,shortest_cost=2.0,positions=[[0,0]]*4)
        for name in BACKENDS:
            with self.subTest(backend=name):
                adapter=RoutingBackend(CanonicalCase(data,"0"*64),name)
                route=adapter.query("unitpath");adapter.verify(route,"unitpath")
                self.assertEqual(route.arcs,[0,1]);self.assertEqual(route.vertices,[0,2,3])
                data["costs"][0]=0.5
                with self.assertRaises(ValueError):adapter.query("unitpath")
                data["costs"][0]=1.0

    def test_route_record_kind_does_not_collide_with_record_type(self):
        import contextlib, io
        output=io.StringIO()
        with contextlib.redirect_stdout(output):
            ExternalCell.emit("ROUTE",kind="path",verified="true")
        self.assertEqual(output.getvalue(),"EVIDENCE_ROUTE\tkind=path\tverified=true\n")

    def test_full_weighted_paths_and_christofides_preserve_original_incidences(self):
        from routing import RoutingBackend
        data=dict(family="tiny",nodes=4,arcs=3,incidences=7,memberships=[[0,1,2],[2,3],[0,3]],
            costs=[0.5,2.0,1.0],components=1,labels=[0]*4,reachable=list(range(4)),
            start=0,goal=3,shortest_hops=1,shortest_cost=1.0,positions=[[0,0]]*4)
        data["optimal_tour_cost"]=_ExactTourOracle.cost(data)
        # 0->1->2->0->3->0 costs .5+.5+.5+1+1 = 3.5.
        self.assertEqual(data["optimal_tour_cost"],3.5)
        for name in BACKENDS:
            for operation in ("path","astarpath","tour"):
                with self.subTest(backend=name,operation=operation):
                    adapter=RoutingBackend(CanonicalCase(data,"0"*64),name)
                    result=adapter.query(operation)
                    adapter.verify(result,operation)
                    adapter.verify(adapter.query(operation),operation)
                    result.cost+=1
                    with self.assertRaises(ValueError): adapter.verify(result,operation)

    def test_route_boundary_and_exact_reference_reject_invalid_scope(self):
        case=self.case(2,[[0,1]],[0,0],[0,1],1)
        with self.assertRaises(ValueError): ExternalCell(case,"xgi","prepared","path")
        with self.assertRaises(ValueError): ExternalCell(case,"xgi","first","components")
        with self.assertRaises(ValueError): _ExactTourOracle.cost({"nodes":17})
    def test_domain_shortcuts_do_not_claim_a_library_search(self):
        disconnected=self.case(5,[[0,1],[1,2]],[0,0,0,1,2],[0,1,2],None)
        self.assertEqual(ExternalCell(disconnected,"hypernetx","prepared","distance").strategy(),
                         "declared_domain_isolate_short_circuit")
        isolated=self.case(2,[],[0,1],[0],None)
        self.assertEqual(ExternalCell(isolated,"hypernetx","prepared","reachable").strategy(),
                         "declared_domain_isolate_short_circuit")
        single=self.case(1,[],[0],[0],0)
        self.assertEqual(ExternalCell(single,"hypernetx","prepared","distance").strategy(),
                         "same_vertex_identity_short_circuit")

    def case(self, n, edges, labels, reachable=None, hops=None):
        return CanonicalCase(dict(family="contract", nodes=n, arcs=len(edges),
            incidences=sum(map(len,edges)), memberships=edges, labels=labels,
            components=len(set(labels)),reachable=reachable or [],shortest_hops=hops), "0"*64)

    def test_full_partitions_include_isolates_duplicates_and_singletons(self):
        cases = [self.case(6, [[0,1],[1,2],[3,4]], [0,0,0,1,1,2]),
                 self.case(5, [[0,1],[0,1],[2]], [0,0,1,2,3]),
                 self.case(3, [], [0,1,2]), self.case(0, [], [])]
        for case in cases:
            for name, factory in BACKENDS.items():
                with self.subTest(backend=name,nodes=case.n,edges=len(case.edges)):
                    adapter = factory(case)
                    adapter.verify(adapter.query())
                    adapter.verify(adapter.query())

    def test_hypernetx_domain_completion_is_explicit(self):
        case=self.case(5, [[0,1],[1,2]], [0,0,0,1,2])
        for name,factory in BACKENDS.items():
            adapter=factory(case)
            self.assertEqual(adapter.completed_isolates,2 if name=="hypernetx" else 0)

    def test_full_partition_check_rejects_equal_count_wrong_labels(self):
        adapter=BACKENDS["xgi"](self.case(4, [[0,1],[2,3]], [0,0,1,1]))
        count=adapter.query()
        adapter.output[:]=np.array([0,1,0,1])
        with self.assertRaises(ValueError):adapter.verify(count)

    def test_invalid_target_rejected_before_library_construction(self):
        with self.assertRaises(ValueError):self.case(3, [[0,4]], [0,1,2])

    def test_reachability_and_hyperedge_hop_distance(self):
        for case in [self.case(4, [[0,1,2],[2,3]], [0,0,0,0], [0,1,2,3], 2),
                     self.case(5, [[0,1,2],[2,3]], [0,0,0,0,1], [0,1,2,3], None),
                     self.case(1, [], [0], [0], 0)]:
            for name,factory in BACKENDS.items():
                for operation in ["reachable","distance"]:
                    with self.subTest(backend=name,operation=operation,nodes=case.n):
                        adapter=factory(case)
                        adapter.verify(adapter.query(operation),operation)


if __name__ == "__main__":
    unittest.main()
