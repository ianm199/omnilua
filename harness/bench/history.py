#!/usr/bin/env python3
"""Build a static lua-rs perf-history dashboard from the chassis evidence ledger.

Aesthetic, CSS, and chart-drawing JS are a direct adaptation of
``redis-rs-port/harness/bench/history.py`` so the two pilot dashboards
look like siblings. The data shape is lua-specific: per-commit
wall_ratio and rss_ratio per workload, sourced from
``harness/evidence/ledger.jsonl`` (kind=bench, target=rust-vs-reference).

Usage:
  python3 harness/bench/history.py
  python3 harness/bench/history.py --serve --port 8055
"""

from __future__ import annotations

import argparse
import html
import json
import math
import statistics
import subprocess
from datetime import datetime, timezone
from http.server import ThreadingHTTPServer, SimpleHTTPRequestHandler
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
LEDGER = ROOT / "harness/evidence/ledger.jsonl"
DEFAULT_OUT = ROOT / "harness/bench/history"


WORKLOADS = [
    "fibonacci",
    "mandelbrot",
    "binarytrees",
    "closure_ops",
    "table_ops",
    "table_ops_long",
    "string_ops",
    "string_ops_long",
]

WORKLOAD_COLORS = {
    "fibonacci":       "#2f6fed",
    "mandelbrot":      "#0f8f68",
    "binarytrees":     "#c16a1a",
    "closure_ops":     "#7a4cc2",
    "table_ops":       "#d33f49",
    "table_ops_long":  "#a13d63",
    "string_ops":      "#008c9e",
    "string_ops_long": "#8a6500",
}


