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
        return result

    @staticmethod
    def matrix(suite: str = "graph") -> list[Cell]:
        if suite == "parsing":
            workloads = (
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
            phases = (
                "parse_ast",
                "ast_to_ir",
                "compile",
                "incidence_index",
                "source_to_dijkstra",
            )
            return [
                Cell("informed", "parsing::parsing_cell", w, p)
                for w in workloads
                for p in phases
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
        if cell.get("section") == "parsing":
            sources = records.get("SOURCE", [])
            if len(sources) != 1 or int(sources[0]["bytes"]) <= 0:
                raise ValueError("missing parsing input provenance")
            if (
                sources[0]["fnv1a64"] != cell["checksum"]
                or sources[0]["projection"] != "graph_fixture_v1"
            ):
                raise ValueError("parsing input identity mismatch")
        return records


class ProcessRunner:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        self.binaries = {
            "informed": Path(args.informed).resolve(),
        }
        if args.suite == "graph":
            self.binaries["algorithm"] = Path(args.algorithm).resolve()

    @staticmethod
    def memory(pid: int, process: subprocess.Popen) -> int:
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
                ctypes.c_void_p(int(process._handle)),
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
    ) -> dict:
        binary = self.binaries[suite]
        command = [str(binary), *arguments]
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
            "exit_code": process.returncode,
        }


