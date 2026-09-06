#!/usr/bin/env python3
"""Generate the P3.3 rank-2 matroid SVG figures from the committed latency TSV (no external dependency).
Reuses the shared `svg_plot` emitter (§6.1). Usage: python scripts/p33_matroid_figures.py"""
import csv, pathlib, sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from svg_plot import bar_svg, line_svg

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA = ROOT / "reports" / "data"
OUT = ROOT / "reports" / "figures"
OUT.mkdir(parents=True, exist_ok=True)

rows = list(csv.DictReader(open(DATA / "2026-09-03-p33-matroid-latency.tsv"), delimiter="\t"))

# Figure 1: latency vs arcs (log-log): spanning forest, weighted basis, incremental stream.
def pts(col):
    return [(int(r["arcs"]), float(r[col])) for r in rows]

line_svg(
    "P3.3 rank-2 matroid: computation latency vs |ground set| (arcs)",
    [
        ("spanning forest (DSU)", pts("forest_us"), "#3b6"),
        ("weighted basis (Kruskal)", pts("weighted_us"), "#b36"),
        ("incremental stream", pts("incr_us"), "#36b"),
        ("index build (context)", pts("idx_build_us"), "#999"),
    ],
    "arcs (log)", "median us (log)", OUT / "p33-matroid-latency.svg",
)

# Figure 2: basis size (rank) and DSU workspace bytes per family (the memory picture).
brows = [(r["family"], [int(r["rank"]), int(r["dsu_bytes"]) / 1024.0]) for r in rows]
bar_svg(
    "P3.3 rank-2 matroid: basis size (rank) and DSU workspace memory",
    brows, ["rank", "dsu_KiB"], ["rank (edges)", "DSU KiB (high-water)"],
    ["#3b6", "#b63"], "value", OUT / "p33-matroid-basis-memory.svg",
)

# Figure 3: incremental stream vs batch spanning-forest (per family) — the update-cost comparison.
irows = [(r["family"], [float(r["forest_us"]), float(r["incr_us"])]) for r in rows]
bar_svg(
    "P3.3 rank-2 matroid: incremental stream vs batch spanning-forest",
    irows, ["forest_us", "incr_us"], ["batch forest", "incremental"],
    ["#3b6", "#36b"], "median us", OUT / "p33-matroid-incremental.svg",
)