def load_ledger_rows() -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Returns (bench_rows, test_rows). Bench rows are kind=bench/target=rust-vs-reference;
    test rows are kind=tests/target=official-suite."""
    bench_rows: list[dict[str, Any]] = []
    test_rows: list[dict[str, Any]] = []
    if not LEDGER.exists():
        return bench_rows, test_rows
    for line in LEDGER.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        kind = row.get("kind")
        if kind == "bench" and row.get("target") == "rust-vs-reference":
            if row.get("metric") in {"wall_ratio", "rss_ratio"} and row.get("workload") in WORKLOAD_COLORS:
                bench_rows.append(row)
        elif kind == "tests" and row.get("target") == "official-suite":
            test_rows.append(row)
    return bench_rows, test_rows


def parse_ts(ts: str) -> datetime:
    """Parse either compact (20260523T030313Z) or ISO (2026-05-22T22:17:47Z[.ffffff]) UTC timestamps."""
    for fmt in ("%Y%m%dT%H%M%SZ", "%Y-%m-%dT%H:%M:%S.%fZ", "%Y-%m-%dT%H:%M:%SZ"):
        try:
            return datetime.strptime(ts, fmt).replace(tzinfo=timezone.utc)
        except ValueError:
            continue
    raise ValueError(f"unrecognized timestamp format: {ts!r}")


def git_commit_info(commits: list[str]) -> dict[str, dict[str, str]]:
    info: dict[str, dict[str, str]] = {}
    for commit in commits:
        try:
            result = subprocess.run(
                ["git", "log", "-1", "--format=%H%n%h%n%ad%n%s", "--date=iso-strict", commit],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=True,
            )
        except (subprocess.CalledProcessError, FileNotFoundError):
            continue
        lines = result.stdout.strip().split("\n")
        if len(lines) < 4:
            continue
        info[commit] = {
            "full_sha": lines[0],
            "short": lines[1],
            "date": lines[2],
            "subject": lines[3],
        }
    return info


def build_history() -> dict[str, Any]:
    rows, test_rows = load_ledger_rows()
    rows.sort(key=lambda r: r["ts"])
    test_rows.sort(key=lambda r: r["ts"])
    unique_commits = list(dict.fromkeys([r["commit"] for r in rows] + [r["commit"] for r in test_rows]))
    commit_info = git_commit_info(unique_commits)

    points: list[dict[str, Any]] = []
    series: dict[str, list[dict[str, Any]]] = {}
    for row in rows:
        ts = row["ts"]
        commit = row["commit"]
        workload = row["workload"]
        metric = row["metric"]
        value = float(row["value"])
        info = commit_info.get(commit, {})
        point = {
            "ts": parse_ts(ts).isoformat(),
            "ts_raw": ts,
            "commit": commit,
            "commit_subject": info.get("subject", ""),
            "commit_date": info.get("date", ""),
            "workload": workload,
            "metric": metric,
            "value": value,
            "evidence": row.get("evidence", ""),
            "runs": row.get("runs", 1),
        }
        points.append(point)
        series_id = f"{metric}_{workload}"
        series.setdefault(series_id, []).append(point)

    for series_points in series.values():
        series_points.sort(key=lambda p: p["ts"])

    latest_by_workload: dict[str, dict[str, dict[str, Any]]] = {}
    for metric in ("wall_ratio", "rss_ratio"):
        for workload in WORKLOADS:
            series_points = series.get(f"{metric}_{workload}", [])
            if series_points:
                latest_by_workload.setdefault(workload, {})[metric] = series_points[-1]

    wall_latest_values = [
        latest_by_workload[w]["wall_ratio"]["value"]
        for w in WORKLOADS
        if w in latest_by_workload and "wall_ratio" in latest_by_workload[w]
    ]
    rss_latest_values = [
        latest_by_workload[w]["rss_ratio"]["value"]
        for w in WORKLOADS
        if w in latest_by_workload and "rss_ratio" in latest_by_workload[w]
    ]

    def geomean(values: list[float]) -> float:
        if not values:
            return 0.0
        log_values = [math.log(max(v, 1e-9)) for v in values]
        return math.exp(statistics.fmean(log_values))

    headline = {
        "wall_geomean": geomean(wall_latest_values),
        "rss_geomean": geomean(rss_latest_values),
        "best_workload": min(
            ((w, latest_by_workload[w]["wall_ratio"]["value"]) for w in latest_by_workload if "wall_ratio" in latest_by_workload[w]),
            key=lambda kv: kv[1],
            default=("-", 0.0),
        ),
        "worst_workload": max(
            ((w, latest_by_workload[w]["wall_ratio"]["value"]) for w in latest_by_workload if "wall_ratio" in latest_by_workload[w]),
            key=lambda kv: kv[1],
            default=("-", 0.0),
        ),
        "n_commits": len(unique_commits),
        "n_points": len(points),
    }

    test_points: list[dict[str, Any]] = []
    for row in test_rows:
        info = commit_info.get(row["commit"], {})
        total = int(row.get("total", 0) or 0)
        pass_count = int(row.get("value", 0) or 0)
        pass_rate = (pass_count / total * 100.0) if total > 0 else 0.0
        test_points.append({
            "ts": parse_ts(row["ts"]).isoformat(),
            "ts_raw": row["ts"],
            "commit": row["commit"],
            "commit_subject": info.get("subject", ""),
            "pass_count": pass_count,
            "total": total,
            "fail": int(row.get("fail", 0) or 0),
            "timeout": int(row.get("timeout", 0) or 0),
            "runtime_s": int(row.get("runtime_s", 0) or 0),
            "value": pass_rate,
        })

    latest_test = test_points[-1] if test_points else None

    signature = {
        "n_points": len(points) + len(test_points),
        "latest_ts": max(
            (points[-1]["ts_raw"] if points else ""),
            (test_points[-1]["ts_raw"] if test_points else ""),
        ),
        "latest_commit": (test_points[-1]["commit"] if test_points else (points[-1]["commit"] if points else "")),
    }

    return {
        "generated_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "point_count": len(points),
        "commit_count": len(unique_commits),
        "points": points,
        "series": series,
        "latest_by_workload": latest_by_workload,
        "headline": headline,
        "signature": signature,
        "workloads": WORKLOADS,
        "workload_colors": WORKLOAD_COLORS,
        "test_points": test_points,
        "latest_test": latest_test,
    }


def fmt_ratio(v: float) -> str:
    if v >= 1000:
        return f"{v:,.0f}×"
    if v >= 100:
        return f"{v:.1f}×"
    return f"{v:.2f}×"


def render_html(history: dict[str, Any]) -> str:
    headline = history["headline"]
    best_w, best_v = headline["best_workload"]
    worst_w, worst_v = headline["worst_workload"]

    card_html = []
    card_html.append(f"""
      <div class="metric-card">
        <div class="eyebrow">Wall-time geomean</div>
        <div class="metric">{fmt_ratio(headline['wall_geomean'])}</div>
        <div class="subtle">over {len([w for w in history['latest_by_workload'] if 'wall_ratio' in history['latest_by_workload'][w]])} workloads at latest commit</div>
      </div>
    """)
    card_html.append(f"""
      <div class="metric-card">
        <div class="eyebrow">RSS geomean</div>
        <div class="metric">{fmt_ratio(headline['rss_geomean'])}</div>
        <div class="subtle">resident-set vs reference lua-c</div>
      </div>
    """)
    card_html.append(f"""
      <div class="metric-card">
        <div class="eyebrow">Best workload</div>
        <div class="metric">{fmt_ratio(best_v)}</div>
        <div class="subtle">{html.escape(best_w)}</div>
      </div>
    """)
    card_html.append(f"""
      <div class="metric-card">
        <div class="eyebrow">Worst workload</div>
        <div class="metric">{fmt_ratio(worst_v)}</div>
        <div class="subtle">{html.escape(worst_w)}</div>
      </div>
    """)
    series_defs = {}
    for workload in WORKLOADS:
        color = WORKLOAD_COLORS[workload]
        series_defs[f"wall_ratio_{workload}"] = {"label": workload, "color": color, "metric": "wall_ratio"}
        series_defs[f"rss_ratio_{workload}"] = {"label": workload, "color": color, "metric": "rss_ratio"}
    series_defs["tests_pass_rate"] = {"label": "official suite pass-rate", "color": "#0f8f68", "metric": "pass_rate"}

    wall_ids = [f"wall_ratio_{w}" for w in WORKLOADS]
    rss_ids = [f"rss_ratio_{w}" for w in WORKLOADS]

    series_with_tests = dict(history["series"])
    series_with_tests["tests_pass_rate"] = history["test_points"]

    payload = {
        "generated_at": history["generated_at"],
        "point_count": history["point_count"],
        "commit_count": history["commit_count"],
        "signature": history["signature"],
        "points": history["points"],
        "series": series_with_tests,
        "series_defs": series_defs,
        "wall_ids": wall_ids,
        "rss_ids": rss_ids,
        "test_ids": ["tests_pass_rate"],
        "test_points": history["test_points"],
    }
    history_json = json.dumps(payload, separators=(",", ":"))

    return f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>lua-rs Performance History</title>
  <style>
    :root {{
      --bg: #f7f8fb;
      --panel: #ffffff;
      --text: #18202f;
      --muted: #5e6878;
      --line: #d8deea;
      --accent: #2f6fed;
      --good: #0f8f68;
      --warn: #c16a1a;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }}
    * {{ box-sizing: border-box; }}
    body {{ margin: 0; background: var(--bg); color: var(--text); }}
    main {{ max-width: 1440px; margin: 0 auto; padding: 28px; }}
    header {{ display: flex; justify-content: space-between; gap: 24px; align-items: flex-start; margin-bottom: 24px; }}
    h1 {{ margin: 0 0 8px; font-size: 28px; letter-spacing: 0; }}
    h2 {{ margin: 0 0 12px; font-size: 18px; letter-spacing: 0; }}
    p {{ margin: 0; color: var(--muted); line-height: 1.5; }}
    a {{ color: var(--accent); text-decoration: none; }}
    a:hover {{ text-decoration: underline; }}
    .grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 12px; }}
    .metric-card, .panel {{ background: var(--panel); border: 1px solid var(--line); border-radius: 8px; }}
    .metric-card {{ padding: 16px; }}
    .eyebrow {{ color: var(--muted); font-size: 12px; text-transform: uppercase; letter-spacing: .08em; }}
    .metric {{ margin-top: 4px; font-size: 32px; font-weight: 700; }}
    .subtle {{ color: var(--muted); font-size: 12px; margin-top: 5px; overflow-wrap: anywhere; }}
    .panel {{ padding: 18px; margin-top: 14px; }}
    .chart-wrap {{ width: 100%; overflow-x: auto; }}
    svg {{ display: block; width: 100%; min-width: 880px; height: 420px; }}
    .axis {{ stroke: #9aa6bb; stroke-width: 1; }}
    .gridline {{ stroke: #e9edf5; stroke-width: 1; }}
    .series-line {{ fill: none; stroke-width: 2.5; }}
    .point {{ stroke: #fff; stroke-width: 1.5; }}
    .legend {{ display: flex; flex-wrap: wrap; gap: 10px 18px; margin: 12px 0 0; }}
    .legend-item {{ display: inline-flex; align-items: center; gap: 7px; color: var(--muted); font-size: 13px; cursor: pointer; user-select: none; }}
    .legend-item.muted-series {{ opacity: 0.35; }}
    .swatch {{ width: 11px; height: 11px; border-radius: 999px; display: inline-block; }}
    table {{ width: 100%; border-collapse: collapse; font-size: 13px; }}
    th, td {{ text-align: left; border-bottom: 1px solid var(--line); padding: 9px 8px; vertical-align: top; }}
    th {{ color: var(--muted); font-weight: 600; }}
    code {{ font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 12px; }}
    .note {{ background: #eef4ff; border: 1px solid #cbdcff; color: #23314d; border-radius: 8px; padding: 12px 14px; }}
    .warn-note {{ background: #fff7eb; border-color: #f0c994; color: #3c2a12; }}
    .tooltip {{
      position: fixed;
      z-index: 20;
      max-width: 360px;
      pointer-events: none;
      background: rgba(24, 32, 47, .96);
      color: #fff;
      border-radius: 8px;
      padding: 10px 12px;
      box-shadow: 0 10px 24px rgba(20, 30, 45, .18);
      font-size: 12px;
      line-height: 1.35;
      opacity: 0;
      transform: translate(-50%, calc(-100% - 12px));
      transition: opacity .08s ease-out;
      overflow-wrap: anywhere;
    }}
    .tooltip strong {{ display: block; font-size: 13px; margin-bottom: 4px; }}
    .tooltip .muted {{ color: #c9d3e4; }}
    .tooltip.show {{ opacity: 1; }}
    .point-hit {{ fill: transparent; cursor: crosshair; }}
    .refresh-status {{ text-align: right; }}
    .bench-info {{
      margin: 14px 0 0;
      display: grid;
      grid-template-columns: max-content 1fr;
      gap: 6px 14px;
      font-size: 12px;
      background: #f4f6fb;
      border: 1px solid var(--line);
      border-radius: 8px;
      padding: 10px 14px;
    }}
    .bench-info dt {{
      font-size: 11px;
      text-transform: uppercase;
      letter-spacing: .08em;
      color: var(--muted);
      font-weight: 600;
      margin: 0;
    }}
    .bench-info dd {{ margin: 0; color: var(--text); }}
    .bench-info code {{ background: rgba(47, 111, 237, .08); padding: 1px 5px; border-radius: 4px; }}
    .control-row {{ display: flex; gap: 18px; align-items: center; margin: 10px 0; flex-wrap: wrap; font-size: 13px; color: var(--muted); }}
    .control-row .group-label {{ font-weight: 600; color: var(--text); }}
    .pill-group {{ display: inline-flex; gap: 4px; background: #eef1f7; border: 1px solid var(--line); border-radius: 999px; padding: 3px; }}
    .pill {{ border: 0; background: transparent; color: var(--muted); font-size: 12px; padding: 4px 12px; border-radius: 999px; cursor: pointer; font-weight: 600; }}
    .pill:hover {{ color: var(--text); }}
    .pill.active {{ background: var(--accent); color: #fff; }}
    .pill.active:hover {{ color: #fff; }}
    .overflow-arrow {{ fill: #5e6878; opacity: .7; }}
    @media (max-width: 900px) {{
      main {{ padding: 18px; }}
      header {{ display: block; }}
      .grid {{ grid-template-columns: 1fr; }}
    }}
  </style>
</head>
<body>
<main>
  <header>
    <div>
      <h1>lua-rs Performance History</h1>
      <p>Commit-keyed bench trajectory generated from <code>harness/evidence/ledger.jsonl</code>. Each point is one <code>compare.sh</code> run; the y-axis is the ratio of lua-rs to reference lua-c on the same workload. Parity at <code>1.00×</code>.</p>
    </div>
    <p class="subtle refresh-status">Generated {html.escape(history['generated_at'])}<br>{history['point_count']} measurements over {history['commit_count']} commits<br><span id="refresh-status">Auto-refresh enabled</span></p>
  </header>

  <section class="grid">
    {''.join(card_html)}
  </section>

  <section class="panel">
    <h2>Wall-time ratio per workload</h2>
    <p>Ratio = lua-rs wall time ÷ reference lua-c wall time on the same workload. Lower is better; <code>1.00×</code> is parity. Click a legend chip to mute that series. Off-screen points show as small triangles at the top edge.</p>
    <div class="control-row">
      <span class="group-label">y-max</span>
      <span class="pill-group" id="wall-ymax-pills"></span>
      <span class="group-label">window</span>
      <span class="pill-group" id="wall-window-pills"></span>
    </div>
    <div class="chart-wrap"><svg id="wall-chart" role="img" aria-label="Wall-time ratio per workload over time"></svg></div>
    <div class="legend" id="wall-legend"></div>
    <dl class="bench-info">
      <dt>Tool</dt><dd><code>harness/bench/compare.sh</code> · 3–5 runs per commit, min wall picked</dd>
      <dt>Reference</dt><dd>PUC-Rio Lua 5.4.7 built from <code>reference/lua-c/</code></dd>
      <dt>Workloads</dt><dd><code>fibonacci</code> · <code>mandelbrot</code> · <code>binarytrees</code> · <code>closure_ops</code> · <code>table_ops</code> · <code>table_ops_long</code> · <code>string_ops</code> · <code>string_ops_long</code></dd>
      <dt>Reading</dt><dd>Each point's y-value = <code>lua-rs wall / lua-c wall</code> on the matching workload, at the commit being benchmarked.</dd>
    </dl>
  </section>

  <section class="panel">
    <h2>RSS ratio per workload</h2>
    <p>Ratio = lua-rs peak resident-set ÷ reference lua-c peak resident-set on the same workload. Lower is better; <code>1.00×</code> is parity.</p>
    <div class="control-row">
      <span class="group-label">y-max</span>
      <span class="pill-group" id="rss-ymax-pills"></span>
      <span class="group-label">window</span>
      <span class="pill-group" id="rss-window-pills"></span>
    </div>
    <div class="chart-wrap"><svg id="rss-chart" role="img" aria-label="RSS ratio per workload over time"></svg></div>
    <div class="legend" id="rss-legend"></div>
  </section>

  <section class="panel">
    <h2>Latest per-workload status</h2>
    <table id="latest-table">
      <thead>
        <tr><th>Workload</th><th>Wall ratio (latest)</th><th>RSS ratio (latest)</th><th>Parity gate</th><th>Last commit</th></tr>
      </thead>
      <tbody></tbody>
    </table>
  </section>

  <section class="panel">
    <h2>Recent runs</h2>
    <p>All bench measurements from <code>harness/evidence/ledger.jsonl</code>, newest first.</p>
    <table id="runs-table">
      <thead>
        <tr><th>When</th><th>Commit</th><th>Subject</th><th>Workload</th><th>Wall ratio</th><th>RSS ratio</th><th>Runs</th></tr>
      </thead>
      <tbody></tbody>
    </table>
  </section>
</main>

<div id="chart-tooltip" class="tooltip"></div>

<script>
const HISTORY = {history_json};
const INITIAL_SIGNATURE = JSON.stringify(HISTORY.signature || {{}});
const SERIES = HISTORY.series_defs;

function fmtRatio(v) {{
  if (v === null || v === undefined || Number.isNaN(v)) return "-";
  if (v >= 1000) return v.toLocaleString(undefined, {{maximumFractionDigits: 0}}) + "×";
  if (v >= 100) return v.toFixed(1) + "×";
  return v.toFixed(2) + "×";
}}
function fmtVal(v, unit) {{
  if (v === null || v === undefined || Number.isNaN(v)) return "-";
  if (unit === "%") return v.toFixed(1) + "%";
  return fmtRatio(v);
}}

function shortTime(iso) {{
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {{
    year: "2-digit", month: "2-digit", day: "2-digit",
    hour: "2-digit", minute: "2-digit",
  }});
}}

function shortCommit(c) {{ return (c || "").slice(0, 7); }}

function tooltipHtml(spec, p) {{
  const unit = spec.metric === "pass_rate" ? "%" : "×";
  const extras = (spec.metric === "pass_rate")
    ? `<div class="muted">pass=${{p.pass_count}}/${{p.total}} · fail=${{p.fail}} · timeout=${{p.timeout}} · runtime=${{p.runtime_s}}s</div>`
    : "";
  return `
    <strong>${{spec.label}}</strong>
    <div>${{fmtVal(p.value, unit)}} <span class="muted">(${{spec.metric}})</span></div>
    <div class="muted">${{shortCommit(p.commit)}} · ${{shortTime(p.ts)}}</div>
    <div class="muted">${{p.commit_subject || ""}}</div>
    ${{extras}}
  `;
}}

function showTooltip(event, htmlText) {{
  const tip = document.getElementById("chart-tooltip");
  tip.innerHTML = htmlText;
  tip.style.left = event.clientX + "px";
  tip.style.top = event.clientY + "px";
  tip.classList.add("show");
}}
function moveTooltip(event) {{
  const tip = document.getElementById("chart-tooltip");
  tip.style.left = event.clientX + "px";
  tip.style.top = event.clientY + "px";
}}
function hideTooltip() {{
  document.getElementById("chart-tooltip").classList.remove("show");
}}

const MUTED_SERIES = new Set();

function pickGridStep(yMax) {{
  if (yMax <= 3) return 0.25;
  if (yMax <= 10) return 1;
  if (yMax <= 30) return 5;
  if (yMax <= 100) return 10;
  if (yMax <= 500) return 50;
  if (yMax <= 2000) return 250;
  return Math.pow(10, Math.floor(Math.log10(yMax / 8)));
}}

function drawChart(svgId, legendId, seriesIds, opts = {{}}) {{
  const svg = document.getElementById(svgId);
  const legend = document.getElementById(legendId);
  const width = 1120, height = 420;
  const margin = {{left: 64, right: 24, top: 24, bottom: 70}};
  svg.setAttribute("viewBox", `0 0 ${{width}} ${{height}}`);
  svg.innerHTML = "";
  legend.innerHTML = "";

  const active = seriesIds.filter(id => !MUTED_SERIES.has(svgId + "::" + id));
  const all = active.flatMap(id => (HISTORY.series[id] || []).map(p => ({{...p, seriesId: id}})));
  if (!all.length) return;

  const sortedAll = [...all].sort((a, b) => new Date(a.ts) - new Date(b.ts));
  const allTimestamps = [];
  const seenAllTs = new Set();
  for (const p of sortedAll) {{
    if (!p.ts || seenAllTs.has(p.ts)) continue;
    seenAllTs.add(p.ts);
    allTimestamps.push(p.ts);
  }}

  const windowN = opts.windowN || allTimestamps.length;
  const allowedTs = new Set(allTimestamps.slice(Math.max(0, allTimestamps.length - windowN)));

  const filtered = sortedAll.filter(p => allowedTs.has(p.ts));
  if (!filtered.length) return;

  const tickPoints = filtered.filter((p, idx, arr) => idx === 0 || arr[idx - 1].ts !== p.ts);
  const timestamps = [...allowedTs];
  const xByTs = new Map(timestamps.map((ts, idx) => [ts, idx]));
  const maxIndex = Math.max(1, timestamps.length - 1);

  const yMax = opts.yMax || 10;
  const yMin = 0;

  const chartW = width - margin.left - margin.right;
  const chartH = height - margin.top - margin.bottom;
  const x = ts => margin.left + (xByTs.get(ts) ?? 0) / maxIndex * chartW;
  const y = value => margin.top + (yMax - Math.min(value, yMax)) / (yMax - yMin) * chartH;

  let gridTicks = [];
  const step = pickGridStep(yMax);
  for (let v = 0; v <= yMax + 0.0001; v += step) gridTicks.push(v);

  for (const tick of gridTicks) {{
    const yy = y(tick);
    const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
    line.setAttribute("x1", margin.left);
    line.setAttribute("x2", width - margin.right);
    line.setAttribute("y1", yy);
    line.setAttribute("y2", yy);
    line.setAttribute("class", "gridline");
    svg.appendChild(line);
    const text = document.createElementNS("http://www.w3.org/2000/svg", "text");
    text.setAttribute("x", margin.left - 10);
    text.setAttribute("y", yy + 4);
    text.setAttribute("text-anchor", "end");
    text.setAttribute("font-size", "11");
    text.setAttribute("fill", "#5e6878");
    const unit = opts.unit || "×";
    text.textContent = (tick >= 1000 ? tick.toLocaleString() : (tick >= 10 ? tick.toFixed(0) : tick.toFixed(2))) + unit;
    svg.appendChild(text);
  }}

  const xAxis = document.createElementNS("http://www.w3.org/2000/svg", "line");
  xAxis.setAttribute("x1", margin.left);
  xAxis.setAttribute("x2", width - margin.right);
  xAxis.setAttribute("y1", height - margin.bottom);
  xAxis.setAttribute("y2", height - margin.bottom);
  xAxis.setAttribute("class", "axis");
  svg.appendChild(xAxis);

  tickPoints.forEach((point, idx) => {{
    if (idx % Math.ceil(tickPoints.length / 10) !== 0 && idx !== tickPoints.length - 1) return;
    const tx = x(point.ts);
    const text = document.createElementNS("http://www.w3.org/2000/svg", "text");
    text.setAttribute("x", tx);
    text.setAttribute("y", height - margin.bottom + 20);
    text.setAttribute("text-anchor", "end");
    text.setAttribute("font-size", "11");
    text.setAttribute("fill", "#5e6878");
    text.setAttribute("transform", `rotate(-35 ${{tx}} ${{height - margin.bottom + 20}})`);
    text.textContent = shortCommit(point.commit);
    svg.appendChild(text);
  }});

  for (const id of seriesIds) {{
    const spec = SERIES[id];
    const muted = MUTED_SERIES.has(svgId + "::" + id);
    const allPoints = (HISTORY.series[id] || []);
    const points = allPoints.filter(p => allowedTs.has(p.ts));
    if (points.length && !muted) {{
      const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
      path.setAttribute("class", "series-line");
      path.setAttribute("stroke", spec.color);
      path.setAttribute("d", points.map((p, idx) => `${{idx ? "L" : "M"}} ${{x(p.ts).toFixed(1)}} ${{y(p.value).toFixed(1)}}`).join(" "));
      svg.appendChild(path);

      points.forEach(p => {{
        const overflow = p.value > yMax;
        if (overflow) {{
          const tri = document.createElementNS("http://www.w3.org/2000/svg", "polygon");
          const tx = x(p.ts);
          const ty = margin.top + 1;
          tri.setAttribute("points", `${{tx}},${{ty}} ${{tx - 5}},${{ty + 8}} ${{tx + 5}},${{ty + 8}}`);
          tri.setAttribute("fill", spec.color);
          tri.setAttribute("class", "overflow-arrow");
          svg.appendChild(tri);
        }} else {{
          const circle = document.createElementNS("http://www.w3.org/2000/svg", "circle");
          circle.setAttribute("class", "point");
          circle.setAttribute("cx", x(p.ts));
          circle.setAttribute("cy", y(p.value));
          circle.setAttribute("r", 4);
          circle.setAttribute("fill", spec.color);
          svg.appendChild(circle);
        }}

        const hit = document.createElementNS("http://www.w3.org/2000/svg", "circle");
        hit.setAttribute("class", "point-hit");
        hit.setAttribute("cx", x(p.ts));
        hit.setAttribute("cy", overflow ? margin.top + 5 : y(p.value));
        hit.setAttribute("r", 11);
        hit.addEventListener("mouseenter", event => showTooltip(event, tooltipHtml(spec, p)));
        hit.addEventListener("mousemove", moveTooltip);
        hit.addEventListener("mouseleave", hideTooltip);
        svg.appendChild(hit);
      }});
    }}

    const item = document.createElement("span");
    item.className = "legend-item" + (muted ? " muted-series" : "");
    item.innerHTML = `<span class="swatch" style="background:${{spec.color}}"></span>${{spec.label}}`;
    item.addEventListener("click", () => {{
      const key = svgId + "::" + id;
      if (MUTED_SERIES.has(key)) MUTED_SERIES.delete(key);
      else MUTED_SERIES.add(key);
      drawChart(svgId, legendId, seriesIds, opts);
    }});
    legend.appendChild(item);
  }}

  const parityValue = opts.parityValue !== undefined ? opts.parityValue : 1.0;
  const parityLabel = opts.parityLabel || `parity (${{parityValue.toFixed(2)}}${{opts.unit || "×"}})`;
  if (parityValue !== null && parityValue >= yMin && parityValue <= yMax) {{
    const py = y(parityValue);
    const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
    line.setAttribute("x1", margin.left);
    line.setAttribute("x2", width - margin.right);
    line.setAttribute("y1", py);
    line.setAttribute("y2", py);
    line.setAttribute("stroke", "#0f8f68");
    line.setAttribute("stroke-width", "1.5");
    line.setAttribute("stroke-dasharray", "6 4");
    svg.appendChild(line);
    const label = document.createElementNS("http://www.w3.org/2000/svg", "text");
    label.setAttribute("x", width - margin.right - 6);
    label.setAttribute("y", py - 6);
    label.setAttribute("text-anchor", "end");
    label.setAttribute("font-size", "11");
    label.setAttribute("fill", "#0f8f68");
    label.setAttribute("font-weight", "600");
    label.textContent = parityLabel;
    svg.appendChild(label);
  }}

  const gateValue = opts.gateValue !== undefined ? opts.gateValue : 1.5;
  const gateLabel = opts.gateLabel || `parity gate (${{gateValue ? gateValue.toFixed(2) : ""}}${{opts.unit || "×"}})`;
  if (gateValue !== null && gateValue > yMin && gateValue <= yMax) {{
    const py = y(gateValue);
    const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
    line.setAttribute("x1", margin.left);
    line.setAttribute("x2", width - margin.right);
    line.setAttribute("y1", py);
    line.setAttribute("y2", py);
    line.setAttribute("stroke", "#c16a1a");
    line.setAttribute("stroke-width", "1");
    line.setAttribute("stroke-dasharray", "3 4");
    svg.appendChild(line);
    const label = document.createElementNS("http://www.w3.org/2000/svg", "text");
    label.setAttribute("x", width - margin.right - 6);
    label.setAttribute("y", py - 4);
    label.setAttribute("text-anchor", "end");
    label.setAttribute("font-size", "10");
    label.setAttribute("fill", "#c16a1a");
    label.textContent = gateLabel;
    svg.appendChild(label);
  }}
}}

function renderLatestTable() {{
  const tbody = document.querySelector("#latest-table tbody");
  tbody.innerHTML = "";
  for (const workload of HISTORY.points.map(p => p.workload).filter((v, i, a) => a.indexOf(v) === i)) {{
    const wallSeries = HISTORY.series["wall_ratio_" + workload] || [];
    const rssSeries = HISTORY.series["rss_ratio_" + workload] || [];
    const wall = wallSeries[wallSeries.length - 1];
    const rss = rssSeries[rssSeries.length - 1];
    if (!wall && !rss) continue;
    const gate = wall ? (wall.value <= 1.5 ? '<span style="color:var(--good);font-weight:600">✓ parity</span>' : '<span style="color:var(--warn);font-weight:600">over gate</span>') : "-";
    const tr = document.createElement("tr");
    tr.innerHTML = `
      <td><strong>${{workload}}</strong></td>
      <td>${{wall ? fmtRatio(wall.value) : "-"}}</td>
      <td>${{rss ? fmtRatio(rss.value) : "-"}}</td>
      <td>${{gate}}</td>
      <td><code>${{shortCommit(wall ? wall.commit : (rss ? rss.commit : ""))}}</code><div class="subtle">${{(wall ? wall.commit_subject : (rss ? rss.commit_subject : "")) || ""}}</div></td>
    `;
    tbody.appendChild(tr);
  }}
}}

function renderRunsTable() {{
  const tbody = document.querySelector("#runs-table tbody");
  tbody.innerHTML = "";
  const byTsCommit = new Map();
  for (const p of HISTORY.points) {{
    const key = p.ts_raw + "::" + p.commit + "::" + p.workload;
    if (!byTsCommit.has(key)) byTsCommit.set(key, {{ts_raw: p.ts_raw, ts: p.ts, commit: p.commit, commit_subject: p.commit_subject, workload: p.workload, runs: p.runs}});
    const entry = byTsCommit.get(key);
    entry[p.metric] = p.value;
  }}
  const sorted = [...byTsCommit.values()].sort((a, b) => b.ts_raw.localeCompare(a.ts_raw));
  for (const row of sorted.slice(0, 200)) {{
    const tr = document.createElement("tr");
    tr.innerHTML = `
      <td>${{shortTime(row.ts)}}</td>
      <td><code>${{shortCommit(row.commit)}}</code></td>
      <td>${{row.commit_subject || ""}}</td>
      <td><strong>${{row.workload}}</strong></td>
      <td>${{fmtRatio(row.wall_ratio)}}</td>
      <td>${{fmtRatio(row.rss_ratio)}}</td>
      <td>${{row.runs || 1}}</td>
    `;
    tbody.appendChild(tr);
  }}
}}

const YMAX_PRESETS = [
  {{label: "3×", value: 3}},
  {{label: "10×", value: 10}},
  {{label: "30×", value: 30}},
  {{label: "100×", value: 100}},
  {{label: "all", value: Infinity}},
];
const WINDOW_PRESETS = [
  {{label: "10", value: 10}},
  {{label: "25", value: 25}},
  {{label: "50", value: 50}},
  {{label: "all", value: Infinity}},
];

const TESTS_YMAX_PRESETS = [
  {{label: "50%",  value: 50}},
  {{label: "75%",  value: 75}},
  {{label: "90%",  value: 90}},
  {{label: "100%", value: 100}},
];

const chartState = {{
  wall:  {{yMaxIdx: 1, windowIdx: 2}},
  rss:   {{yMaxIdx: 1, windowIdx: 2}},
  tests: {{yMaxIdx: 3, windowIdx: 3}},
}};

function renderPills(containerId, presets, activeIdx, onPick) {{
  const el = document.getElementById(containerId);
  el.innerHTML = "";
  presets.forEach((preset, idx) => {{
    const btn = document.createElement("button");
    btn.className = "pill" + (idx === activeIdx ? " active" : "");
    btn.textContent = preset.label;
    btn.addEventListener("click", () => onPick(idx));
    el.appendChild(btn);
  }});
}}

function maxOfSeries(seriesIds) {{
  let m = 0;
  for (const id of seriesIds) {{
    for (const p of (HISTORY.series[id] || [])) {{
      if (Number.isFinite(p.value) && p.value > m) m = p.value;
    }}
  }}
  return m;
}}

function redrawWall() {{
  const ymax = YMAX_PRESETS[chartState.wall.yMaxIdx].value;
  const win = WINDOW_PRESETS[chartState.wall.windowIdx].value;
  const resolvedYMax = Number.isFinite(ymax) ? ymax : Math.max(maxOfSeries(HISTORY.wall_ids) * 1.05, 1.5);
  drawChart("wall-chart", "wall-legend", HISTORY.wall_ids, {{yMax: resolvedYMax, windowN: Number.isFinite(win) ? win : Infinity}});
  renderPills("wall-ymax-pills", YMAX_PRESETS, chartState.wall.yMaxIdx, idx => {{ chartState.wall.yMaxIdx = idx; redrawWall(); }});
  renderPills("wall-window-pills", WINDOW_PRESETS, chartState.wall.windowIdx, idx => {{ chartState.wall.windowIdx = idx; redrawWall(); }});
}}
function redrawRss() {{
  const ymax = YMAX_PRESETS[chartState.rss.yMaxIdx].value;
  const win = WINDOW_PRESETS[chartState.rss.windowIdx].value;
  const resolvedYMax = Number.isFinite(ymax) ? ymax : Math.max(maxOfSeries(HISTORY.rss_ids) * 1.05, 1.5);
  drawChart("rss-chart", "rss-legend", HISTORY.rss_ids, {{yMax: resolvedYMax, windowN: Number.isFinite(win) ? win : Infinity}});
  renderPills("rss-ymax-pills", YMAX_PRESETS, chartState.rss.yMaxIdx, idx => {{ chartState.rss.yMaxIdx = idx; redrawRss(); }});
  renderPills("rss-window-pills", WINDOW_PRESETS, chartState.rss.windowIdx, idx => {{ chartState.rss.windowIdx = idx; redrawRss(); }});
}}
function redrawTests() {{
  const ymax = TESTS_YMAX_PRESETS[chartState.tests.yMaxIdx].value;
  const win = WINDOW_PRESETS[chartState.tests.windowIdx].value;
  drawChart("tests-chart", "tests-legend", HISTORY.test_ids, {{
    yMax: ymax,
    windowN: Number.isFinite(win) ? win : Infinity,
    unit: "%",
    parityValue: 100,
    parityLabel: "all tests pass (100%)",
    gateValue: null,
  }});
  renderPills("tests-ymax-pills", TESTS_YMAX_PRESETS, chartState.tests.yMaxIdx, idx => {{ chartState.tests.yMaxIdx = idx; redrawTests(); }});
  renderPills("tests-window-pills", WINDOW_PRESETS, chartState.tests.windowIdx, idx => {{ chartState.tests.windowIdx = idx; redrawTests(); }});
}}

redrawWall();
redrawRss();
renderLatestTable();
renderRunsTable();

async function checkForRefresh() {{
  const status = document.getElementById("refresh-status");
  try {{
    const response = await fetch(`history.json?check=${{Date.now()}}`, {{cache: "no-store"}});
    const next = await response.json();
    const nextSignature = JSON.stringify(next.signature || {{}});
    if (nextSignature !== INITIAL_SIGNATURE) {{
      status.textContent = "New data found; reloading...";
      window.location.reload();
      return;
    }}
    status.textContent = "Auto-refresh checked " + new Date().toLocaleTimeString();
  }} catch (err) {{
    status.textContent = "Auto-refresh check failed";
  }}
}}
setInterval(checkForRefresh, 30000);
</script>
</body>
</html>
"""


