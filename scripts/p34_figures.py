#!/usr/bin/env python3
"""Generate the P3.4 SVG figures from committed raw TSVs (no external dependency). Reuses the shared
`svg_plot` emitter (§6.1). Usage: python scripts/p34_figures.py"""
import csv, pathlib, sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from svg_plot import bar_svg, line_svg

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA = ROOT / "reports" / "data"
OUT = ROOT / "reports" / "figures"
OUT.mkdir(parents=True, exist_ok=True)


def read_tsv(name):
    # utf-8-sig strips the BOM PowerShell's Out-File writes.
    with open(DATA / name, encoding="utf-8-sig") as f:
        return list(csv.DictReader(f, delimiter="\t"))


# Figure 1: CPU scaling — speedup vs threads for the parallel batch op, per workload.
cpu = [r for r in read_tsv("2026-09-03-p34-cpu-scaling.tsv") if r["op"] == "batch"]
COLORS = {"sts1023": "#3b6", "random1e6": "#b36", "map512": "#36b"}
series = []
for w, color in COLORS.items():
    pts = sorted((int(r["threads"]), float(r["speedup"])) for r in cpu if r["workload"] == w)
    if pts:
        series.append((w, pts, color))
# ideal linear reference
maxt = max(x for _, pts, _ in series for x, _ in pts)
series.append(("ideal linear", [(1, 1), (maxt, maxt)], "#999"))
line_svg("P3.4 CPU scaling: batch-reachability speedup vs Rayon threads",
         series, "threads (log)", "speedup vs 1 thread (log)", OUT / "p34-cpu-scaling.svg")

# Figure 2: memory — IncidenceIndex vs whole-workload bytes, per incidence (index is NOT total).
mem = read_tsv("2026-09-03-p34-memory.tsv")
rows2 = []
for r in mem:
    inc = float(r["incidences"])
    idx_bpi = float(r["index_retained_b"]) / inc
    tot_bpi = float(r["total_bytes"]) / inc
    rows2.append((r["workload"], [idx_bpi, tot_bpi]))
bar_svg("P3.4 memory: IncidenceIndex is a fraction of total (bytes per incidence)",
        rows2, ["idx", "total"], ["IncidenceIndex B/inc", "whole-workload B/inc"],
        ["#3b6", "#b63"], "bytes / incidence", OUT / "p34-memory.svg")

# Figure 3: representation break-even Q vs direct, per representation (sts1023).
rep = [r for r in read_tsv("2026-09-03-p34-representation.tsv") if r["workload"] == "sts1023"]
def be_q(s):
    if s.startswith("Q>="):
        return float(s[3:])
    return 0.0  # tie/NEVER -> 0 (baseline)
rows3 = [(r["representation"].replace("_", " "), [be_q(r["breakeven_vs_direct"])]) for r in rep]
bar_svg("P3.4 connectivity break-even vs direct (STS(1023)): queries to amortize prep",
        rows3, ["Q*"], ["break-even Q (0 = baseline/NEVER)"], ["#63b"], "queries Q*",
        OUT / "p34-breakeven.svg")

# Figure 4: Levi rank / basis-incidence ratio + ns-per-incidence per workload.
lp = read_tsv("2026-09-03-p34-levi-perf.tsv")
rows4 = [(r["workload"], [float(r["basis_ratio"]), float(r["ns_per_inc"])]) for r in lp if int(r["incidences"]) > 0]
bar_svg("P3.4 Levi basis: rank/incidence ratio and ns per incidence",
        rows4, ["ratio", "ns/inc"], ["basis/incidence ratio", "ns per incidence"],
        ["#3b6", "#b36"], "value", OUT / "p34-rank-basis-ratio.svg")