class Campaign:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        self.runner = ProcessRunner(args)
        self.output = Path(args.output).resolve()
        self.output.mkdir(parents=True, exist_ok=True)

    @staticmethod
    def resume_checkpoint(current: dict, saved: dict) -> dict:
        for key in (
            "source_commit",
            "binaries",
            "suite",
            "platform",
            "runtime",
            "runs",
            "smoke",
        ):
            if current[key] != saved[key]:
                raise ValueError(f"checkpoint identity changed: {key}")
        identities = [(c["run"], c["id"]) for c in saved["cells"]]
        if len(identities) != len(set(identities)):
            raise ValueError("duplicate checkpoint cell")
        return saved

    def save_checkpoint(self, result: dict) -> None:
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

        result = {
            "schema_version": 1,
            "suite": self.args.suite,
            "runs": self.args.runs,
            "smoke": self.args.smoke,
            "platform": self.args.platform,
            "created_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            "source_commit": git("rev-parse", "HEAD"),
            "dirty_paths": git("status", "--porcelain"),
            "host": platform.node(),
            "os": platform.platform(),
            "cpu_count": os.cpu_count(),
            "python": platform.python_version(),
            "metric_scope": "iteration batch means, not request tails",
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
            print(f"correctness {suite}", flush=True)
            log = self.output / f"correctness-{suite}.log"
            observation = self.runner.invoke(
                suite, ["--test-threads=1"], {"RAYON_NUM_THREADS": "4"}, log
            )
            result["correctness"].append(
                {"suite": suite, "log": log.name, **observation}
            )
            self.save_checkpoint(result)
        cells = Cell.matrix(self.args.suite)
        if self.args.smoke:
            cells = (
                [
                    c
                    for c in cells
                    if (c.workload, c.subject)
                    in {
                        ("map512", "compile"),
                        ("random1e6", "compile"),
                        ("grid5", "parse_ast"),
                    }
                ]
                if self.args.suite == "parsing"
                else [cells[0], cells[-1]]
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
                print(
                    f"{self.args.platform} run {run}/{self.args.runs} {ordinal}/{len(cells)} {cell.key}",
                    flush=True,
                )
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
        result["complete"] = True
        result["smoke"] = self.args.smoke
        target.write_text(json.dumps(result, indent=2) + "\n", encoding="utf8")
        print(f"PASS: {len(result['cells'])} fresh processes; {target}", flush=True)


class CampaignSummary:
    """Validate published observations, then emit one cross-platform table."""

    def __init__(self, root: Path):
        self.root = root
        self.rows = []
        self.reference = {}
        self.resources = []
        self.source_commits = set()

    def accept(self, campaign: dict) -> None:
        if not campaign.get("complete") or campaign.get("smoke"):
            raise ValueError("only complete full campaigns may be summarized")
        self.source_commits.add(campaign["source_commit"])
        if len(self.source_commits) != 1:
            raise ValueError("platforms measured different source revisions")
        expected = {cell.key for cell in Cell.matrix(campaign.get("suite", "graph"))}
        if len(campaign["cells"]) != 3 * len(expected):
            raise ValueError("campaign process count does not match declared matrix")
        groups = {}
        results = {}
        for observation in campaign["cells"]:
            key = observation["id"]
            records = observation["records"]
            cell = records["CELL"][0]
            identity = {
                k: cell[k]
                for k in ("status", "nodes", "arcs", "incidences", "checksum")
            }
            for kind in ("RESULT", "WORK", "SOURCE"):
                identity[kind] = [
                    {k: v for k, v in r.items() if k != "run"}
                    for r in records.get(kind, [])
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
            if sorted(o["run"] for o in observations) != [1, 2, 3]:
                raise ValueError(f"missing/duplicate independent process run: {key}")
            cell = observations[0]["records"]["CELL"][0]
            if cell["status"] != "OK":
                continue
            samples = sorted(
                float(s["wall_us"])
                for o in observations
                for s in o["records"]["SAMPLE"]
            )
            if len(samples) != 27 or not all(
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
            self.rows.append(
                {
                    "platform": campaign["platform"],
                    "benchmark": key,
                    "workload": cell["workload"],
                    "algorithm": cell["subject"],
                    "threads": cell.get("threads", "1"),
                    "samples": 27,
                    "median_ns": samples[13] * 1000,
                    "q1_ns": samples[6] * 1000,
                    "q3_ns": samples[20] * 1000,
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

    def write(self) -> None:
        for platform_name in ("windows", "native", "arm64"):
            self.accept(
                json.loads(
                    (self.root / platform_name / "campaign.json").read_text(
                        encoding="utf8"
                    )
                )
            )
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
            "bfs_dijkstra_and_astar_optimal_cost_checks": True,
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
        if self.rows[0]["source_bytes"]:
            selections = [
                (w, f"parsing_parsing_cell-{w}-parse_ast-t1")
                for w in ("map32", "map128", "map512")
            ]
        platforms = ["windows", "native", "arm64"]
        indexed = {(row["platform"], row["benchmark"]): row for row in self.rows}
        plotted = [
            (label, [indexed[p, key]["median_ns"] / 1000 for p in platforms])
            for label, key in selections
        ]
        bar_svg(
            "Selected parsing medians (ARM64 is QEMU)"
            if self.rows[0]["source_bytes"]
            else "Selected graph search medians (ARM64 is QEMU)",
            plotted,
            [],
            platforms,
            ["#2563eb", "#0d9488", "#d97706"],
            "microseconds / operation",
            self.root / "search-platforms.svg",
        )
        print(json.dumps(receipt), flush=True)


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--summarize":
        CampaignSummary(Path(sys.argv[2]).resolve()).write()
        raise SystemExit(0)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--platform", choices=("windows", "native", "arm64"), required=True
    )
    parser.add_argument("--informed", required=True)
    parser.add_argument("--algorithm")
    parser.add_argument("--suite", choices=("graph", "parsing"), default="graph")
    parser.add_argument("--output", required=True)
    parser.add_argument("--qemu")
    parser.add_argument(
        "--image",
        default="sha256:32d322b19846336d25f755f73618a448e3621982d52c48064e95af8b3dcbc2d9",
    )
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--smoke", action="store_true")
    parser.add_argument("--resume", action="store_true")
    arguments = parser.parse_args()
    if arguments.suite == "graph" and not arguments.algorithm:
        parser.error("--algorithm is required for the graph suite")
    if arguments.runs < 1 or (arguments.platform == "arm64" and not arguments.qemu):
        parser.error("positive --runs and an ARM64 --qemu path are required")
    Campaign(arguments).run()
