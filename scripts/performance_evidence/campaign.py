"""Run the existing Rust evidence clients on Windows, Linux, or Docker ARM64.

No graph implementation lives here. Each cell is a fresh process; the Rust
client owns fixtures, warmups, sampling, checksums and algorithm calls.
"""

from __future__ import annotations

import argparse
import csv
import ctypes
import hashlib
import json
import math
import os
import platform
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

if __package__:
    from .host import HostBusy, QuietHostGuard
else:
    from host import HostBusy, QuietHostGuard

PHASES = (
    (
        "generate_ir",
        "generation",
        "Deterministic generator including direct IR construction",
    ),
    (
        "emit_source",
        "serialization",
        "Existing fixture IR to resident HyMeKo UTF-8 text",
    ),
    ("read_source", "input", "Local HyMeKo file to resident UTF-8 text; warm OS cache"),
    ("parse_ast", "parsing", "Resident HyMeKo UTF-8 text to AST"),
    (
        "ast_to_ir",
        "graph_build",
        "Existing AST through interning, resolution and IR lowering",
    ),
    (
        "compile",
        "compilation",
        "Resident text through fresh ModuleStore compilation, validation and hashes",
    ),
    ("incidence_index", "preparation", "Compiled IR to incidence index"),
    (
        "dijkstra_prepared",
        "execution",
        "Dijkstra on compiled IR and ready index; fresh query workspace",
    ),
    (
        "source_to_dijkstra",
        "end_to_end_memory",
        "Resident text through fresh compilation, index and Dijkstra",
    ),
    (
        "file_to_dijkstra",
        "end_to_end_file",
        "Local file read, fresh compilation, index and Dijkstra; warm OS cache",
    ),
)

COMPARISON_WORKLOADS = (
    "rank2_1m", "rank32_1m", "rank1000_1m",
    "mixed_1m", "hub32_1m", "components8_1m", "chain65k", "scale_small", "scale_large",
)


GENERATED_WORKLOADS = ("steiner255", "complete32_3", "sunflower1024", "mycielski7")
ASSISTED_WORKLOADS = ("grid200_d80", "map128", "transport128")
ASSISTED_SUBJECTS = ("dijkstra", "astar", "assisted", "prepare", "first_dijkstra", "first_astar", "first_assisted")
ROUTING_WORKLOADS = ASSISTED_WORKLOADS + ("tour16",)
SCALING_WORKLOADS = ("grid512_d511", "map256", "transport256_off", "tour24", "tour32", "tour64", "tour64_corridor")
EXTERNAL_FRAMEWORKS = ("hypernetx", "xgi", "hypergraphx", "reticula")
EXTERNAL_OPERATIONS = ("components", "reachable", "distance", "path", "astarpath", "unitpath", "tour", "pagerank")
RANKING_WORKLOADS = tuple(w + suffix for w in ("chain65k","rank2_1m","hub32_1m","components8_1m")
                         for suffix in ("","@17"))
TRAVERSAL_SUBJECTS = ("reachable", "dual_reachable", "berge_path", "dijkstra",
                      "generate_ir", "incidence_index", "levi", "s_components2")

@dataclass(frozen=True)
class Cell:
    suite: str
    test: str
    workload: str
    subject: str
    threads: int = 1

    @property
    def key(self) -> str:
        return f"{self.test.replace('::', '_')}-{self.workload}-{self.subject}-t{self.threads}"

    def environment(self, run: int) -> dict[str, str]:
        prefix = "INFORMED_BERGE" if self.suite == "informed" else "EVIDENCE"
        result = {
            f"{prefix}_RUN": f"run{run}-{self.key}",
            f"{prefix}_WORKLOAD": self.workload,
            "RAYON_NUM_THREADS": "4",
        }
        if self.suite == "informed":
            result[f"{prefix}_SUBJECT"] = self.subject
        else:
            result["EVIDENCE_OPERATION"] = self.subject
            result["EVIDENCE_THREADS"] = str(self.threads)
        if self.test == "ranking::ranking_cell":
            result["HYMEKO_RANK_BENCH"] = "1"
            result["RAYON_NUM_THREADS"] = "1"
        if self.test == "native::native_cell":
            result["HYMEKO_NATIVE_BENCH"] = "1"
            result["RAYON_NUM_THREADS"] = "1"
        if self.test in {"assisted::assisted_cell", "assisted::tour_cell"}:
            result.update({"HYMEKO_ASSISTED_BENCH": "1", "RAYON_NUM_THREADS": "1"})
        if self.suite == "external":
            result.update({"HYMEKO_EXTERNAL_BENCH": "1", "RAYON_NUM_THREADS": "1",
                           "OPENBLAS_NUM_THREADS": "1", "OMP_NUM_THREADS": "1",
                           "MKL_NUM_THREADS": "1", "NUMEXPR_NUM_THREADS": "1"})
        if self.test in {"memory_cell", "native::memory::memory_cell"}:
            result.update({"HYMEKO_MEMORY_BENCH": "1", "RAYON_NUM_THREADS": "1"})
        return result

    @staticmethod
    def select(suite: str = "graph", phases: list[str] | None = None,
               subjects: list[str] | None = None, smoke: bool = False,
               workloads: list[str] | None = None) -> list[Cell]:
        """Select an explicit subset of the existing matrix, never another runner."""
        cells = Cell.matrix(suite, phases, workloads)
        if subjects is not None:
            known = set(TRAVERSAL_SUBJECTS) if suite == "traversal" else {cell.subject for cell in cells}
            if not subjects or len(set(subjects)) != len(subjects) or not set(subjects) <= known:
                raise ValueError("subjects must be nonempty, unique members of the selected suite")
            if suite == "traversal":
                chosen_workloads = list(dict.fromkeys(cell.workload for cell in cells))
                cells = [Cell("algorithm", "ranking::ranking_cell", workload, subject)
                         for workload in chosen_workloads for subject in subjects]
            else:
                cells = [cell for cell in cells if cell.subject in subjects]
        if not smoke or workloads is not None:
            return cells
        if suite == "parsing":
            return [c for c in cells if (
                c.workload in {"map512", "random1e6"}
                and c.subject in (subjects or phases or ["compile"])
            ) or (c.workload, c.subject) == ("grid5", "parse_ast")]
        if suite in {"ranking", "comparison"}:
            return [c for c in cells if c.workload == "rank32_1m"]
        return [cells[0], cells[-1]] if len(cells) > 1 else cells

    @staticmethod
    def matrix(suite: str = "graph", phases: list[str] | None = None,
               workloads: list[str] | None = None) -> list[Cell]:
        if workloads is not None and (
            suite not in {"comparison", "traversal", "external", "external_ranking", "memory", "assisted", "routing", "routing_unit", "routing_scale"} or not workloads or len(set(workloads)) != len(workloads)
            or not set(workloads) <= set(RANKING_WORKLOADS if suite in {"external_ranking", "memory"} else SCALING_WORKLOADS if suite=="routing_scale" else ("grid200_d80",) if suite=="routing_unit" else ROUTING_WORKLOADS if suite=="routing" else ASSISTED_WORKLOADS if suite=="assisted" else COMPARISON_WORKLOADS + GENERATED_WORKLOADS)
        ):
            raise ValueError("workloads must be nonempty unique names from the comparison/traversal catalog")
        if suite == "assisted":
            return [Cell("informed", "assisted::assisted_cell", w, s)
                    for w in (workloads or ASSISTED_WORKLOADS) for s in ASSISTED_SUBJECTS]
        if suite in {"routing", "routing_scale"}:
            cells=[]
            for w in workloads or (SCALING_WORKLOADS if suite=="routing_scale" else ROUTING_WORKLOADS):
                if w.startswith("tour"):
                    cells.append(Cell("informed","assisted::tour_cell",w,"christofides"))
                    operations=("tour",)
                else:
                    cells.extend(Cell("informed","assisted::assisted_cell",w,s) for s in ("first_dijkstra","first_astar","first_assisted"))
                    operations=("path","astarpath")
                cells.extend(Cell("external","external_cell",w,f"{b}-{op}-first") for b in EXTERNAL_FRAMEWORKS for op in operations)
                if w == "grid512_d511":
                    cells.append(Cell("external","external_cell",w,"reticula-unitpath-first"))
            return cells
        if suite == "routing_unit":
            return [Cell("external","external_cell","grid200_d80",f"{b}-unitpath-first") for b in EXTERNAL_FRAMEWORKS]
        if suite == "memory":
            selected = workloads or RANKING_WORKLOADS
            return ([Cell("external", "memory_cell", w, b) for w in selected for b in EXTERNAL_FRAMEWORKS]
                    + [Cell("algorithm", "native::memory::memory_cell", w, b)
                       for w in selected for b in ("hymeko", "hypergraphz")])
        if suite == "external_ranking":
            selected = workloads or RANKING_WORKLOADS
            return ([Cell("external","external_cell",w,f"{b}-pagerank-first")
                     for w in selected for b in EXTERNAL_FRAMEWORKS]
                    + [Cell("algorithm","native::native_cell",w,s)
                       for w in selected for s in ("rank_hymeko","rank_hypergraphz")])
        if suite == "external":
            selected = workloads or ["chain65k", "hub32_1m", "components8_1m", *GENERATED_WORKLOADS]
            foreign = [Cell("external", "external_cell", w, f"{backend}-{operation}-{mode}")
                       for w in selected for backend in EXTERNAL_FRAMEWORKS
                       for operation in ("components","reachable","distance") for mode in ("prepared", "cold")]
            native = [Cell("algorithm", "native::native_cell", w, subject)
                      for w in selected for subject in ("components_hymeko", "components_hypergraphz")]
            searches = [Cell("algorithm", "ranking::ranking_cell", w, subject)
                        for w in selected for subject in ("reachable", "berge_path", "dijkstra")]
            return foreign + native + searches
        if suite == "traversal":
            return [Cell("algorithm", "ranking::ranking_cell", workload, operation)
                    for workload in (workloads if workloads is not None else
                                     ("scale_small", "hub32_1m", "components8_1m", "chain65k"))
                    for operation in ("reachable", "dual_reachable", "berge_path", "dijkstra")]
        if suite == "comparison":
            return [
                Cell("algorithm", "native::native_cell", workload, operation)
                for workload in (workloads if workloads is not None else COMPARISON_WORKLOADS[:3])
                for operation in ("rank_hymeko", "rank_hypergraphz", "rank_prepared",
                                  "components_hymeko", "components_hypergraphz")
            ]
        if suite == "ranking":
            return [
                Cell("algorithm", "ranking::ranking_cell", workload, operation)
                for workload in ("rank2_1m", "rank32_1m", "rank1000_1m", "disjoint_1m")
                for operation in ("prepare", "solve", "reachable")
            ]
        if suite == "parsing":
            parsing_workloads = (
                "grid200_d80",
                "map32",
                "map128",
                "map512",
                "corridor128",
                "transport128",
                "path65k",
                "grid256",
                "sts1023",
                "random1e6",
                "layered10",
                "grid6",
                "grid5",
                "sts31",
                "parallel16",
                "sts511",
            )
            selected = phases or (
                "parse_ast",
                "ast_to_ir",
                "compile",
                "incidence_index",
                "source_to_dijkstra",
            )
            if any(p not in {item[0] for item in PHASES} for p in selected):
                raise ValueError("unknown parsing phase")
            return [
                Cell("informed", "parsing::parsing_cell", w, p)
                for w in parsing_workloads
                for p in selected
            ]
        if suite != "graph":
            raise ValueError(f"unknown suite: {suite}")
        cells = []
        for distance in (20, 40, 80):
            for subject in ("dijkstra_unit", "astar_l1", "astar_l2"):
                cells.append(
                    Cell("informed", "search_cell", f"grid200_d{distance}", subject)
                )
        for workload in ("map32", "map128", "map512", "corridor128", "transport128"):
            subjects = (
                ("dijkstra_weight", "astar_discrete")
                if workload == "transport128"
                else (
                    "dijkstra_weight",
                    "astar_metric",
                    "astar_discrete",
                    "astar_combined",
                )
            )
            cells.extend(Cell("informed", "search_cell", workload, s) for s in subjects)
            cells.append(
                Cell("informed", "preparation_cell", workload, "discrete_hops")
            )
        for workload in ("path65k", "grid256", "sts1023", "random1e6", "map512"):
            cells.extend(
                Cell("informed", "shortest_cell", workload, s)
                for s in ("bfs_shortest", "dijkstra_unit")
            )
        for workload, subject in (
            ("layered10", "paths_goal"),
            ("grid6", "paths_goal"),
            ("grid5", "cycles"),
            ("sts31", "paths_all"),
            ("sts31", "cycles"),
            ("parallel16", "cycles"),
        ):
            cells.append(Cell("informed", "enumeration_cell", workload, subject))
        for workload in ("path65k", "sts511"):
            cells.extend(
                Cell("algorithm", "operation_cell", workload, s)
                for s in ("reachable", "components", "rank2", "levi")
            )
        cells.extend(
            Cell("algorithm", "operation_cell", "sts511", "independent_query_batch", t)
            for t in (1, 2, 4)
        )
        return cells


