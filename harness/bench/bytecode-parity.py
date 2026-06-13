#!/usr/bin/env python3
"""Diff lua-rs emitted bytecode against luac -l -l (PERF_PUSH_SPEC W2.4/P2.3).

Compares the flattened mnemonic stream (all functions, listing order) so the
gate is robust to function-header formatting. Divergences are reported with
context; known accepted divergences live in bytecode-parity-allow.txt as
`<workload>:<n>` counts.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LUAC = ROOT / "reference/lua-5.4.7/src/luac"
LUARS = ROOT / "target/release/omnilua"
ALLOW = Path(__file__).parent / "bytecode-parity-allow.txt"

LUAC_RE = re.compile(r"^\t\d+\t\[\d+\]\t([A-Z0-9]+)")


def luac_stream(path: Path) -> list[str]:
    out = subprocess.run([str(LUAC), "-l", "-l", str(path)],
                         capture_output=True, text=True, check=True).stdout
    return [m.group(1) for line in out.splitlines() if (m := LUAC_RE.match(line))]


def luars_stream(path: Path) -> list[str]:
    out = subprocess.run([str(LUARS), str(path)], capture_output=True, text=True,
                         env={"LUA_RS_LIST_BYTECODE": "1", "PATH": "/usr/bin:/bin"},
                         check=True).stdout
    return [l.rstrip("_") for l in out.splitlines()
            if l and not l.startswith("function")]


def diff_count(a: list[str], b: list[str]) -> tuple[int, str]:
    import difflib
    sm = difflib.SequenceMatcher(a=a, b=b, autojunk=False)
    bad = sum(max(i2 - i1, j2 - j1) for tag, i1, i2, j1, j2 in sm.get_opcodes()
              if tag != "equal")
    detail = ""
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag != "equal" and not detail:
            detail = f"first divergence @C[{i1}]: C={a[i1:i2][:4]} rs={b[j1:j2][:4]}"
    return bad, detail


def load_allow() -> dict[str, int]:
    allow = {}
    if ALLOW.exists():
        for line in ALLOW.read_text().splitlines():
            line = line.split("#")[0].strip()
            if ":" in line:
                k, v = line.rsplit(":", 1)
                allow[k] = int(v)
    return allow


def main() -> int:
    targets = sys.argv[1:] or sorted(
        str(p) for p in (ROOT / "harness/bench/workloads").glob("*.lua"))
    allow = load_allow()
    failed = 0
    for t in targets:
        p = Path(t)
        name = p.stem
        try:
            c, r = luac_stream(p), luars_stream(p)
        except subprocess.CalledProcessError as e:
            print(f"FAIL {name}: listing failed ({e})")
            failed += 1
            continue
        bad, detail = diff_count(c, r)
        budget = allow.get(name, 0)
        status = "OK" if bad <= budget else "FAIL"
        extra = f" ({bad} divergent ops, allow {budget})" if bad else ""
        print(f"{status:<5}{name}{extra}")
        if bad and detail and status == "FAIL":
            print(f"      {detail}")
        if status == "FAIL":
            failed += 1
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
