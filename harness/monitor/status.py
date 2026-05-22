#!/usr/bin/env python3
"""One-shot text snapshot of Phase A state. No curses, just prints and exits.

Usage:
    ./harness/monitor/status.py
"""

from __future__ import annotations

import sys
from pathlib import Path

# Allow running from anywhere
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from backend import LiveBackend  # noqa: E402


GLYPH = {"wait": "·", "work": "▶", "done": "✓", "fail": "✗", "skip": "−"}


def main() -> None:
    root = HERE.parent.parent
    b = LiveBackend(root)
    fs = b.files()
    s = b.summary()

    elapsed = f"{s.elapsed_s // 60}m{s.elapsed_s % 60:02d}s"
    print(
        f"Elapsed (from latest write): {elapsed}   "
        f"Spent: ${s.total_cost:.2f}"
    )
    print(
        f"Status:  {s.done_count} done · {s.fail_count} fail · "
        f"{s.work_count} work · {s.wait_count} wait · {s.skip_count} skip"
    )
    print()
    print(
        f"  ST   {'FILE':<17} {'TARGET':<28} "
        f"{'COST':>7} {'DUR':>7}  HK SX"
    )
    print("  " + "─" * 80)
    for f in fs:
        g = GLYPH.get(f.status, "?")
        cost = f"${f.cost_usd:.2f}" if f.cost_usd else "    —"
        dur = (
            f"{f.duration_s // 60:>3}:{f.duration_s % 60:02d}"
            if f.duration_s
            else "    —"
        )
        target = f.target.replace("crates/", "").replace("/src/", "/")
        hk = " " if f.hooks_pass is None else ("✓" if f.hooks_pass else "✗")
        sx = " " if f.syntax_ok is None else ("✓" if f.syntax_ok else "✗")
        print(
            f"  {g}   {f.cfile:<17} {target[:28]:<28} "
            f"{cost:>7} {dur:>7}   {hk}  {sx}"
        )
        if f.last_event and f.status in ("work", "fail"):
            ev = f.last_event[:88]
            print(f"        ⤷ {ev}")


if __name__ == "__main__":
    main()