class Evidence:
    @staticmethod
    def verify_assisted(records: dict[str, list[dict[str, str]]], cell: dict[str, str]) -> None:
        source=Evidence.one(records,"INPUT")
        result=Evidence.one(records,"RESULT")
        subject=cell["subject"]
        first=subject.startswith("first_")
        prep=subject=="prepare"
        if source.get("pipeline")!="berge_bound_assist" or source.get("verified")!="true" or source.get("flow")!="undirected":
            raise ValueError("missing assisted-search validation")
        expected={"construction_timed":"false","index_timed":str(first).lower(),
                  "caller_setup_timed":str(first).lower(),"goal_preparation_timed":str(prep or subject=="first_assisted").lower()}
        scope=("resident_ir_to_index_caller_setup_search_and_drop" if first else
               "prepared_index_to_goal_certificate_and_drop_reused_traversal_workspace" if prep else
               "prepared_index_bounds_reused_search_workspace_query_result_drop")
        if any(source.get(k)!=v for k,v in expected.items()) or source.get("metric_scope")!=scope:
            raise ValueError("assisted-search timing boundary drift")
        nodes=int(cell["nodes"])
        if not all(0<=int(source[k])<nodes for k in ("start","goal")):
            raise ValueError("assisted-search endpoint outside IR")
        if prep:
            if result.get("items")!=str(nodes) or cell["section"]!="heuristic_preparation":
                raise ValueError("invalid prepared-goal result")
        elif result.get("cost_micros")!=source.get("expected_cost_micros") or cell["section"]!="informed_search":
            raise ValueError("assisted-search optimal cost mismatch")

    @staticmethod
    def one(records: dict[str, list[dict[str, str]]], kind: str) -> dict[str, str]:
        values = records.get(kind, [])
        if len(values) != 1:
            raise ValueError(f"expected one {kind} record")
        return values[0]

    @staticmethod
    def verify_query(records: dict[str, list[dict[str, str]]], cell: dict[str, str]) -> None:
        inputs, queries = records.get("INPUT", []), records.get("QUERY", [])
        if len(inputs) != 1 or inputs[0].get("construction_timed") != "false" or not inputs[0].get("metric_scope") or len(queries) != 1:
            raise ValueError("missing query input/boundary/result")
        query = queries[0]
        kind = {"reachable": "nodes", "dual_reachable": "arcs", "berge_path": "path", "dijkstra": "path"}.get(cell["subject"])
        if query.get("kind") != kind or kind is None or query.get("checksum") != cell["checksum"]:
            raise ValueError("query kind/result mismatch")
        if query.get("flow") != "undirected" or query.get("weights") != "unit":
            raise ValueError("query semantic drift")
        nodes, arcs = int(cell["nodes"]), int(cell["arcs"])
        visited, hops, cost = int(query["visited"]), int(query["hops"]), float(query["cost"])
        if not 0 <= visited <= (arcs if kind == "arcs" else nodes) or not math.isfinite(cost):
            raise ValueError("query count/cost outside domain")
        if query.get("reachable") != str(visited > 0).lower():
            raise ValueError("query reachability mismatch")
        if query.get("start") != "0" or query.get("start_kind") != ("arc" if kind == "arcs" else "node"):
            raise ValueError("query start mismatch")
        if kind == "path":
            if not 0 <= int(query["goal_node"]) < nodes or not 0 <= hops < nodes or cost != hops or visited != (hops + 1 if visited else 0) or (visited == 0 and hops != 0):
                raise ValueError("unit path cost/hop mismatch")
        elif visited == 0 or hops != 0 or cost != 0 or query.get("goal_node") != "none":
            raise ValueError("invalid reachable-set evidence")

    @staticmethod
    def verify_shape(records: dict[str, list[dict[str, str]]], cell: dict[str, str]) -> None:
        shapes = records.get("SHAPE", [])
        keys = {f"{kind}_{q}" for kind in ("arity", "degree") for q in ("min", "q1", "median", "q3", "max")}
        if len(shapes) != 1 or not keys.union({"isolated_nodes", "degree_sum"}) <= shapes[0].keys():
            raise ValueError("missing native shape statistics")
        shape = shapes[0]
        nodes, arcs, incidences = (int(cell[k]) for k in ("nodes", "arcs", "incidences"))
        for kind, limit in (("arity", nodes), ("degree", arcs)):
            values = [float(shape[f"{kind}_{q}"]) for q in ("min", "q1", "median", "q3", "max")]
            if not all(math.isfinite(v) and v.is_integer() and 0 <= v <= limit for v in values) or values != sorted(values):
                raise ValueError("invalid native shape distribution")
        isolated = int(shape["isolated_nodes"])
        if int(shape["degree_sum"]) != incidences or not 0 <= isolated <= nodes:
            raise ValueError("native shape cardinality mismatch")
        if nodes and ((isolated > 0) != (float(shape["degree_min"]) == 0)):
            raise ValueError("native isolated-node statistics mismatch")

    @staticmethod
    def verify_native(records: dict[str, list[dict[str, str]]], cell: dict[str, str]) -> None:
        source = Evidence.one(records, "INPUT")
        result = Evidence.one(records, "NATIVE")
        if source.get("construction_timed") != "false" or not source.get("metric_scope"):
            raise ValueError("missing native measurement boundary")
        if records.get("SHAPE") or source.get("shape_reported") == "true":
            Evidence.verify_shape(records, cell)
        if result.get("parity") != "true" or result.get("checksum") != cell["checksum"]:
            raise ValueError("native result/cell mismatch")
        if result["operation"] == "rank":
            for key, limit in (("vector_l1", 2e-10), ("hymeko_bound", 1.01e-10), ("hypergraphz_bound", 1.01e-10)):
                value = float(result[key])
                if not math.isfinite(value) or not 0 <= value <= limit:
                    raise ValueError("native PageRank parity/residual violation")
            if result.get("personalization") != "uniform" or result.get("tolerance") != "1e-10":
                raise ValueError("native PageRank semantic drift")

    @staticmethod
    def verify_algebra(records: dict[str, list[dict[str, str]]], cell: dict[str, str]) -> None:
        source = Evidence.one(records, "INPUT")
        if not source.get("metric_scope"):
            raise ValueError("missing algorithm provenance")
        subject = cell["subject"]
        construction = subject in {"generate_ir", "incidence_index"}
        proof = Evidence.one(records, "CONSTRUCTION" if construction else "ALGEBRA")
        if proof.get("verified") != "true" or proof.get("checksum") != cell["checksum"]:
            raise ValueError("missing verified algorithm result")
        if source.get("construction_timed") != str(subject == "generate_ir").lower() or source.get("index_timed") != str(subject == "incidence_index").lower():
            raise ValueError("algorithm measurement boundary mismatch")
        if construction:
            if cell["section"] != "construction" or proof.get("destruction_timed") != "false" or proof.get("full_validation") != "retained_result" or cell["checksum"] != source.get("fingerprint"):
                raise ValueError("construction verification/boundary mismatch")
        else:
            if cell["section"] != "algebra" or subject not in {"levi", "s_components2"}:
                raise ValueError("unknown algebra operation")
            limit = int(cell["arcs"]) + (int(cell["nodes"]) if subject == "levi" else 0)
            if not 0 <= int(proof["value"]) <= limit or not 0 <= int(proof["pair_work"]) <= 5_000_000 or proof.get("max_pair_work") != "5000000":
                raise ValueError("algebra result/work exceeds its domain")

    @staticmethod
    def verify_external(records: dict[str, list[dict[str, str]]], cell: dict[str, str]) -> None:
        source = Evidence.one(records, "INPUT")
        external = Evidence.one(records, "EXTERNAL")
        backend, operation, mode = cell["subject"].split("-")
        if backend not in EXTERNAL_FRAMEWORKS or operation not in EXTERNAL_OPERATIONS or mode not in {"prepared", "cold", "first"}:
            raise ValueError("unknown external query")
        if external.get("backend") != backend or external.get("operation") != operation or external.get("mode") != mode:
            raise ValueError("external query identity mismatch")
        if external.get("full_result") != "true" or external.get("checksum") != cell["checksum"]:
            raise ValueError("external result was not fully verified")
        if source.get("construction_timed") != "false" or source.get("cache_policy") != mode or not source.get("metric_scope"):
            raise ValueError("external measurement boundary drift")
        if len(source.get("fixture_sha256", "")) != 64 or len(external.get("implementation_sha256", "")) != 64:
            raise ValueError("missing external input/implementation hash")
        if not external.get("version") or not external.get("python") or external.get("interface") != "python_api":
            raise ValueError("missing external runtime identity")
        if int(cell["threads"]) != 1 or int(cell["warmups"]) != 2 or (mode == "cold" and int(cell["repetitions"]) != 1):
            raise ValueError("external sampling contract drift")
        nodes = int(cell["nodes"])
        completed = int(external["completed_isolates"])
        if not 0 <= completed <= int(external["declared_isolates"]) <= nodes or int(external["native_nodes"]) + completed != nodes:
            raise ValueError("external isolate/domain mismatch")
        if operation == "pagerank":
            rank = Evidence.one(records, "EXTERNAL_RANK")
            if (mode != "first" or rank.get("solver") != "scipy_gmres" or not rank.get("solver_version")
                    or rank.get("flow") != "undirected" or rank.get("weights") != "unit"
                    or rank.get("self_return") != "true" or rank.get("personalization") != "uniform"
                    or rank.get("converged") != "true" or rank.get("score_hash") != cell["checksum"]
                    or external.get("native_pagerank") != "composed_scipy_gmres"
                    or rank.get("composition") != external.get("query_strategy")):
                raise ValueError("external ranking attribution/semantics mismatch")
            numbers = [float(rank[k]) for k in ("damping","tolerance","residual_bound","reference_l1","mass")]
            if (not all(math.isfinite(v) for v in numbers) or numbers[0] != .85 or numbers[1] != 1e-10
                    or not 0 <= numbers[2] <= 1.01e-10 or not 0 <= numbers[3] <= 2.01e-10
                    or abs(numbers[4] - (1 if nodes else 0)) > 1e-10
                    or int(rank["max_iterations"]) != 200 or not 0 <= int(rank["iterations"]) <= 200
                    or int(rank["matvecs"]) < 0):
                raise ValueError("external ranking error/convergence contract drift")

    @staticmethod
    def parse(output: str) -> dict[str, list[dict[str, str]]]:
        records: dict[str, list[dict[str, str]]] = {}
        for line in output.splitlines():
            if "EVIDENCE_" not in line:
                continue
            parts = line[line.index("EVIDENCE_") :].split("\t")
            row = dict(part.split("=", 1) for part in parts[1:] if "=" in part)
            records.setdefault(parts[0].removeprefix("EVIDENCE_"), []).append(row)
        cells = records.get("CELL", [])
        if len(cells) != 1:
            raise ValueError(f"expected one CELL, got {len(cells)}")
        cell = cells[0]
        if cell.get("section") == "memory":
            from memory import MemoryEvidence
            MemoryEvidence.verify(records)
            return records
        samples = records.get("SAMPLE", [])
        if cell["status"] == "NA":
            if samples or not cell.get("reason"):
                raise ValueError("invalid not-applicable cell")
            return records
        if cell["status"] != "OK" or len(samples) != 9 or int(cell["samples"]) != 9:
            raise ValueError("not a successful nine-sample cell")
        if {int(s["ordinal"]) for s in samples} != set(range(9)):
            raise ValueError("duplicate/missing sample ordinal")
        if {s["checksum"] for s in samples} != {cell["checksum"]}:
            raise ValueError("sample/cell checksum drift")
        values = sorted(float(s["wall_us"]) for s in samples)
        if not all(math.isfinite(v) and v > 0 for v in values):
            raise ValueError("invalid sample duration")
        for field, value in zip(
            ("q1", "median", "q3", "worst"),
            (values[2], values[4], values[6], values[8]),
        ):
            if abs(float(cell[f"wall_{field}_us"]) - value) > 0.002:
                raise ValueError(f"raw/summary {field} mismatch")
        results = records.get("RESULT", [])
        if (
            cell.get("section")
            in {
                "informed_search",
                "berge_shortest",
                "berge_enumeration",
                "heuristic_preparation",
            }
            and len(results) != 1
        ):
            raise ValueError("missing informed-client result")
        if results and (
            len(results) != 1 or results[0]["path_hash"] != cell["checksum"]
        ):
            raise ValueError("result/cell mismatch")
        if cell.get("section") in {"informed_search","heuristic_preparation"} and cell.get("subject") in ASSISTED_SUBJECTS:
            Evidence.verify_assisted(records,cell)
        if cell.get("section") == "parsing":
            sources = records.get("SOURCE", [])
            if len(sources) != 1 or int(sources[0]["bytes"]) <= 0:
                raise ValueError("missing parsing input provenance")
            if (
                sources[0]["fnv1a64"] != cell["checksum"]
                or sources[0]["projection"] != "graph_fixture_v1"
            ):
                raise ValueError("parsing input identity mismatch")
            if cell.get("subject") in {"read_source", "file_to_dijkstra"} and (
                sources[0].get("file_io") != "true"
                or sources[0].get("file_cache") != "warm"
            ):
                raise ValueError("missing file I/O measurement boundary")
        if cell.get("section") == "pagerank":
            inputs = records.get("INPUT", [])
            if len(inputs) != 1 or inputs[0].get("construction_timed") != "false":
                raise ValueError("missing ranking fixture/boundary provenance")
            if cell["subject"] in {"prepare", "solve"}:
                ranks = records.get("RANK", [])
                if len(ranks) != 1:
                    raise ValueError("missing PageRank convergence evidence")
                rank = ranks[0]
                if (
                    rank["converged"] != "true"
                    or not math.isfinite(float(rank["error_bound_l1"]))
                    or not 0 <= float(rank["error_bound_l1"]) <= float(rank["tolerance"])
                    or abs(float(rank["mass"]) - 1.0) > 1e-9
                    or not math.isfinite(float(rank["mass"]))
                ):
                    raise ValueError("invalid PageRank convergence/mass")
                expected = rank["score_hash"] if cell["subject"] == "solve" else inputs[0]["fingerprint"]
                if expected != cell["checksum"]:
                    raise ValueError("ranking result/cell identity mismatch")
        if cell.get("section") == "native":
            Evidence.verify_native(records, cell)
        if cell.get("section") == "traversal":
            Evidence.verify_query(records, cell)
        if cell.get("section") in {"construction", "algebra"}:
            Evidence.verify_algebra(records, cell)
        if cell.get("section") == "external_graph":
            Evidence.verify_external(records, cell)
        return records


