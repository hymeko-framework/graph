"""External graph query clients for the existing guarded campaign runner.

Default execution is correctness-only. Timings require HYMEKO_EXTERNAL_BENCH=1
and the explicit external_cell command. Graph topology is loaded from the
canonical Core IR exporter; no graph-generation implementation lives here.
"""
from __future__ import annotations

from abc import ABC, abstractmethod
import hashlib
import importlib
import importlib.metadata
import inspect
import json
import math
import os
from pathlib import Path
import signal
import sys
import time
from typing import Any

import numpy as np

VERSIONS = {"hypernetx": "2.4.3", "xgi": "0.10.2", "hypergraphx": "1.8.0", "reticula": "0.13.0"}
MODES = ("prepared", "cold", "first", "profile")
SCOPES = {
    "prepared": "prepared_native_graph_component_query_normalize_drop_reused_output",
    "cold": "fresh_native_graph_lazy_component_query_normalize_drop_build_excluded",
    "first": "resident_native_hypergraph_to_projection_weight_binding_full_route_drop",
    "profile": "resident_native_hypergraph_rank_same_call_phase_observation",
}
OPERATIONS = ("components", "reachable", "distance", "path", "astarpath", "unitpath", "tour", "pagerank")


class OperationLimit(RuntimeError):
    pass


class QueryDeadline:
    """Preempt Python query batches on Linux; ProcessRunner owns the outer cap."""
    def __enter__(self):
        self.enabled = hasattr(signal, "setitimer")
        if self.enabled:
            self.previous = signal.signal(signal.SIGALRM, self.expired)
            signal.setitimer(signal.ITIMER_REAL, 15)
        return self

    @staticmethod
    def expired(_signal, _frame):
        raise OperationLimit("query batch exceeded 15 seconds")

    def __exit__(self, *_error):
        if self.enabled:
            signal.setitimer(signal.ITIMER_REAL, 0)
            signal.signal(signal.SIGALRM, self.previous)


class CanonicalCase:
    def __init__(self, data: dict[str, Any], source_hash: str):
        self.data = data
        self.name = data["family"]
        self.n = int(data["nodes"])
        self.edges = data["memberships"]
        self.source_hash = source_hash
        if self.n < 0 or len(self.edges) != data["arcs"]:
            raise ValueError("invalid declared shape")
        if sum(map(len, self.edges)) != data["incidences"]:
            raise ValueError("incidence count mismatch")
        if any(type(v) is not int or not 0 <= v < self.n for edge in self.edges for v in edge):
            raise ValueError("node outside declared domain")
        self.active = {v for edge in self.edges for v in edge}
        self.identity = np.arange(self.n, dtype=np.int64)
        labels = np.asarray(data["labels"], dtype=np.int64)
        if labels.shape != (self.n,) or (self.n and (labels.min() < 0 or labels.max() >= self.n)):
            raise ValueError("invalid reference partition")
        if len(set(map(int, labels))) != data["components"]:
            raise ValueError("reference component count mismatch")
        minima = np.full(self.n, self.n, dtype=np.int64)
        np.minimum.at(minima, labels, self.identity)
        self.expected = minima[labels]
        self.checksum = self.hash_values(self.expected)
        self.reachable = np.zeros(self.n, dtype=np.int64)
        self.reachable[data.get("reachable", [])] = 1
        self.checksums = {"components": self.checksum,
                          "reachable": self.hash_values(np.flatnonzero(self.reachable)),
                          "distance": self.hash_values([data["shortest_hops"] if data.get("shortest_hops") is not None else 2**64-1])}

    @staticmethod
    def hash_values(values):
        result = 0xcbf29ce484222325
        for byte in np.asarray(values, dtype="<u8").tobytes():
            result = ((result ^ byte) * 0x100000001b3) & (2**64-1)
        return str(result)

    @classmethod
    def load(cls, path: Path, workload: str) -> CanonicalCase:
        raw = path.read_bytes()
        value = json.loads(raw)
        if value.get("representation") != "canonical Core Ir":
            raise ValueError("input must come from the canonical IR exporter")
        candidates = [row for row in value["cases"] if row["family"] == workload]
        if len(candidates) != 1:
            raise ValueError("workload must identify exactly one exported case")
        return cls(candidates[0], hashlib.sha256(raw).hexdigest())


