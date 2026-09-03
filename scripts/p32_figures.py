#!/usr/bin/env python3
"""Generate P3.2 SVG figures from committed raw TSV (no external dependency; emits SVG directly).
Usage: python scripts/p32_figures.py"""
import csv, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA = ROOT / "reports" / "data"
OUT = ROOT / "reports" / "figures"
OUT.mkdir(parents=True, exist_ok=True)


def bar_svg(title, rows, labels, series, colors, ylabel, path):
    # rows: list of (label, [values per series]); series: list of series names
    w, h, pad = 720, 380, 60
    n = len(rows)
    maxv = max((max(v) for _, v in rows), default=1) or 1
    bw = (w - 2 * pad) / (n * (len(series) + 1))
    svg = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" font-family="sans-serif">']
    svg.append(f'<text x="{w/2}" y="24" text-anchor="middle" font-size="16" font-weight="bold">{title}</text>')
    svg.append(f'<line x1="{pad}" y1="{h-pad}" x2="{w-pad}" y2="{h-pad}" stroke="#333"/>')
    svg.append(f'<line x1="{pad}" y1="{pad}" x2="{pad}" y2="{h-pad}" stroke="#333"/>')
    svg.append(f'<text x="16" y="{h/2}" text-anchor="middle" font-size="11" transform="rotate(-90 16 {h/2})">{ylabel}</text>')
    for i, (lab, vals) in enumerate(rows):
        x0 = pad + i * (len(series) + 1) * bw + bw / 2
        for s, v in enumerate(vals):
            bh = (h - 2 * pad) * v / maxv
            x = x0 + s * bw
            svg.append(f'<rect x="{x:.1f}" y="{h-pad-bh:.1f}" width="{bw*0.9:.1f}" height="{bh:.1f}" fill="{colors[s]}"/>')
        svg.append(f'<text x="{x0 + bw*len(series)/2:.1f}" y="{h-pad+14}" text-anchor="middle" font-size="10">{lab}</text>')
    for s, name in enumerate(series):
        lx = w - pad - 150
        svg.append(f'<rect x="{lx}" y="{pad + s*18}" width="12" height="12" fill="{colors[s]}"/>')
        svg.append(f'<text x="{lx+16}" y="{pad + s*18 + 11}" font-size="11">{name}</text>')
    svg.append('</svg>')
    path.write_text("\n".join(svg), encoding="utf-8")
    print("wrote", path.name)


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