class ProcessRunner:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        self.binaries = {}
        if args.suite not in {"ranking", "comparison", "traversal", "external", "external_ranking", "memory", "routing_unit"}:
            self.binaries["informed"] = Path(args.informed).resolve()
        if args.suite in {"graph", "ranking", "comparison", "traversal"}:
            self.binaries["algorithm"] = Path(args.algorithm).resolve()
        if args.suite in {"external", "external_ranking", "memory", "routing", "routing_unit", "routing_scale"}:
            # Preserve the venv symlink: resolving it selects system Python and
            # silently discards the pinned framework environment on Linux.
            self.binaries["external"] = Path(args.external_python).absolute()
            if getattr(args, "algorithm", None):
                self.binaries["algorithm"] = Path(args.algorithm).resolve()
        self.external_files = {}
        if args.suite in {"external", "external_ranking", "memory", "routing", "routing_unit", "routing_scale"}:
            self.external_files = {"external_input": Path(args.external_input).resolve(),
                                   "external_client": Path(__file__).with_name("external.py"),
                                   "external_tests": Path(__file__).with_name("test_external.py"),
                                   "external_lock": Path(__file__).with_name("external-requirements.txt")}
            self.external_files["routing_client"] = Path(__file__).with_name("routing.py")
            self.external_files["ranking_client"] = Path(__file__).with_name("ranking.py")
            self.external_files["ranking_tests"] = Path(__file__).with_name("test_ranking.py")
        if args.suite == "memory":
            self.external_files["memory_client"] = Path(__file__).with_name("memory.py")
            self.external_files["memory_tests"] = Path(__file__).with_name("test_memory.py")
            directory = Path(args.external_input).resolve().parent
            for path in sorted(directory.glob("*")):
                if path.is_file():
                    self.external_files["memory_input_" + path.name] = path
        self.external_hashes = {key: hashlib.sha256(path.read_bytes()).hexdigest() for key,path in self.external_files.items()}
        self.native_library = Path(args.native_library).resolve() if getattr(args, "native_library", None) else None
        self.native_sha = hashlib.sha256(self.native_library.read_bytes()).hexdigest() if self.native_library else None

    def dependency_identity(self) -> dict[str, str | None]:
        if self.native_library and hashlib.sha256(self.native_library.read_bytes()).hexdigest() != self.native_sha:
            raise ValueError("native library changed during campaign")
        if {key: hashlib.sha256(path.read_bytes()).hexdigest() for key,path in self.external_files.items()} != self.external_hashes:
            raise ValueError("external input, client or dependency lock changed")
        return {"path": str(self.native_library) if self.native_library else None, "sha256": self.native_sha, **self.external_hashes}

    def cpu_set(self) -> list[int] | None:
        """Validate explicit processor sets while preserving the single-CPU CLI."""
        values = getattr(self.args, "cpus", None)
        if values is None:
            cpu = getattr(self.args, "cpu", None)
            values = None if cpu is None else [cpu]
        if values is None:
            return None
        if (not isinstance(values, list) or not values
                or any(type(cpu) is not int or cpu < 0 for cpu in values)
                or len(set(values)) != len(values)):
            raise ValueError("CPU set must contain unique nonnegative integer IDs")
        if self.args.platform == "arm64":
            raise ValueError("host CPU pinning is not an ARM64-emulation option")
        if self.args.platform == "windows" and max(values) >= 8 * ctypes.sizeof(ctypes.c_void_p):
            raise ValueError("CPU ID is outside the supported Windows processor-group mask")
        return sorted(values)

    @staticmethod
    def verify_thread_affinity(pid: int, cpus: list[int], previous_peak: int) -> int:
        """Check every observed Linux thread whenever its population grows."""
        try:
            tasks = list(Path(f"/proc/{pid}/task").iterdir())
        except FileNotFoundError:
            return previous_peak
        if len(tasks) <= previous_peak:
            return previous_peak
        getter = getattr(os, "sched_getaffinity", None)
        if getter is None:
            raise OSError("native thread affinity cannot be verified")
        checked = 0
        for task in tasks:
            try:
                actual = getter(int(task.name))
            except ProcessLookupError:
                continue
            if actual != set(cpus):
                raise RuntimeError(f"worker affinity escaped configured CPUs: {actual}")
            checked += 1
        return max(previous_peak, checked)

    @staticmethod
    def windows_handle(process: subprocess.Popen[bytes]) -> ctypes.c_void_p:
        """Access the Windows-only Popen handle after checking its availability."""
        handle = getattr(process, "_handle", None)
        if handle is None:
            raise OSError("Windows process handle is unavailable")
        return ctypes.c_void_p(int(handle))

    @staticmethod
    def memory(pid: int, process: subprocess.Popen[bytes]) -> int:
        if os.name == "nt":

            class Counters(ctypes.Structure):
                _fields_ = [("cb", ctypes.c_ulong), ("faults", ctypes.c_ulong)] + [
                    (name, ctypes.c_size_t)
                    for name in (
                        "peak",
                        "working",
                        "paged_peak",
                        "paged",
                        "nonpaged_peak",
                        "nonpaged",
                        "page",
                        "page_peak",
                    )
                ]

            counters = Counters()
            counters.cb = ctypes.sizeof(counters)
            ok = ctypes.windll.psapi.GetProcessMemoryInfo(
                ProcessRunner.windows_handle(process),
                ctypes.byref(counters),
                counters.cb,
            )
            return int(counters.peak) if ok else 0
        try:
            fields = Path(f"/proc/{pid}/status").read_text().splitlines()
            return max(
                (
                    int(x.split()[1]) * 1024
                    for x in fields
                    if x.startswith(("VmHWM:", "VmRSS:"))
                ),
                default=0,
            )
        except (FileNotFoundError, ProcessLookupError):
            return 0

    def invoke(
        self, suite: str, arguments: list[str], environment: dict[str, str], log: Path
    ) -> dict[str, Any]:
        binary = self.binaries[suite]
        self.dependency_identity()
        if self.native_library:
            environment = environment | {"HYMEKO_NATIVE_LIBRARY": str(self.native_library)}
        command = [str(binary), *arguments]
        if suite == "external":
            command = [str(binary), str(self.external_files["memory_client" if self.args.suite == "memory" else "external_client"]), *arguments]
            environment = environment | {"HYMEKO_EXTERNAL_INPUT": str(self.external_files["external_input"]),
                "OPENBLAS_NUM_THREADS": "1", "OMP_NUM_THREADS": "1", "MKL_NUM_THREADS": "1", "NUMEXPR_NUM_THREADS": "1"}
        if self.args.suite == "memory":
            environment = environment | {
                "HYMEKO_MEMORY_INPUT": str(Path(self.args.external_input).resolve().parent),
                "HYMEKO_MEMORY_SCORES": str(log.with_suffix(".scores"))}
        cpus = self.cpu_set()
        if cpus is not None and self.args.platform == "native":
            # Apply affinity before the Rust test harness or worker pool creates
            # threads. The post-spawn set below also pins the short-lived wrapper.
            command = ["taskset", "--cpu-list", ",".join(map(str, cpus)), *command]
        container = None
        if self.args.platform == "arm64":
            container = f"hymeko-graph-cell-{os.getpid()}"
            command = [
                "docker",
                "run",
                "--name",
                container,
                "--network=none",
                "--platform=linux/arm64",
                "--memory=16g",
                "--memory-swap=16g",
                "--cpus=4",
                "--read-only",
                "-v",
                f"{binary}:/bench:ro",
                "-v",
                f"{Path(self.args.qemu).resolve()}:/qemu:ro",
            ]
            for key, value in environment.items():
                command += ["-e", f"{key}={value}"]
            command += ["--entrypoint", "/qemu", self.args.image, "/bench", *arguments]
        env = os.environ | environment
        start = time.monotonic()
        peak = 0
        peak_threads = 0
        verified_affinity = False
        next_update = 10
        with log.open("w", encoding="utf8") as stream:
            process = subprocess.Popen(
                command,
                stdout=stream,
                stderr=subprocess.STDOUT,
                env=env,
                creationflags=0x08000000 if os.name == "nt" else 0,
            )
            pid = process.pid
            try:
                if cpus is not None:
                    if self.args.platform == "windows":
                        if not ctypes.windll.kernel32.SetProcessAffinityMask(
                            self.windows_handle(process), ctypes.c_size_t(sum(1 << cpu for cpu in cpus))
                        ):
                            raise OSError(f"could not pin child to CPUs {cpus}")
                    elif self.args.platform == "native":
                        affinity = getattr(os, "sched_setaffinity", None)
                        if affinity is None:
                            raise OSError("native CPU affinity is unavailable")
                        affinity(pid, set(cpus))
                    else:
                        raise ValueError("host CPU pinning is not an ARM64-emulation option")
                if container:
                    for _ in range(40):
                        probe = subprocess.run(
                            check=False,
                            args=[
                                "docker",
                                "inspect",
                                "--format",
                                "{{.State.Pid}}",
                                container,
                            ],
                            capture_output=True,
                            text=True,
                        )
                        if probe.returncode == 0 and int(probe.stdout.strip()) > 0:
                            pid = int(probe.stdout.strip())
                            break
                        time.sleep(0.1)
                while process.poll() is None:
                    peak = max(peak, self.memory(pid, process))
                    if cpus is not None and self.args.platform == "native":
                        peak_threads = self.verify_thread_affinity(pid, cpus, peak_threads)
                        verified_affinity |= peak_threads > 0
                    elapsed = time.monotonic() - start
                    if peak > 16 * 1024**3 or elapsed > self.args.timeout:
                        raise RuntimeError(
                            f"resource budget exceeded: {log.name}, {peak} B, {elapsed:.1f}s"
                        )
                    if elapsed >= next_update:
                        print(
                            f"  running {log.name}: {elapsed:.1f}s, peak {peak / 1024**2:.1f} MiB",
                            flush=True,
                        )
                        next_update += 10
                    time.sleep(0.05)
                peak = max(peak, self.memory(pid, process))
                if process.returncode:
                    raise RuntimeError(f"child exit {process.returncode}; see {log}")
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                if container:
                    subprocess.run(
                        ["docker", "rm", "-f", container],
                        capture_output=True,
                        check=True,
                    )
        return {
            "wall_seconds": time.monotonic() - start,
            "peak_rss_bytes": peak,
            "memory_scope": "whole process, sampled 50ms VmHWM/PeakWorkingSet",
            "command": command,
            "requested_cpus": cpus,
            "peak_thread_count": peak_threads,
            "thread_affinity_verified": verified_affinity,
            "exit_code": process.returncode,
        }


