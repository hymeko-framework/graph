"""Full-route composition clients. Search and Christofides are NetworkX-owned.

Native framework incidence/projection APIs provide topology. Every conversion,
weight binding, metric closure and returned route expansion is inside query().
This is explicitly a composed workflow, not four native weighted TSP engines.
"""
from dataclasses import dataclass
import math
import networkx as nx
from external import ComposedBackend, HyperNetXBackend, CanonicalCase


class RoutingHyperNetX(HyperNetXBackend):
    def build(self):
        # Public bipartite() uses original IDs in both partitions. Disjoint labels
        # prevent node/edge identity collisions, without changing memberships.
        return self.module.Hypergraph({("edge", a): row for a, row in enumerate(self.case.edges)})


@dataclass(eq=True)
class Route:
    vertices: list[int]
    arcs: list[int]
    cost: float


class RoutingBackend(ComposedBackend):
    def __init__(self, case, backend):
        super().__init__(case, backend, RoutingHyperNetX if backend == "hypernetx" else None)
        self.first = None
        if len(case.data["costs"]) != len(case.edges) or any(not math.isfinite(c) or c < 0 for c in case.data["costs"]):
            raise ValueError("invalid symmetric traversal weights")
        if len({tuple(sorted(e)) for e in case.edges}) != len(case.edges):
            raise ValueError("this composed input adapter requires unique hyperedges")

    def projection(self):
        native = self.native.graph
        if self.name == "hypernetx":
            graph = native.bipartite()
            vertex = {int(v): int(v) for v in native.nodes}
            arc = {("edge", a): a for a in range(len(self.case.edges))}
        elif self.name == "xgi":
            graph, vertex, arc = self.native.module.to_bipartite_graph(native, index=True)
        elif self.name == "hypergraphx":
            from hypergraphx.representations.projections import bipartite_projection
            graph, original = bipartite_projection(native)
            by_members = {tuple(sorted(e)): a for a,e in enumerate(self.case.edges)}
            vertex = {key: int(value) for key,value in original.items() if isinstance(value,int)}
            arc = {key: by_members[tuple(sorted(value))] for key,value in original.items() if isinstance(value,tuple)}
        else:
            # Reticula has no matched weighted full-path Python call in the
            # pinned version. This small bridge exports its public incidences.
            graph = nx.Graph()
            vertex = {int(v): int(v) for v in native.vertices()}
            graph.add_nodes_from(vertex)
            arc = {}
            by_members = {tuple(sorted(e)): a for a,e in enumerate(self.case.edges)}
            for edge in native.edges():
                members = tuple(sorted(map(int,edge.incident_verts())))
                a = by_members[members]; label = ("edge",a); arc[label] = a
                graph.add_node(label)
                graph.add_edges_from((label,v) for v in members)
        # Preserve declared isolates, including HyperNetX's omitted ones.
        original_to_vertex = {value:key for key,value in vertex.items()}
        for v in range(self.case.n):
            if v not in original_to_vertex:
                label = ("isolated",v); graph.add_node(label)
                vertex[label] = v; original_to_vertex[v] = label
        for label,a in arc.items():
            for v in graph[label]: graph[label][v]["weight"] = self.case.data["costs"][a]/2
        return graph,vertex,arc,original_to_vertex

    def query(self, operation="path", materialize=True):
        if operation == "unitpath":
            return self.unit_query()
        graph,vertices,arcs,ids = self.projection()
        start,goal = self.case.data.get("start",0),self.case.data.get("goal",self.case.n-1)
        if operation == "tour":
            # The official wrapper computes shortest-path metric closure,
            # Christofides, then expands each metric leg into graph paths.
            walk = nx.approximation.traveling_salesman_problem(graph,nodes=[ids[v] for v in range(self.case.n)],
                cycle=True,method=nx.approximation.christofides,weight="weight")
        else:
            try:
                if operation == "astarpath":
                    positions = self.case.data["positions"]
                    gx,gy=positions[goal]
                    kind=self.case.data.get("heuristic", "zero" if self.case.name.startswith("transport") else "manhattan" if self.case.name.startswith("grid") else "euclidean")
                    def h(v):
                        if kind == "zero": return 0.0
                        x,y=positions[v]
                        return abs(x-gx)+abs(y-gy) if kind=="manhattan" else math.hypot(x-gx,y-gy)
                    def heuristic(node,_goal):
                        if node in vertices: return h(vertices[node])
                        return min(h(vertices[v]) for v in graph[node])+self.case.data["costs"][arcs[node]]/2
                    walk=nx.astar_path(graph,ids[start],ids[goal],heuristic=heuristic,weight="weight")
                else:
                    _,walk=nx.bidirectional_dijkstra(graph,ids[start],ids[goal],weight="weight")
            except nx.NetworkXNoPath:
                return Route([],[],math.inf)
        result=Route([vertices[v] for v in walk[::2]],[arcs[a] for a in walk[1::2]],0.0)
        result.cost=sum(self.case.data["costs"][a] for a in result.arcs)
        return result

    def unit_query(self):
        """Native unweighted SSSP or public clique query, then full witnesses.

        The descending-distance walk only reconstructs a library result; it does
        not introduce another graph search implementation in the adapter.
        """
        if any(c != 1.0 for c in self.case.data["costs"]):
            raise ValueError("unit route requested for weighted input")
        start,goal=self.case.data["start"],self.case.data["goal"]
        native=self.native.graph
        by_members={tuple(sorted(e)):a for a,e in enumerate(self.case.edges)} if self.name in {"reticula","hypergraphx"} else None
        hnx_memberships=native.nodes.memberships if self.name=="hypernetx" else None
        hnx_edges=native.incidence_dict if self.name=="hypernetx" else None
        def incident(v):
            if self.name == "xgi":
                return [(int(a),self.native.graph.edges.members(a)) for a in native.nodes.memberships(v)]
            if self.name == "reticula":
                return [(by_members[tuple(sorted(map(int,e.incident_verts())))],e.incident_verts()) for e in native.incident_edges(v)]
            if self.name == "hypernetx":
                return [(a[1],hnx_edges[a]) for a in hnx_memberships[v]]
            return [(by_members[tuple(sorted(e))],e) for e in native.get_incident_edges(v)]
        if self.name in {"xgi","reticula"}:
            distances=(self.native.module.single_source_shortest_path_length(native,start) if self.name=="xgi"
                       else self.native.module.shortest_path_lengths_from(native,start))
            if goal not in distances or not math.isfinite(distances[goal]):return Route([],[],math.inf)
            vertices=[goal];arcs=[];current=goal
            while current != start:
                candidates=[(int(v),a) for a,members in incident(current) for v in members if distances.get(v,math.inf)==distances[current]-1]
                if not candidates:raise ValueError("native distances have no incidence predecessor")
                current,a=min(candidates);vertices.append(current);arcs.append(a)
            vertices.reverse();arcs.reverse()
        else:
            if self.name == "hypernetx":
                projected=native.get_linegraph(s=1,edges=False)
            else:
                from hypergraphx.representations.projections import clique_projection
                projected=clique_projection(native,keep_isolated=True)
            try:vertices=list(map(int,nx.bidirectional_shortest_path(projected,start,goal)))
            except nx.NetworkXNoPath:return Route([],[],math.inf)
            arcs=[min(a for a,members in incident(u) if v in members) for u,v in zip(vertices,vertices[1:])]
        return Route(vertices,arcs,float(len(arcs)))

    def verify(self, result, operation="path"):
        if not result.vertices:
            if operation == "tour" or self.case.data.get("shortest_cost") is not None:
                raise ValueError("unexpected unreachable route")
            return
        if len(result.vertices)!=len(result.arcs)+1:
            raise ValueError("invalid alternating route")
        for u,a,v in zip(result.vertices,result.arcs,result.vertices[1:]):
            if u not in self.case.edges[a] or v not in self.case.edges[a]: raise ValueError("invalid incidence witness")
        if result.cost != sum(self.case.data["costs"][a] for a in result.arcs): raise ValueError("incorrect route cost")
        if operation == "tour":
            if result.vertices[0]!=result.vertices[-1] or set(result.vertices)!=set(range(self.case.n)):
                raise ValueError("tour is not closed or misses vertices")
            optimum=self.case.data.get("optimal_tour_cost")
            if optimum is not None:
                if not optimum <= result.cost <= 1.5*optimum: raise ValueError("tour violates exact optimum/approximation bound")
            elif not self.case.data["tour_lower_bound"] <= result.cost <= 2*self.case.data["tour_mst_cost"]:
                raise ValueError("tour violates independent lower/tree-walk upper bounds")
        else:
            if (result.vertices[0],result.vertices[-1]) != (self.case.data.get("start",0),self.case.data.get("goal",self.case.n-1)):
                raise ValueError("route endpoints changed")
            if result.cost != self.case.data["shortest_cost"]: raise ValueError("nonoptimal route")
        if self.first is None: self.first=result
        elif self.first != result: raise ValueError("route changed within process")

    def route_hash(self, result):
        return CanonicalCase.hash_values([len(result.vertices),*result.vertices,len(result.arcs),*result.arcs,int(result.cost*1e6)])
