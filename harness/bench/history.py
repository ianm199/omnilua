#!/usr/bin/env python3
"""Build a static lua-rs perf-history dashboard from the chassis evidence ledger.

Reads harness/evidence/ledger.jsonl, filters to kind=bench rows, and writes a
self-contained HTML page with one SVG line chart per (workload, metric) pair
plotted over commits in time order.

Inspired by the redis-rs-port dashboard (harness/bench/history.py) but
trimmed to fit the simpler "ratio vs reference C Lua per workload" shape
that lua-rs uses. No external charting library — vanilla SVG so the output
is a single self-contained file.

Usage:
  python3 harness/bench/history.py
  python3 harness/bench/history.py --open    # open in default browser after build

Output:
  harness/bench/history/index.html
  harness/bench/history/history.json   (machine-readable timeline)
"""

from __future__ import annotations

import argparse
import html
import json
import subprocess
import webbrowser
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
LEDGER = ROOT / "harness/evidence/ledger.jsonl"
OUT_DIR = ROOT / "harness/bench/history"
REMOTE_COMMIT_PREFIX = "https://github.com/ianm199/lua-rs-port/commit/"


def commit_subject(sha: str) -> str:
    """Return the first line of the commit message for `sha`, or empty if
    git is unavailable / the commit is unreachable."""
    try:
        out = subprocess.check_output(
            ["git", "-C", str(ROOT), "log", "-1", "--format=%s", sha],
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
        return out
    except (subprocess.CalledProcessError, FileNotFoundError):
        return ""


def load_bench_rows() -> list[dict[str, Any]]:
    if not LEDGER.exists():
        return []
    rows: list[dict[str, Any]] = []
    with LEDGER.open() as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            r = json.loads(line)
            if r.get("kind") == "bench" and r.get("target") == "rust-vs-reference":
                rows.append(r)
    rows.sort(key=lambda r: (r.get("ts", ""), r.get("commit", "")))
    return rows


def build_timeline(rows: list[dict[str, Any]]) -> dict[str, Any]:
    """Pivot ledger rows into a structure the dashboard can render.

    Returns:
      {
        "commits": [{"sha": ..., "ts": ..., "subject": ...}, ...],
        "series": {
          "wall_ratio": {"fibonacci": [v0, v1, ...], "mandelbrot": [...], ...},
          "rss_ratio":  {...},
        },
        "workloads": [...],
        "metrics": ["wall_ratio", "rss_ratio"],
      }
    """
    seen_commits: dict[str, dict[str, str]] = {}
    for r in rows:
        sha = r["commit"]
        if sha not in seen_commits:
            seen_commits[sha] = {
                "sha": sha,
                "ts": r.get("ts", ""),
                "subject": commit_subject(sha),
            }
    commits = sorted(seen_commits.values(), key=lambda c: c["ts"])
    sha_order = [c["sha"] for c in commits]

    workloads = sorted({r["workload"] for r in rows})
    metrics = ["wall_ratio", "rss_ratio"]

    series: dict[str, dict[str, list[Any]]] = {
        m: {w: [None] * len(commits) for w in workloads} for m in metrics
    }
    for r in rows:
        i = sha_order.index(r["commit"])
        m = r["metric"]
        w = r["workload"]
        if m in series:
            series[m][w][i] = r["value"]

    return {
        "commits": commits,
        "series": series,
        "workloads": workloads,
        "metrics": metrics,
    }


def render_svg_chart(
    workload: str,
    values: list[float | None],
    commits: list[dict[str, str]],
    metric_label: str,
    width: int = 360,
    height: int = 200,
) -> str:
    """Render one SVG chart for a (workload, metric) series.

    Log scale on Y so a 10,000x ratio and a 2x ratio can be read on the
    same axis without dwarfing the small values.
    """
    pad_l, pad_r, pad_t, pad_b = 48, 12, 24, 36
    plot_w = width - pad_l - pad_r
    plot_h = height - pad_t - pad_b

    pts = [(i, v) for i, v in enumerate(values) if v is not None and v > 0]
    if not pts:
        return f'<svg viewBox="0 0 {width} {height}" class="chart"><text x="{width//2}" y="{height//2}" text-anchor="middle" fill="#888">(no data)</text></svg>'

    import math
    ys = [math.log10(v) for _, v in pts]
    y_min = min(ys + [math.log10(1.0)])
    y_max = max(ys + [math.log10(2.0)])
    if y_max - y_min < 0.3:
        y_max = y_min + 0.3
    n = max(len(commits), 2)

    def x_at(i: int) -> float:
        if n == 1:
            return pad_l + plot_w / 2
        return pad_l + (i / (n - 1)) * plot_w

    def y_at(logv: float) -> float:
        return pad_t + plot_h - (logv - y_min) / (y_max - y_min) * plot_h

    # Path through known data points
    path_d = ""
    last_cmd = "M"
    for i, v in enumerate(values):
        if v is None or v <= 0:
            last_cmd = "M"
            continue
        x = x_at(i)
        y = y_at(math.log10(v))
        path_d += f"{last_cmd}{x:.1f},{y:.1f} "
        last_cmd = "L"

    # Reference line at ratio = 1.0 (parity with reference Lua)
    y_one = y_at(0.0)
    parity_line = ""
    if y_one >= pad_t and y_one <= pad_t + plot_h:
        parity_line = (
            f'<line x1="{pad_l}" y1="{y_one:.1f}" x2="{pad_l+plot_w}" y2="{y_one:.1f}" '
            f'stroke="#bbb" stroke-dasharray="2,3" stroke-width="1"/>'
            f'<text x="{pad_l-4}" y="{y_one+3:.1f}" text-anchor="end" font-size="9" fill="#888">1x</text>'
        )

    # Y-axis log gridlines
    grid = ""
    tick_logs = []
    lo = int(math.floor(y_min))
    hi = int(math.ceil(y_max))
    for k in range(lo, hi + 1):
        if y_at(k) < pad_t - 1 or y_at(k) > pad_t + plot_h + 1:
            continue
        tick_logs.append(k)
    for k in tick_logs:
        y = y_at(k)
        label = f"{10**k:g}x"
        grid += (
            f'<line x1="{pad_l}" y1="{y:.1f}" x2="{pad_l+plot_w}" y2="{y:.1f}" '
            f'stroke="#eee" stroke-width="1"/>'
            f'<text x="{pad_l-4}" y="{y+3:.1f}" text-anchor="end" font-size="9" fill="#666">{label}</text>'
        )

    # Data points + per-commit tooltips
    points = ""
    for i, v in enumerate(values):
        if v is None or v <= 0:
            continue
        cx = x_at(i)
        cy = y_at(math.log10(v))
        sha = commits[i]["sha"]
        subject = commits[i].get("subject", "")
        title = f"{sha[:7]}  {v:.2f}x  {html.escape(subject)}"
        points += (
            f'<g class="pt">'
            f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="3.5" fill="#2f6fed"/>'
            f'<text x="{cx:.1f}" y="{cy-7:.1f}" text-anchor="middle" font-size="9" fill="#333">{v:.1f}x</text>'
            f'<title>{title}</title>'
            f'</g>'
        )

    # X-axis labels (commits, short SHA)
    x_labels = ""
    for i, c in enumerate(commits):
        x = x_at(i)
        label = c["sha"][:7]
        x_labels += (
            f'<text x="{x:.1f}" y="{pad_t+plot_h+14}" text-anchor="middle" '
            f'font-size="9" fill="#666">{label}</text>'
        )

    return (
        f'<svg viewBox="0 0 {width} {height}" class="chart">'
        f'<text x="{pad_l}" y="14" font-size="11" font-weight="600" fill="#222">{html.escape(workload)}</text>'
        f'<text x="{width-pad_r}" y="14" font-size="9" text-anchor="end" fill="#888">{metric_label}</text>'
        f'{grid}{parity_line}'
        f'<path d="{path_d}" fill="none" stroke="#2f6fed" stroke-width="1.5"/>'
        f'{points}{x_labels}'
        f'</svg>'
    )


def render_html(timeline: dict[str, Any]) -> str:
    commits = timeline["commits"]
    workloads = timeline["workloads"]
    series = timeline["series"]

    if not commits:
        return "<!doctype html><html><body><p>No bench data in ledger yet. Run <code>bash harness/bench/compare.sh</code> first.</p></body></html>"

    head = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>lua-rs perf history</title>
<style>
  body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
         margin: 24px; color: #222; max-width: 1200px; }
  h1 { font-size: 22px; margin-bottom: 4px; }
  .lede { color: #666; font-size: 13px; margin-bottom: 24px; }
  .lede a { color: #2f6fed; text-decoration: none; }
  .lede a:hover { text-decoration: underline; }
  h2 { font-size: 16px; margin-top: 32px; margin-bottom: 8px; color: #444; }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(360px, 1fr));
          gap: 16px; }
  .card { border: 1px solid #ddd; border-radius: 6px; padding: 8px; background: white; }
  .chart { width: 100%; height: auto; }
  table.commits { border-collapse: collapse; font-size: 12px; margin-top: 12px; }
  table.commits td, table.commits th { padding: 4px 12px 4px 0; text-align: left; vertical-align: top; }
  table.commits th { color: #666; font-weight: 500; border-bottom: 1px solid #ddd; }
  code { font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 12px; }
  .note { color: #888; font-size: 12px; margin-top: 24px; padding-top: 12px; border-top: 1px solid #eee; }
</style>
</head>
<body>
"""
    title = (
        f'<h1>lua-rs perf history</h1>'
        f'<div class="lede">Ratio of lua-rs wall-clock (and max RSS) over upstream Lua 5.4.7, '
        f'per workload, plotted over commits in time order. Log Y-axis. '
        f'Lower is better; the dashed line marks parity (1x). '
        f'Source: <code>harness/evidence/ledger.jsonl</code> (kind=bench).</div>'
    )

    body = title

    for metric in ("wall_ratio", "rss_ratio"):
        label = "wall-clock vs reference" if metric == "wall_ratio" else "max RSS vs reference"
        body += f'<h2>{html.escape(label)}</h2>'
        body += '<div class="grid">'
        for w in workloads:
            chart = render_svg_chart(w, series[metric][w], commits, label)
            body += f'<div class="card">{chart}</div>'
        body += '</div>'

    body += '<h2>Commits in this timeline</h2><table class="commits">'
    body += '<thead><tr><th>SHA</th><th>UTC</th><th>Subject</th></tr></thead><tbody>'
    for c in commits:
        sha = html.escape(c["sha"])
        ts = html.escape(c.get("ts", ""))
        subject = html.escape(c.get("subject", ""))
        link = f'<a href="{REMOTE_COMMIT_PREFIX}{sha}" target="_blank"><code>{sha[:10]}</code></a>'
        body += f'<tr><td>{link}</td><td>{ts}</td><td>{subject}</td></tr>'
    body += '</tbody></table>'

    body += (
        '<div class="note">'
        'Each datapoint is the best wall-clock (and max RSS) across the runs configured for that bench '
        'invocation (typically N=5). The reference column is upstream Lua 5.4.7 (<code>reference/lua-5.4.7/src/lua</code>) '
        'compiled with <code>make macosx</code>. lua-rs is the matching release build '
        '(<code>target/release/lua-rs</code>). Hardware fingerprint is in <code>harness/bench/results/&lt;ts&gt;-&lt;sha&gt;-compare.json</code>; '
        'do not merge datapoints from different machines.'
        '</div>'
    )

    return head + body + "</body></html>"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--open", action="store_true", help="open the resulting HTML in the default browser")
    args = ap.parse_args()

    rows = load_bench_rows()
    timeline = build_timeline(rows)

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "history.json").write_text(json.dumps(timeline, indent=2, sort_keys=True))
    html_out = render_html(timeline)
    out_path = OUT_DIR / "index.html"
    out_path.write_text(html_out)
    print(f"wrote {out_path} ({len(rows)} bench rows, {len(timeline['commits'])} commits, {len(timeline['workloads'])} workloads)")

    if args.open:
        webbrowser.open(out_path.as_uri())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