class Campaign:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        self.runner = ProcessRunner(args)
        self.output = Path(args.output).resolve()
        self.output.mkdir(parents=True, exist_ok=True)
        quiet = getattr(args, "quiet_config", None)
        self.guard_config = json.loads(Path(quiet).read_text(encoding="utf-8")) if quiet else None
        self.guard = QuietHostGuard(self.guard_config) if self.guard_config else None

    def admit(self, result: dict[str, Any]) -> dict[str, Any] | None:
        if self.guard is None:
            return None
        try:
            observation = self.guard.check()
        except HostBusy as error:
            result["blocked"] = {"time": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
                                 "reasons": error.reasons, "observation": error.observation}
            self.save_checkpoint(result)
            print("HOST_BUSY: no new workload started", error, flush=True)
            raise SystemExit(75) from error
        result.pop("blocked", None)
        return observation

    @staticmethod
    def capture_sources(roots: dict[str, Path], *, inventory: str = "git") -> dict[str, Any]:
        """Record source bytes for an explicitly unpublished local revision set.

        Git supplies the inventory, including untracked sources but excluding
        ignored build/report outputs. This records files, not a fabricated commit.
        """
        if inventory not in {"git", "snapshot"}:
            raise ValueError("source inventory must be git or snapshot")
        files = {}
        for label, root in roots.items():
            if inventory == "git":
                paths = subprocess.check_output(
                    ["git", "-C", str(root), "ls-files", "--cached", "--others", "--exclude-standard", "-z"]
                ).decode().split("\0")
            else:
                # Explicit unpacked source bundle: build/dependency/output directories
                # live outside these roots. Added source files must fail verification.
                paths = [p.relative_to(root).as_posix() for p in root.rglob("*") if p.is_file()]
            for relative in sorted(set(paths)):
                path = root / relative
                if path.is_file() and (path.suffix in {".rs", ".py", ".toml", ".zig"} or path.name == "Cargo.lock"):
                    files[f"{label}/{relative}"] = hashlib.sha256(path.read_bytes()).hexdigest()
        result: dict[str, Any] = {"roots": {label: str(root.resolve()) for label, root in roots.items()}, "files": files}
        if inventory != "git":
            result["inventory"] = inventory
        return result

    @staticmethod
    def verify_sources(repo: Path, manifest: Path) -> dict[str, Any]:
        raw = manifest.read_bytes()
        saved = json.loads(raw)
        roots = {label: Path(path) for label, path in saved["roots"].items()}
        if repo.resolve() not in {root.resolve() for root in roots.values()}:
            raise ValueError("source manifest omits the measured repository")
        if not saved["files"] or Campaign.capture_sources(roots, inventory=saved.get("inventory", "git")) != saved:
            raise ValueError("source manifest is stale or incomplete")
        return {"path": str(manifest.resolve()), "sha256": hashlib.sha256(raw).hexdigest(), "files": len(saved["files"]), "content_sha256": hashlib.sha256(json.dumps(saved["files"], sort_keys=True).encode()).hexdigest()}

    @staticmethod
    def resume_checkpoint(current: dict[str, Any], saved: dict[str, Any]) -> dict[str, Any]:
        for key in (
            "source_commit",
            "binaries",
            "suite",
            "platform",
            "runtime",
            "runs",
            "smoke",
            "phases",
            "subjects",
            "workloads",
            "source_manifest",
            "cpu_affinity",
            "native_dependency",
            "quiet_guard",
        ):
            if current.get(key) != saved.get(key):
                raise ValueError(f"checkpoint identity changed: {key}")
        identities = [(c["run"], c["id"]) for c in saved["cells"]]
        if len(identities) != len(set(identities)):
            raise ValueError("duplicate checkpoint cell")
        return saved

    def save_checkpoint(self, result: dict[str, Any]) -> None:
        temporary = self.output / "checkpoint.pending.json"
        temporary.write_text(json.dumps(result, indent=2) + "\n", encoding="utf8")
        temporary.replace(self.output / "checkpoint.json")

    def run(self) -> None:
        target = self.output / "campaign.json"
        if target.exists():
            raise ValueError(f"refusing to overwrite existing evidence: {target}")
        repo = Path(__file__).resolve().parents[2]

        def git(*arguments: str) -> str:
            return subprocess.check_output(
                ["git", "-C", str(repo), *arguments], text=True
            ).strip()

        manifest = getattr(self.args, "source_manifest", None)
        source_ref = self.args.source_ref or ("" if manifest else git("rev-parse", "HEAD"))
        source_identity = self.verify_sources(repo, Path(manifest)) if manifest else None
        if not manifest:
            subprocess.run(
                ["git", "-C", str(repo), "diff", "--exit-code", source_ref, "--",
                 "Cargo.toml", "Cargo.lock", "hymeko_graph/src", "hymeko_graph/tests"],
                check=True, capture_output=True,
            )
        result = {
            "schema_version": 1,
            "suite": self.args.suite,
            "runs": self.args.runs,
            "smoke": self.args.smoke,
            "phases": self.args.phases,
            "subjects": getattr(self.args, "subjects", None),
            "workloads": getattr(self.args, "workloads", None),
            "platform": self.args.platform,
            "created_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            "source_commit": None if manifest else git("rev-parse", source_ref),
            "source_manifest": source_identity,
            "cpu_affinity": getattr(self.args, "cpu", None),
            "driver_commit": None if manifest else git("rev-parse", "HEAD"),
            "driver_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            "dirty_paths": None if manifest else git("status", "--porcelain"),
            "quiet_guard": self.guard_config,
            "host": platform.node(),
            "os": platform.platform(),
            "cpu_count": os.cpu_count(),
            "python": platform.python_version(),
            "metric_scope": ("phase resident RSS and reset VmHWM; single framework; controller verification"
                             if self.args.suite == "memory" else "iteration batch means, not request tails"),
            "runtime": {
                "image": self.args.image if self.args.platform == "arm64" else None,
                "emulated": self.args.platform == "arm64",
            },
            "binaries": {
                k: {
                    "path": str(v),
                    "sha256": hashlib.sha256(v.read_bytes()).hexdigest(),
                }
                for k, v in self.runner.binaries.items()
            },
            "native_dependency": self.runner.dependency_identity(),
            "cells": [],
            "correctness": [],
        }
        checkpoint = self.output / "checkpoint.json"
        if checkpoint.exists():
            if not self.args.resume:
                raise ValueError(
                    "checkpoint exists; use --resume with the same binary and source"
                )
            result = self.resume_checkpoint(
                result, json.loads(checkpoint.read_text(encoding="utf8"))
            )
            for cell in result["cells"]:
                if (
                    Evidence.parse(
                        (self.output / cell["log"]).read_text(encoding="utf8")
                    )
                    != cell["records"]
                ):
                    raise ValueError("checkpoint/raw evidence mismatch")
        self.save_checkpoint(result)
        for suite in self.runner.binaries:
            if any(c["suite"] == suite for c in result["correctness"]):
                continue
            self.admit(result)
            print(f"correctness {suite}", flush=True)
            log = self.output / f"correctness-{suite}.log"
            observation = self.runner.invoke(
                suite, (["native::memory::single_input_roundtrip_rejects_corruption_and_uses_canonical_builder", "--exact", "--test-threads=1"]
                        if self.args.suite == "memory" and suite == "algorithm" else ["--test-threads=1"]),
                {"RAYON_NUM_THREADS": "4"}, log
            )
            result["correctness"].append(
                {"suite": suite, "log": log.name, **observation}
            )
            self.save_checkpoint(result)
        cells = Cell.select(
            self.args.suite, self.args.phases, getattr(self.args, "subjects", None), self.args.smoke,
            getattr(self.args, "workloads", None),
        )
        previous = {
            c["id"]: c["records"]["CELL"][0]["checksum"] for c in result["cells"]
        }
        completed = {(c["run"], c["id"]) for c in result["cells"]}
        for run in range(1, self.args.runs + 1):
            order = cells if run % 2 else list(reversed(cells))
            for ordinal, cell in enumerate(order, 1):
                if (run, cell.key) in completed:
                    continue
                if manifest and self.verify_sources(repo, Path(manifest)) != source_identity:
                    raise ValueError("source identity changed during campaign")
                print(
                    f"{self.args.platform} run {run}/{self.args.runs} {ordinal}/{len(cells)} {cell.key}",
                    flush=True,
                )
                quiet = self.admit(result)
                log = self.output / f"run{run}-{cell.key}.log"
                observation = self.runner.invoke(
                    cell.suite,
                    [cell.test, "--exact", "--ignored", "--nocapture"],
                    cell.environment(run),
                    log,
                )
                records = Evidence.parse(log.read_text(encoding="utf8"))
                observed_peaks = [
                    int(p["self_peak_b"])
                    for p in records.get("PHASE", [])
                    if p.get("self_peak_b", "NA") != "NA"
                ]
                observation["peak_rss_bytes"] = max(
                    [observation["peak_rss_bytes"], *observed_peaks]
                )
                if self.args.suite == "memory":
                    from memory import MemoryEvidence
                    observation["memory_verification"] = MemoryEvidence.verify_scores(
                        Path(self.args.external_input).resolve().parent, cell.workload,
                        log.with_suffix(".scores"), records["MEMORY"][0]["fingerprint"])
                checksum = records["CELL"][0]["checksum"]
                if cell.key in previous and previous[cell.key] != checksum:
                    raise ValueError(f"cross-run checksum drift: {cell.key}")
                previous[cell.key] = checksum
                result["cells"].append(
                    {
                        "id": cell.key,
                        "run": run,
                        "log": log.name,
                        "records": records,
                        "resources": observation,
                        "quiet_preflight": quiet,
                    }
                )
                self.save_checkpoint(result)
                (self.output / "progress.json").write_text(
                    json.dumps(
                        {
                            "completed": len(result["cells"]),
                            "expected": len(cells) * self.args.runs,
                            "last": cell.key,
                        }
                    ),
                    encoding="utf8",
                )
        if manifest:
            self.verify_sources(repo, Path(manifest))
        result["complete"] = True
        result["smoke"] = self.args.smoke
        target.write_text(json.dumps(result, indent=2) + "\n", encoding="utf8")
        print(f"PASS: {len(result['cells'])} fresh processes; {target}", flush=True)


