#!/usr/bin/env python3
"""Summarize lua-rs GC profile snapshots.

`gc-profile.sh` records a start snapshot after CLI/library startup and an end
snapshot after the process finishes. This helper emits a numeric delta table and
rate table so GC profiles answer cadence questions without hand arithmetic.
"""

from __future__ import annotations

import argparse
import math
from pathlib import Path
from typing import Optional


RATE_METRICS = (
    "collections",
    "minor_collections",
    "full_collections",
)

GAUGE_RATE_METRICS = (
    "interned_short_strings",
)


def read_tsv(path: Path) -> dict[str, str]:
    rows: dict[str, str] = {}
    with path.open("r", encoding="utf-8") as fh:
        header = fh.readline().rstrip("\n").split("\t")
        if header != ["metric", "value"]:
            raise SystemExit(f"{path}: expected 'metric\\tvalue' header")
        for line in fh:
            line = line.rstrip("\n")
            if not line:
                continue
            try:
                metric, value = line.split("\t", 1)
            except ValueError as exc:
                raise SystemExit(f"{path}: malformed row: {line!r}") from exc
            rows[metric] = value
    return rows


def as_int(value: str) -> Optional[int]:
    try:
        return int(value)
    except ValueError:
        return None


def fmt_float(value: float) -> str:
    if math.isfinite(value):
        return f"{value:.6f}".rstrip("0").rstrip(".")
    return "nan"


def write_delta(path: Path, start: dict[str, str], end: dict[str, str]) -> dict[str, int]:
    deltas: dict[str, int] = {}
    with path.open("w", encoding="utf-8") as fh:
        fh.write("metric\tstart\tend\tdelta\n")
        for metric in end:
            start_i = as_int(start.get(metric, ""))
            end_i = as_int(end[metric])
            if start_i is None or end_i is None:
                continue
            delta = end_i - start_i
            deltas[metric] = delta
            fh.write(f"{metric}\t{start_i}\t{end_i}\t{delta}\n")
    return deltas


def write_rates(
    path: Path,
    deltas: dict[str, int],
    end: dict[str, str],
    elapsed_seconds: float,
    repeat: int,
) -> None:
    seconds = elapsed_seconds if elapsed_seconds > 0 else float("nan")
    with path.open("w", encoding="utf-8") as fh:
        fh.write("metric\tvalue\n")
        fh.write(f"elapsed_seconds\t{fmt_float(elapsed_seconds)}\n")
        fh.write(f"profile_repeat\t{repeat}\n")
        for metric in RATE_METRICS:
            delta = deltas.get(metric)
            if delta is None:
                continue
            fh.write(f"{metric}_delta\t{delta}\n")
            fh.write(f"{metric}_per_run\t{fmt_float(delta / repeat)}\n")
            fh.write(f"{metric}_per_second\t{fmt_float(delta / seconds)}\n")
        for metric in GAUGE_RATE_METRICS:
            delta = deltas.get(metric)
            end_i = as_int(end.get(metric, ""))
            if delta is None or end_i is None:
                continue
            fh.write(f"{metric}_end\t{end_i}\n")
            fh.write(f"{metric}_net_delta\t{delta}\n")
            fh.write(f"{metric}_net_delta_per_run\t{fmt_float(delta / repeat)}\n")
            fh.write(f"{metric}_net_delta_per_second\t{fmt_float(delta / seconds)}\n")

        collections = deltas.get("collections", 0)
        for metric in (
            "marked",
            "traced",
            "sweep_visited",
            "sweep_freed",
            "sweep_freed_bytes",
        ):
            end_i = as_int(end.get(metric, ""))
            if end_i is None:
                continue
            fh.write(f"last_cycle_{metric}\t{end_i}\n")
            if collections > 0:
                fh.write(
                    f"last_cycle_{metric}_per_collection\t"
                    f"{fmt_float(end_i / collections)}\n"
                )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--start", required=True, type=Path)
    parser.add_argument("--end", required=True, type=Path)
    parser.add_argument("--delta-out", required=True, type=Path)
    parser.add_argument("--rates-out", required=True, type=Path)
    parser.add_argument("--elapsed-seconds", required=True, type=float)
    parser.add_argument("--repeat", required=True, type=int)
    args = parser.parse_args()

    if args.repeat < 1:
        raise SystemExit("--repeat must be >= 1")

    start = read_tsv(args.start)
    end = read_tsv(args.end)
    deltas = write_delta(args.delta_out, start, end)
    write_rates(args.rates_out, deltas, end, args.elapsed_seconds, args.repeat)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