def build(out_dir: Path, *, quiet: bool = False) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    history = build_history()
    (out_dir / "history.json").write_text(
        json.dumps({"signature": history["signature"]}, indent=2) + "\n",
        encoding="utf-8",
    )
    (out_dir / "index.html").write_text(render_html(history), encoding="utf-8")
    if not quiet:
        print(f"wrote {out_dir / 'index.html'}")
        print(f"points: {history['point_count']} over {history['commit_count']} commits")


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None


def needs_rebuild(out_dir: Path) -> bool:
    index = out_dir / "index.html"
    history_json = out_dir / "history.json"
    if not index.exists() or not history_json.exists():
        return True
    existing = load_json(history_json) or {}
    existing_signature = existing.get("signature")
    current_signature = build_history().get("signature")
    return existing_signature != current_signature


def serve(out_dir: Path, port: int) -> None:
    class Handler(SimpleHTTPRequestHandler):
        def do_GET(self) -> None:
            route = self.path.split("?", 1)[0]
            if route in {"/", "/index.html", "/history.json"} and needs_rebuild(out_dir):
                try:
                    build(out_dir, quiet=True)
                except Exception as err:
                    print(f"dashboard rebuild failed: {err}")
            super().do_GET()

        def __init__(self, *args: Any, **kwargs: Any) -> None:
            super().__init__(*args, directory=str(out_dir), **kwargs)

    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    print(f"serving http://127.0.0.1:{port}/")
    server.serve_forever()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--serve", action="store_true")
    parser.add_argument("--port", type=int, default=8055)
    args = parser.parse_args()
    if args.serve:
        if needs_rebuild(args.out_dir):
            build(args.out_dir)
        serve(args.out_dir, args.port)
    else:
        build(args.out_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