class ComponentBackend(ABC):
    name: str
    preserves_isolates = True

    def __init__(self, case: CanonicalCase):
        self.case = case
        self.module = importlib.import_module(self.name)
        self.version = importlib.metadata.version(self.name)
        if self.version != VERSIONS[self.name]:
            raise ValueError(f"{self.name} version drift: {self.version}")
        self.output = np.empty(case.n, dtype=np.int64)
        self.graph = self.build()
        actual = set(map(int, self.nodes()))
        expected = set(range(case.n)) if self.preserves_isolates else case.active
        if actual != expected:
            raise ValueError(f"{self.name} changed the native vertex domain")
        self.completed_isolates = case.n-len(actual)

    @abstractmethod
    def build(self): ...

    @abstractmethod
    def nodes(self): ...

    @abstractmethod
    def components(self): ...

    @abstractmethod
    def edge_count(self) -> int: ...

    def reachable_nodes(self):
        raise NotImplementedError

    def distance(self):
        raise NotImplementedError

    def query(self, operation="components", materialize=True) -> int:
        if operation == "distance":
            distance = self.distance()
            return -1 if distance is None or not math.isfinite(distance) else int(distance)
        if operation == "reachable":
            component = self.reachable_nodes()
            count = len(component)
            if materialize:
                self.output.fill(0)
                self.output[np.fromiter(component, dtype=np.int64)] = 1
            return count
        # All adapters use the same reusable normalization destination. Known
        # isolates omitted by HyperNetX remain singleton labels, without adding
        # dummy hyperedges or changing any transition semantics.
        np.copyto(self.output, self.case.identity)
        count = self.completed_isolates
        for component in self.components():
            count += 1
            if len(component) > 1:
                vertices = np.fromiter(component, dtype=np.int64, count=len(component))
                self.output[vertices] = vertices.min()
        return count

    def verify(self, count: int, operation="components") -> None:
        if operation == "distance":
            expected = self.case.data["shortest_hops"]
            if count != (-1 if expected is None else expected):
                raise ValueError(f"{self.name} distance mismatch")
            return
        if operation == "reachable":
            if count != int(self.case.reachable.sum()) or not np.array_equal(self.output, self.case.reachable):
                raise ValueError(f"{self.name} reachable set mismatch")
            return
        if count != self.case.data["components"] or not np.array_equal(self.output, self.case.expected):
            raise ValueError(f"{self.name} full partition mismatch")

    def implementation_hash(self) -> str:
        path = Path(inspect.getfile(type(self.graph))) if self.name != "reticula" else Path(importlib.import_module("reticula._reticula_ext").__file__)
        return hashlib.sha256(path.read_bytes()).hexdigest()


class HyperNetXBackend(ComponentBackend):
    name = "hypernetx"
    preserves_isolates = False

    def build(self):
        return self.module.Hypergraph(dict(enumerate(self.case.edges)))

    def nodes(self):
        return self.graph.nodes

    def components(self):
        if not self.case.active:
            return iter(())
        return self.graph.connected_components(edges=False)

    def reachable_nodes(self):
        if 0 not in self.case.active:
            return [0]
        import networkx as nx
        return nx.node_connected_component(self.graph.get_linegraph(s=1, edges=False), 0)

    def distance(self):
        if self.case.n == 1:
            return 0
        if 0 not in self.case.active or self.case.n-1 not in self.case.active:
            return None
        return self.graph.distance(0, self.case.n-1, s=1)

    def edge_count(self):
        return len(self.graph.edges)


class XGIBackend(ComponentBackend):
    name = "xgi"

    def build(self):
        graph = self.module.Hypergraph(dict(enumerate(self.case.edges)))
        graph.add_nodes_from(range(self.case.n))
        return graph

    def nodes(self):
        return self.graph.nodes

    def components(self):
        return self.module.connected_components(self.graph)

    def reachable_nodes(self):
        return self.module.node_connected_component(self.graph, 0)

    def distance(self):
        return self.module.single_source_shortest_path_length(self.graph, 0)[self.case.n-1]

    def edge_count(self):
        return len(self.graph.edges)


class HypergraphxBackend(ComponentBackend):
    name = "hypergraphx"

    def build(self):
        graph = self.module.Hypergraph(self.case.edges, weighted=False)
        graph.add_nodes(range(self.case.n))
        return graph

    def nodes(self):
        return self.graph.get_nodes()

    def components(self):
        return self.graph.connected_components()

    def reachable_nodes(self):
        return self.graph.node_connected_component(0)

    def distance(self):
        import networkx as nx
        from hypergraphx.representations.projections import clique_projection
        if not hasattr(self, "search_graph"):
            self.search_graph = clique_projection(self.graph, keep_isolated=True)
        try:
            return nx.shortest_path_length(self.search_graph, 0, self.case.n-1)
        except nx.NetworkXNoPath:
            return None

    def edge_count(self):
        return len(self.graph.get_edges())


