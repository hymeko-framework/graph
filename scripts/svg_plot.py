#!/usr/bin/env python3
"""Shared dependency-free SVG plot primitives for the graph reports (no matplotlib). `bar_svg` was
lifted verbatim from the P3.2 figures script so both phases share one emitter (§6.1); `line_svg` adds a
log-log line plot for latency-vs-size."""
import math


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


def line_svg(title, series, xlabel, ylabel, path):
    # series: list of (name, [(x, y), ...], color). Log-log axes.
    w, h, pad = 720, 420, 66
    pts = [(x, y) for _, data, _ in series for (x, y) in data if x > 0 and y > 0]
    xs = [x for x, _ in pts]
    ys = [y for _, y in pts]
    lx0, lx1 = math.log10(min(xs)), math.log10(max(xs))
    ly0, ly1 = math.log10(min(ys)), math.log10(max(ys))
    if lx1 == lx0:
        lx1 += 1
    if ly1 == ly0:
        ly1 += 1

    def px(x):
        return pad + (math.log10(x) - lx0) / (lx1 - lx0) * (w - 2 * pad)

    def py(y):
        return (h - pad) - (math.log10(y) - ly0) / (ly1 - ly0) * (h - 2 * pad)

    svg = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" font-family="sans-serif">']
    svg.append(f'<text x="{w/2}" y="24" text-anchor="middle" font-size="16" font-weight="bold">{title}</text>')
    # decade gridlines
    for d in range(math.floor(lx0), math.ceil(lx1) + 1):
        x = px(10 ** d)
        if pad - 1 <= x <= w - pad + 1:
            svg.append(f'<line x1="{x:.1f}" y1="{pad}" x2="{x:.1f}" y2="{h-pad}" stroke="#eee"/>')
            svg.append(f'<text x="{x:.1f}" y="{h-pad+16}" text-anchor="middle" font-size="10">1e{d}</text>')
    for d in range(math.floor(ly0), math.ceil(ly1) + 1):
        y = py(10 ** d)
        if pad - 1 <= y <= h - pad + 1:
            svg.append(f'<line x1="{pad}" y1="{y:.1f}" x2="{w-pad}" y2="{y:.1f}" stroke="#eee"/>')
            svg.append(f'<text x="{pad-8:.1f}" y="{y+4:.1f}" text-anchor="end" font-size="10">1e{d}</text>')
    svg.append(f'<line x1="{pad}" y1="{h-pad}" x2="{w-pad}" y2="{h-pad}" stroke="#333"/>')
    svg.append(f'<line x1="{pad}" y1="{pad}" x2="{pad}" y2="{h-pad}" stroke="#333"/>')
    svg.append(f'<text x="{w/2}" y="{h-24}" text-anchor="middle" font-size="11">{xlabel}</text>')
    svg.append(f'<text x="18" y="{h/2}" text-anchor="middle" font-size="11" transform="rotate(-90 18 {h/2})">{ylabel}</text>')
    for name, data, color in series:
        d = sorted((x, y) for x, y in data if x > 0 and y > 0)
        path_d = " ".join(f'{"M" if i == 0 else "L"}{px(x):.1f},{py(y):.1f}' for i, (x, y) in enumerate(d))
        svg.append(f'<path d="{path_d}" fill="none" stroke="{color}" stroke-width="2"/>')
        for x, y in d:
            svg.append(f'<circle cx="{px(x):.1f}" cy="{py(y):.1f}" r="3" fill="{color}"/>')
    for s, (name, _, color) in enumerate(series):
        lx = w - pad - 150
        svg.append(f'<rect x="{lx}" y="{pad + s*18}" width="12" height="12" fill="{color}"/>')
        svg.append(f'<text x="{lx+16}" y="{pad + s*18 + 11}" font-size="11">{name}</text>')
    svg.append('</svg>')
    path.write_text("\n".join(svg), encoding="utf-8")
    print("wrote", path.name)
