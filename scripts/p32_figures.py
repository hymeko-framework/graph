#!/usr/bin/env python3
"""Generate P3.2 SVG figures from committed raw TSV (no external dependency; emits SVG directly).
Usage: python scripts/p32_figures.py"""
import csv, pathlib, sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from svg_plot import bar_svg  # shared emitter (§6.1)

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA = ROOT / "reports" / "data"
OUT = ROOT / "reports" / "figures"
OUT.mkdir(parents=True, exist_ok=True)


# Figure 1: work counters (expanded nodes vs pushes) per algo, grid128 — shows the "no pruning" finding.
rows = []
with open(DATA / "2026-09-03-p32-work-counters.tsv") as f:
    for r in csv.DictReader(f, delimiter="\t"):
        if r["workload"] == "grid128":
            rows.append((r["algo"], [int(r["expanded_nodes"]), int(r["pushes"]), int(r["stale_pops"])]))
bar_svg("P3.2 grid128: A* expands the same states as Dijkstra (weak heuristic)",
        rows, ["expanded_nodes", "pushes", "stale_pops"],
        ["expanded", "pushes", "stale"], ["#3b6", "#36b", "#b63"],
        "count", OUT / "p32-work-counters.svg")

# Figure 2: workspace reuse allocations/query.
rows2 = []
with open(DATA / "2026-09-03-p32-workspace-batch-alloc.tsv") as f:
    for r in csv.DictReader(f, delimiter="\t"):
        rows2.append((r["mode"].replace("workspace-", "").replace("reused-", "re:"), [float(r["allocs_per_query"])]))
bar_svg("P3.2 allocations per query (reuse cuts ~4x, not to zero)",
        rows2, ["allocs/query"], ["allocs/query"], ["#b36"], "allocs", OUT / "p32-allocations.svg")