class ReticulaBackend(ComponentBackend):
    name = "reticula"

    def build(self):
        rt = self.module
        edges = [rt.undirected_hyperedge[rt.int64](row) for row in self.case.edges]
        return rt.undirected_hypernetwork[rt.int64](edges, list(range(self.case.n)))

    def nodes(self):
        return self.graph.vertices()

    def components(self):
        return self.module.connected_components(self.graph, True)

    def reachable_nodes(self):
        return self.module.connected_component(self.graph, 0)

    def distance(self):
        return self.module.shortest_path_lengths_from(self.graph, 0).get(self.case.n-1)

    def edge_count(self):
        return len(self.graph.edges())


BACKENDS = {cls.name: cls for cls in [HyperNetXBackend, XGIBackend, HypergraphxBackend, ReticulaBackend]}


class ComposedBackend:
    """Shared native graph ownership for explicitly attributed compositions."""
    def __init__(self, case, backend, factory=None):
        self.case = case
        self.native = (factory or BACKENDS[backend])(case)
        self.name = backend
        self.version = self.native.version
        self.completed_isolates = self.native.completed_isolates

    def nodes(self): return self.native.nodes()
    def edge_count(self): return self.native.edge_count()
    def implementation_hash(self): return self.native.implementation_hash()


class ExternalCell:
    def __init__(self, case: CanonicalCase, backend: str, mode: str, operation="components"):
        if mode not in MODES:
            raise ValueError("unknown cache policy")
        if operation not in OPERATIONS:
            raise ValueError("unknown operation")
        if mode == "profile" and operation != "pagerank":
            raise ValueError("phase observation is restricted to PageRank")
        if (operation in {"path","astarpath","unitpath","tour","pagerank"}) != (mode in {"first", "profile"}):
            raise ValueError("full routes require the first-result boundary")
        self.case, self.backend, self.mode = case, backend, mode
        self.operation = operation
        self.checksum = case.checksums.get(operation, "pending_route_verification")

    def run(self) -> None:
        if self.mode == "profile" and os.environ.get("HYMEKO_PHASE_BENCH") != "1":
            raise SystemExit("phase measurement disabled; explicit HYMEKO_PHASE_BENCH=1 required")
        scope = SCOPES[self.mode].replace("component", self.operation)
        if self.operation == "pagerank":
            scope = "resident_native_hypergraph_incidence_export_normalize_scipy_gmres_full_vector_drop"
        if self.operation != "components":
            scope = scope.replace("normalize_drop_reused_output", "result_drop")
        self.emit("INPUT", fixture_sha256=self.case.source_hash, construction_timed="false",
                  metric_scope=scope, cache_policy=self.mode)
        try:
            self.measure()
        except OperationLimit as error:
            self.emit_na(str(error).replace("\t", " ").replace("\n", " "))

    def measure(self) -> None:
        case, mode = self.case, self.mode
        started = time.perf_counter()
        routing = self.operation in {"path", "astarpath", "unitpath", "tour"}
        ranking = self.operation == "pagerank"
        if ranking:
            from ranking import RankingBackend, PhaseObserver
            adapter = RankingBackend(case,self.backend,PhaseObserver() if mode == "profile" else None)
        elif routing:
            from routing import RoutingBackend
            adapter = RoutingBackend(case,self.backend)
        else:
            adapter = BACKENDS[self.backend](case)
        construction_s = time.perf_counter()-started
        started = time.perf_counter()
        with QueryDeadline():
            count = adapter.query(self.operation)
        first_query_s = time.perf_counter()-started
        adapter.verify(count, self.operation)
        if ranking:
            self.checksum = count.checksum
            self.emit("EXTERNAL_RANK", solver="scipy_gmres", solver_version=importlib.metadata.version("scipy"),
                numpy_version=importlib.metadata.version("numpy"), flow="undirected", weights="unit",
                self_return="true", personalization="uniform", damping=adapter.damping,
                tolerance=adapter.tolerance, max_iterations=adapter.max_iterations,
                iterations=count.iterations, matvecs=count.matvecs, converged="true",
                residual_bound=f"{adapter.verified_bound:.17g}", reference_l1=f"{adapter.reference_l1:.17g}",
                mass=f"{float(count.scores.sum()):.17g}", score_hash=count.checksum,
                native_edges=adapter.edge_count(), canonical_edges=case.data["arcs"],
                composition=self.strategy())
        if routing:
            self.checksum = adapter.route_hash(count)
            native_unit = self.operation == "unitpath" and self.backend in {"xgi", "reticula"}
            self.emit("ROUTE", kind=self.operation, verified="true", closed=str(self.operation=="tour").lower(),
                visited=len(set(count.vertices)), hops=len(count.arcs), cost=count.cost,
                vertices=json.dumps(count.vertices,separators=(",",":")),arc_ids=json.dumps(count.arcs,separators=(",",":")),
                solver=self.backend if native_unit else "networkx",
                solver_version=adapter.version if native_unit else importlib.metadata.version("networkx"),
                composition=self.strategy())
        source_sha = adapter.implementation_hash()
        self.emit("EXTERNAL", backend=self.backend, version=adapter.version, mode=mode,
                  full_result="true", declared_isolates=case.n-len(case.active),
                  completed_isolates=adapter.completed_isolates, native_nodes=len(list(adapter.nodes())),
                  native_edges=adapter.edge_count(), implementation_sha256=source_sha,
                  python=sys.version.split()[0], interface="python_api", checksum=self.checksum,
                  native_pagerank="composed_scipy_gmres" if ranking else "not_matched", operation=self.operation,
                  query_strategy=self.strategy(),
                  construction_us=f"{construction_s*1e6:.3f}",
                  first_query_us=f"{first_query_s*1e6:.3f}")
        if first_query_s > 15:
            self.emit_na("first_query_exceeded_15_seconds")
            return
        with QueryDeadline():
            started = time.perf_counter()
            count = adapter.query(self.operation, materialize=False)
            calibration_s = time.perf_counter()-started
        checked = adapter.query(self.operation)
        adapter.verify(checked, self.operation)
        if checked != count:
            raise ValueError("calibration result drift")
        fresh_input = mode == "cold" or self.operation == "unitpath"
        repetitions = 1 if fresh_input else min(262144, max(1, math.ceil(.25/max(calibration_s, 1e-9))))
        predicted = 11 * ((construction_s + first_query_s) if fresh_input else repetitions*calibration_s)
        predicted += 11*calibration_s + construction_s + first_query_s + 20
        if predicted > 160:
            self.emit_na("predicted_process_budget_exceeds_160_seconds")
            return
        observations = []
        phase_observations = []
        observer = getattr(adapter, "observer", None)
        for ordinal in range(-2, 9):
            if self.operation == "unitpath":
                # Fresh native input outside timing prevents a cached HNX
                # linegraph from turning a first-result query into a warm one.
                adapter = RoutingBackend(case,self.backend)
            elif mode == "cold":
                adapter = BACKENDS[self.backend](case)
            if observer is not None:
                observer.reset()
            with QueryDeadline():
                cpu_start = time.process_time_ns()
                wall_start = time.perf_counter_ns()
                for _ in range(repetitions):
                    count = adapter.query(self.operation, materialize=False)
                wall = (time.perf_counter_ns()-wall_start)/1000/repetitions
                cpu = (time.process_time_ns()-cpu_start)/1000/repetitions
            if observer is not None and ordinal >= 0:
                phase_observations.append((ordinal, observer.means_us(repetitions)))
            with QueryDeadline():
                checked = adapter.query(self.operation)
            adapter.verify(checked, self.operation)
            if checked != count:
                raise ValueError("timed result drift")
            if wall > 15e6:
                self.emit_na("operation_exceeded_15_seconds")
                return
            if ordinal >= 0:
                observations.append((ordinal, wall, cpu))
        for ordinal, phases in phase_observations:
            self.emit("STAGE_SAMPLE", ordinal=ordinal, repetitions=repetitions,
                      **{name:f"{value:.6f}" for name,value in phases.items()},
                      inner_total_us=f"{sum(phases.values()):.6f}", boundary="same_call_partition")
        for ordinal, wall, cpu in observations:
            self.emit("SAMPLE", run=os.environ.get("EVIDENCE_RUN", "external"), section="external_graph",
                      subject=f"{self.backend}-{self.operation}-{mode}", workload=case.name, threads=1, ordinal=ordinal,
                      repetitions=repetitions, wall_us=f"{wall:.3f}", cpu_us=f"{cpu:.3f}", checksum=self.checksum)
        walls = sorted(row[1] for row in observations)
        cpus = sorted(row[2] for row in observations)
        self.emit("CELL", run=os.environ.get("EVIDENCE_RUN", "external"), section="external_graph",
                  subject=f"{self.backend}-{self.operation}-{mode}", workload=case.name, status="OK", threads=1,
                  nodes=case.n, arcs=case.data["arcs"], incidences=case.data["incidences"],
                  warmups=2, samples=9, repetitions=repetitions, checksum=self.checksum,
                  wall_median_us=f"{walls[4]:.3f}", wall_q1_us=f"{walls[2]:.3f}",
                  wall_q3_us=f"{walls[6]:.3f}", wall_worst_us=f"{walls[8]:.3f}",
                  cpu_median_us=f"{cpus[4]:.3f}", cpu_q1_us=f"{cpus[2]:.3f}",
                  cpu_q3_us=f"{cpus[6]:.3f}", cpu_worst_us=f"{cpus[8]:.3f}")

    def emit_na(self, reason):
        self.emit("CELL", section="external_graph", subject=f"{self.backend}-{self.operation}-{self.mode}",
                  workload=self.case.name, status="NA", reason=reason, checksum=self.checksum,
                  nodes=self.case.n, arcs=self.case.data["arcs"], incidences=self.case.data["incidences"])

    def strategy(self):
        if self.operation == "pagerank":
            return "native_incidence_factorized_scipy_gmres_with_explicit_edge_multiplicity"
        if self.operation == "unitpath":
            return {"hypernetx":"native_linegraph_networkx_bidirectional_bfs_full_witness",
                    "hypergraphx":"native_clique_networkx_bidirectional_bfs_full_witness",
                    "xgi":"native_unweighted_sssp_incidence_reconstruction",
                    "reticula":"native_cpp_unweighted_sssp_incidence_reconstruction"}[self.backend]
        if self.operation in {"path","astarpath","tour"}:
            return "native_incidence_projection_networkx_"+{"path":"bidirectional_dijkstra","astarpath":"astar","tour":"metric_christofides"}[self.operation]
        if self.backend == "hypernetx":
            if self.operation == "distance" and self.case.n == 1:
                return "same_vertex_identity_short_circuit"
            if ((self.operation == "reachable" and 0 not in self.case.active)
                    or (self.operation == "distance" and
                        (0 not in self.case.active or self.case.n-1 not in self.case.active))):
                return "declared_domain_isolate_short_circuit"
        if self.operation == "reachable" and self.backend == "hypernetx":
            return "hypernetx_linegraph_networkx_component"
        if self.operation == "distance":
            return {"hypernetx": "native_target_distance_linegraph",
                    "hypergraphx": "hypergraphx_clique_projection_networkx_target_distance",
                    "xgi": "native_full_sssp_then_target_lookup",
                    "reticula": "native_full_sssp_then_target_lookup"}[self.backend]
        return "native_api"

    @staticmethod
    def emit(record_type, **values):
        print("EVIDENCE_"+record_type+"\t"+"\t".join(f"{key}={value}" for key,value in values.items()), flush=True)