class CampaignSummary:
    """Validate published observations, then emit one cross-platform table."""

    def __init__(self, root: Path):
        self.root = root
        self.rows: list[dict[str, Any]] = []
        self.reference: dict[str, dict[str, Any]] = {}
        self.resources: list[dict[str, Any]] = []
        self.source_commits: set[str] = set()
        self.source_manifests: set[str | None] = set()
        self.selected_cells: set[str] | None = None

    def accept(self, campaign: dict[str, Any]) -> None:
        if not campaign.get("complete") or campaign.get("smoke"):
            raise ValueError("only complete full campaigns may be summarized")
        self.source_commits.add(campaign["source_commit"])
        source_manifest = campaign.get("source_manifest")
        self.source_manifests.add(source_manifest.get("content_sha256", source_manifest["sha256"]) if source_manifest else None)
        if len(self.source_commits) != 1 or len(self.source_manifests) != 1:
            raise ValueError("platforms measured different source revisions")
        expected = {
            cell.key
            for cell in Cell.select(
                campaign.get("suite", "graph"), campaign.get("phases"), campaign.get("subjects"),
                workloads=campaign.get("workloads"),
            )
        }
        if self.selected_cells is not None and self.selected_cells != expected:
            raise ValueError("platforms measured different selected matrices")
        self.selected_cells = expected
        runs = int(campaign.get("runs", 3))
        if runs < 1 or len(campaign["cells"]) != runs * len(expected):
            raise ValueError("campaign process count does not match declared matrix")
        groups: dict[str, list[dict[str, Any]]] = {}
        results: dict[tuple[int, str, str], list[dict[str, str]]] = {}
        for observation in campaign["cells"]:
            key = observation["id"]
            records = observation["records"]
            cell = records["CELL"][0]
            identity = {
                k: cell[k]
                for k in ("status", "nodes", "arcs", "incidences", "checksum")
            }
            for kind in ("RESULT", "WORK", "SOURCE", "INPUT", "SHAPE", "RANK", "REACHABLE", "NATIVE", "QUERY", "LAYOUT", "CONSTRUCTION", "ALGEBRA"):
                identity[kind] = [
                    {k: v for k, v in r.items() if k != "run"}
                    for r in records.get(kind, [])
                ]
            identity["EXTERNAL"] = [
                {k: v for k, v in r.items() if k not in {"construction_us", "first_query_us"}}
                for r in records.get("EXTERNAL", [])
            ]
            if key in self.reference and identity != self.reference[key]:
                raise ValueError(
                    f"semantic/work-counter drift: {campaign['platform']} {key}"
                )
            self.reference[key] = identity
            groups.setdefault(key, []).append(observation)
            if records.get("RESULT"):
                results.setdefault(
                    (observation["run"], cell["section"], cell["workload"]), []
                ).append(records["RESULT"][0])
        if set(groups) != expected:
            raise ValueError("missing/unknown matrix cells")
        for (_, section, workload), paired in results.items():
            if (
                section == "informed_search"
                and len({r["cost_micros"] for r in paired}) != 1
            ):
                raise ValueError(f"A*/Dijkstra optimal-cost mismatch: {workload}")
            if section == "berge_shortest":
                if (
                    len(paired) != 2
                    or len({(r["reachable"], r["hops"]) for r in paired}) != 1
                ):
                    raise ValueError(
                        f"BFS/Dijkstra reachability/hop mismatch: {workload}"
                    )
                weighted = next(r for r in paired if r["subject"] == "dijkstra_unit")
                if int(weighted["cost_micros"]) != int(weighted["hops"]) * 1_000_000:
                    raise ValueError(f"unit-cost/hop mismatch: {workload}")
        for key, observations in groups.items():
            if sorted(o["run"] for o in observations) != list(range(1, runs + 1)):
                raise ValueError(f"missing/duplicate independent process run: {key}")
            cell = observations[0]["records"]["CELL"][0]
            if cell["status"] != "OK":
                continue
            samples = sorted(
                float(s["wall_us"])
                for o in observations
                for s in o["records"]["SAMPLE"]
            )
            if len(samples) != 9 * runs or not all(
                math.isfinite(s) and s > 0 for s in samples
            ):
                raise ValueError(f"invalid pooled samples: {key}")
            peaks = [o["resources"]["peak_rss_bytes"] for o in observations]
            if min(peaks) <= 0 or any(
                o["resources"]["exit_code"] != 0 for o in observations
            ):
                raise ValueError(
                    f"missing process memory or unsuccessful process: {key}"
                )
            process_medians = sorted(float(o["records"]["CELL"][0]["wall_median_us"]) for o in observations)
            rank = observations[0]["records"].get("RANK", [{}])[0]
            native = observations[0]["records"].get("NATIVE", [{}])[0]
            if native.get("operation") == "rank":
                backend = "hypergraphz" if cell["subject"].endswith("_hypergraphz") else "hymeko"
                rank = {"iterations": native[f"{backend}_iterations"],
                        "error_bound_l1": native[f"{backend}_bound"]}
            ranking = cell.get("section") == "pagerank"
            shape = observations[0]["records"].get("SHAPE", [{}])[0]
            query = observations[0]["records"].get("QUERY", [{}])[0]
            self.rows.append(
                {
                    "platform": campaign["platform"],
                    "benchmark": key,
                    "workload": cell["workload"],
                    "algorithm": cell["subject"],
                    **{key: observations[0]["records"].get("EXTERNAL", [{}])[0].get(key, "")
                       for key in ("backend", "version", "mode", "operation", "query_strategy", "interface", "completed_isolates")},
                    "phase": next(
                        (p[1] for p in PHASES if p[0] == cell["subject"]),
                        "preparation"
                        if cell.get("section") == "heuristic_preparation" or (ranking and cell["subject"] == "prepare")
                        else "execution",
                    ),
                    "measurement_scope": observations[0]["records"].get("INPUT", [{}])[0].get("metric_scope") or next(
                        (p[2] for p in PHASES if p[0] == cell["subject"]),
                        ("Prepared incidence walk; IR construction excluded" if cell["subject"] == "prepare"
                         else "Warm operation including output/workspace reset; graph construction and preparation excluded")
                        if ranking else observations[0]["records"].get("INPUT", [{}])[0].get("metric_scope", "See original graph benchmark contract"),
                    ),
                    "threads": cell.get("threads", "1"),
                    "samples": len(samples),
                    "processes": runs,
                    "process_median_ns": process_medians[runs // 2] * 1000,
                    "process_q1_ns": process_medians[runs // 4] * 1000,
                    "process_q3_ns": process_medians[3 * runs // 4] * 1000,
                    "median_ns": samples[len(samples) // 2] * 1000,
                    "q1_ns": samples[len(samples) // 4] * 1000,
                    "q3_ns": samples[3 * len(samples) // 4] * 1000,
                    "nodes": cell["nodes"], "arcs": cell["arcs"], "incidences": cell["incidences"],
                    **{key: shape.get(key, "") for key in ("arity_min", "arity_median", "arity_max", "degree_min", "degree_median", "degree_max", "isolated_nodes")},
                    "components": native.get("components", ""),
                    "algebra_value": observations[0]["records"].get("ALGEBRA", [{}])[0].get("value", ""),
                    "overlap_pair_work": observations[0]["records"].get("ALGEBRA", [{}])[0].get("pair_work", ""),
                    "query_kind": query.get("kind", ""),
                    "query_visited": query.get("visited", ""),
                    "query_hops": query.get("hops", ""),
                    "query_cost": query.get("cost", ""),
                    "query_reachable": query.get("reachable", ""),
                    "hymeko_prepared_bytes": observations[0]["records"].get("LAYOUT", [{}])[0].get("hymeko_prepared_bytes", ""),
                    "iterations": rank.get("iterations", ""),
                    "error_bound_l1": rank.get("error_bound_l1", ""),
                    "rank_mass": rank.get("mass", ""),
                    "visited_nodes": observations[0]["records"].get("REACHABLE", [{}])[0].get("visited", ""),
                    "worst_batch_mean_ns": samples[-1] * 1000,
                    "checksum": cell["checksum"],
                    "peak_rss_bytes": max(peaks),
                    "source_bytes": observations[0]["records"]
                    .get("SOURCE", [{}])[0]
                    .get("bytes", ""),
                }
            )
        self.resources.append(
            {
                "platform": campaign["platform"],
                "processes": len(campaign["cells"]),
                "successful_cells": sum(
                    v[0]["records"]["CELL"][0]["status"] == "OK"
                    for v in groups.values()
                ),
                "peak_rss_bytes": max(
                    o["resources"]["peak_rss_bytes"] for o in campaign["cells"]
                ),
                "process_wall_seconds": sum(
                    o["resources"]["wall_seconds"] for o in campaign["cells"]
                ),
            }
        )

    def write(
        self, platforms: tuple[str, ...] = ("windows", "native", "arm64")
    ) -> None:
        campaigns = []
        for platform_name in platforms:
            campaign = json.loads(
                (self.root / platform_name / "campaign.json").read_text(encoding="utf8")
            )
            if campaign["platform"] != platform_name:
                raise ValueError("campaign platform/path mismatch")
            campaigns.append(campaign)
            self.accept(campaign)
        for filename, rows in (
            ("platform-summary.csv", self.rows),
            ("process-resources.csv", self.resources),
        ):
            with (self.root / filename).open(
                "w", encoding="utf8", newline=""
            ) as stream:
                writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
                writer.writeheader()
                writer.writerows(rows)
        receipt = {
            "source_commit": next(iter(self.source_commits)),
            "platforms": len(self.resources),
            "processes": sum(r["processes"] for r in self.resources),
            "metric_rows": len(self.rows),
            "raw_samples": sum(r["samples"] for r in self.rows),
            "semantic_and_work_counter_parity": True,
            "bfs_dijkstra_and_astar_optimal_cost_checks": all(
                c.get("suite", "graph") == "graph" for c in campaigns
            ),
            "parsed_fixture_equivalence_checks": all(
                c.get("suite") == "parsing" for c in campaigns
            ),
            "quantiles": "sorted pooled batch means, indexes floor(n/4), floor(n/2), floor(3n/4)",
        }
        (self.root / "verification.json").write_text(
            json.dumps(receipt, indent=2) + "\n", encoding="utf8"
        )
        sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
        from svg_plot import bar_svg

        selections = [
            ("grid d80", "search_cell-grid200_d80-astar_l1-t1"),
            ("map128", "search_cell-map128-astar_combined-t1"),
            ("transport", "search_cell-transport128-astar_discrete-t1"),
        ]
        if all(c.get("suite") == "ranking" for c in campaigns):
            selections = [(w, f"ranking_ranking_cell-{w}-solve-t1")
                          for w in ("rank2_1m", "rank32_1m", "rank1000_1m", "disjoint_1m")]
        elif all(c.get("suite") == "comparison" for c in campaigns):
            selections = [(f"{w} {s}", f"native_native_cell-{w}-{s}-t1")
                          for w in (campaigns[0].get("workloads") or COMPARISON_WORKLOADS[:3])
                          for s in ("rank_hymeko", "rank_hypergraphz")]
        elif self.rows[0]["source_bytes"]:
            common = set.intersection(
                *(
                    {r["algorithm"] for r in self.rows if r["platform"] == p}
                    for p in platforms
                )
            )
            subject = next(
                (s for s in ("parse_ast", "file_to_dijkstra") if s in common),
                min(common),
            )
            selections = [
                (w, f"parsing_parsing_cell-{w}-{subject}-t1")
                for w in ("map32", "map128", "map512")
            ]
        indexed = {(row["platform"], row["benchmark"]): row for row in self.rows}
        selections = [(label, key) for label, key in selections
                      if all((platform_name, key) in indexed for platform_name in platforms)]
        if not selections:
            selections = [(row["benchmark"], row["benchmark"]) for row in self.rows
                          if row["platform"] == platforms[0]
                          and all((p, row["benchmark"]) in indexed for p in platforms)]
        plotted = [
            (label, [indexed[p, key]["median_ns"] / 1000 for p in platforms])
            for label, key in selections
        ]
        bar_svg(
            "PageRank to convergence: one million incidences"
            if campaigns[0].get("suite") == "ranking" else (
                "Selected pipeline medians (ARM64, where present, is QEMU)"
                if self.rows[0]["source_bytes"] else "Matched native PageRank API medians"
                if campaigns[0].get("suite") == "comparison" else "Selected graph search medians (ARM64 is QEMU)"),
            plotted,
            [],
            platforms,
            ["#2563eb", "#0d9488", "#d97706"],
            "microseconds / operation",
            self.root / "search-platforms.svg",
        )
        provenance = {
            "source_commit": next(iter(self.source_commits)),
            "measurement_contract": (
                "Matched native API calls include normalized output copying and temporary-result destruction. Rank preparation/workspace allocation is included except in the separately labeled rank_prepared cell. Component reverse indexes and graph construction are excluded. Whole-process RSS includes both resident graphs and parity verification."
                if campaigns[0].get("suite") == "comparison" else
                "Separate measured phases; totals measured directly, never summed. Generation constructs fixture IR. File phases use warm OS cache. Final returned-object destruction excluded. RSS includes fixture construction and validation."
            ),
            "external_map_import": "not measured; current inputs are deterministic graph fixtures",
            "phases": [{"algorithm": a, "phase": p, "scope": s} for a, p, s in PHASES],
            "platforms": [
                {k: v for k, v in c.items() if k not in {"cells", "correctness"}}
                for c in campaigns
            ],
        }
        (self.root / "source-provenance.json").write_text(
            json.dumps(provenance, indent=2) + "\n", encoding="utf8"
        )
        # Each nested campaign owns its own manifest and measured revision.
        files = list(self.root.glob("*"))
        for platform_name in platforms:
            files.extend((self.root / platform_name).rglob("*"))
        manifest = {
            str(p.relative_to(self.root)).replace("\\", "/"): hashlib.sha256(
                p.read_bytes()
            ).hexdigest()
            for p in sorted(files)
            if p.is_file() and p.name != "artifacts-manifest.json"
        }
        (self.root / "artifacts-manifest.json").write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf8"
        )
        print(json.dumps(receipt), flush=True)


if __name__ == "__main__":
    if len(sys.argv) >= 3 and sys.argv[1] == "--summarize":
        summary_parser = argparse.ArgumentParser()
        summary_parser.add_argument("--summarize", required=True)
        summary_parser.add_argument(
            "--platforms",
            nargs="+",
            choices=("windows", "native", "arm64"),
            default=["windows", "native", "arm64"],
        )
        summary_args = summary_parser.parse_args()
        if len(set(summary_args.platforms)) != len(summary_args.platforms):
            summary_parser.error("duplicate platforms")
        CampaignSummary(Path(summary_args.summarize).resolve()).write(
            tuple(summary_args.platforms)
        )
        raise SystemExit(0)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--platform", choices=("windows", "native", "arm64"), required=True
    )
    parser.add_argument("--informed")
    parser.add_argument("--algorithm")
    parser.add_argument("--suite", choices=("graph", "parsing", "ranking", "comparison", "traversal", "external", "external_ranking", "memory", "assisted", "routing", "routing_unit", "routing_scale"), default="graph")
    parser.add_argument("--external-python", help="isolated Python interpreter for external framework cells")
    parser.add_argument("--external-input", help="immutable canonical-IR fixture export")
    parser.add_argument("--subjects", nargs="+", help="explicit operations from the selected suite's existing matrix")
    parser.add_argument("--workloads", nargs="+", choices=COMPARISON_WORKLOADS + GENERATED_WORKLOADS + ROUTING_WORKLOADS + SCALING_WORKLOADS + RANKING_WORKLOADS,
                        help="explicit comparison/traversal fixtures; omitted keeps each suite's default matrix")
    parser.add_argument("--native-library", help="explicit pinned Windows native comparison adapter")
    parser.add_argument("--source-manifest", help="explicit source-byte snapshot for unpublished local source and dependency roots")
    parser.add_argument(
        "--phases",
        nargs="+",
        choices=tuple(p[0] for p in PHASES),
    )
    parser.add_argument(
        "--source-ref",
        help="timed Rust revision; accepted only when its source and manifests are unchanged",
    )
    parser.add_argument("--output", required=True)
    parser.add_argument("--qemu")
    parser.add_argument(
        "--image",
        default="sha256:32d322b19846336d25f755f73618a448e3621982d52c48064e95af8b3dcbc2d9",
    )
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--cpu", type=int, help="pin each measurement process to this logical CPU")
    parser.add_argument("--quiet-config", help="explicit Linux quiet-host thresholds; blocks before each workload")
    parser.add_argument("--smoke", action="store_true")
    parser.add_argument("--resume", action="store_true")
    arguments = parser.parse_args()
    if arguments.phases and (
        arguments.suite != "parsing"
        or len(set(arguments.phases)) != len(arguments.phases)
    ):
        parser.error("unique --phases are supported only for the parsing suite")
    if arguments.suite in {"graph", "ranking", "comparison", "traversal"} and not arguments.algorithm:
        parser.error("--algorithm is required for graph/ranking/comparison/traversal suites")
    if arguments.suite == "comparison" and (arguments.platform not in {"windows", "native"} or not arguments.native_library):
        parser.error("comparison requires native Windows/Linux and an explicit --native-library")
    if arguments.suite in {"external", "external_ranking", "memory", "routing", "routing_unit", "routing_scale"} and (arguments.platform not in {"windows", "native"} or not arguments.external_python or not arguments.external_input):
        parser.error("external requires Windows/Linux, --external-python and --external-input")
    if arguments.suite not in {"ranking", "comparison", "traversal", "external", "external_ranking", "memory", "routing_unit"} and not arguments.informed:
        parser.error("--informed is required for graph/parsing suites")
    if arguments.suite == "memory" and arguments.platform != "native":
        parser.error("isolated phase memory currently requires native Linux")
    if arguments.source_manifest and arguments.source_ref:
        parser.error("choose an exact source ref or an unpublished source manifest")
    if arguments.cpu is not None and not 0 <= arguments.cpu < (os.cpu_count() or 1):
        parser.error("--cpu is outside this host's logical CPU range")
    if arguments.runs < 1 or (arguments.platform == "arm64" and not arguments.qemu):
        parser.error("positive --runs and an ARM64 --qemu path are required")
    try:
        selected = Cell.select(arguments.suite, arguments.phases, arguments.subjects, arguments.smoke, arguments.workloads)
        if arguments.suite in {"external", "external_ranking", "memory"}:
            if any(c.suite == "algorithm" for c in selected) and not arguments.algorithm:
                parser.error("selected native baselines require --algorithm")
            if any(c.test in {"native::native_cell", "native::memory::memory_cell"} for c in selected) and not arguments.native_library:
                parser.error("selected Hypergraphz/native parity cells require --native-library")
    except ValueError as error:
        parser.error(str(error))
    Campaign(arguments).run()