if __name__ == "__main__":
    if "--describe" in sys.argv:
        packages = {}
        for name in VERSIONS:
            module = importlib.import_module(name)
            packages[name] = dict(version=importlib.metadata.version(name),
                rank_named_public_exports=[n for n in dir(module) if not n.startswith("_") and "rank" in n.lower()],
                native_pagerank="no_matching_public_call_found",
                python_api=True,
                components="native; declared isolates completed externally" if name=="hypernetx" else "native",
                reachable="linegraph + NetworkX node component" if name=="hypernetx" else "native",
                distance={"hypernetx":"native distance via cached node linegraph",
                          "xgi":"native full single-source distances; then target lookup",
                          "hypergraphx":"library clique_projection + NetworkX shortest_path_length",
                          "reticula":"native C++ full single-source distances; then target lookup"}[name])
        print(json.dumps(dict(python=sys.version, packages=packages,
            pagerank_note="Hypergraphx RW_stationary_state has no teleportation, zeroes the diagonal and reweights hyperedges; it is not the matched PageRank operation."),indent=2))
        raise SystemExit(0)
    if "external_cell" not in sys.argv:
        import unittest
        import test_external
        import test_ranking
        suite = unittest.TestSuite(unittest.defaultTestLoader.loadTestsFromModule(module)
                                   for module in (test_external,test_ranking))
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        raise SystemExit(not result.wasSuccessful())
    if os.environ.get("HYMEKO_EXTERNAL_BENCH") != "1":
        raise SystemExit("external timing disabled; explicit campaign opt-in required")
    backend, operation, mode = os.environ["EVIDENCE_OPERATION"].split("-")
    ExternalCell(CanonicalCase.load(Path(os.environ["HYMEKO_EXTERNAL_INPUT"]), os.environ["EVIDENCE_WORKLOAD"]), backend, mode, operation).run()
