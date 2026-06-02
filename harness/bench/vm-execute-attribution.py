#!/usr/bin/env python3
"""Bucket macOS `sample` call-graph lines inside lua_vm::vm::execute.

`profile-hotspots.sh` already captures raw `/usr/bin/sample` output, but its
leaf-frame summary collapses most interpreter-heavy workloads into one
`lua_vm::vm::execute` symbol. The raw call graph has source lines and offsets;
this helper turns those lines into dispatch/opcode-region buckets.

The primary metric is "self" samples: a call-graph node's inclusive count minus
its immediate child counts. That avoids charging a suspended outer `execute`
frame at `OP_CALL` for time spent in a nested callee's active VM frame.
"""

from __future__ import annotations

import argparse
import dataclasses
import pathlib
import re
from bisect import bisect_left
from collections import defaultdict


EXECUTE_RE = re.compile(
    r"(?P<prefix>.*?)(?P<count>\d+)\s+"
    r"(?P<frame>lua_vm::vm::execute\S*)\s+\(in\s+(?P<lib>[^)]+)\)"
    r".*?(?P<file>[A-Za-z0-9_./-]+):(?P<line>\d+)"
)

ANY_FRAME_RE = re.compile(
    r"(?P<prefix>.*?)(?P<count>\d+)\s+"
    r"(?P<frame>.+?)\s+\(in\s+(?P<lib>[^)]+)\)"
)

OFFSET_RE = re.compile(r"\(in\s+[^)]+\)\s+\+\s+(?P<offsets>[0-9,.\s]+?)\s+\[[^\]]+\]")

OP_RE = re.compile(r"\bOpCode::([A-Za-z0-9_]+)\s*=>")


@dataclasses.dataclass
class Node:
    indent: int
    count: int
    frame: str
    file: str | None = None
    line: int | None = None
    offsets_text: str = ""
    offsets: tuple[int, ...] = ()
    offsets_truncated: bool = False
    children: list["Node"] = dataclasses.field(default_factory=list)

    @property
    def self_count(self) -> int:
        child_total = sum(child.count for child in self.children)
        return max(0, self.count - child_total)


@dataclasses.dataclass(frozen=True)
class Region:
    start: int
    end: int
    name: str


@dataclasses.dataclass(frozen=True)
class KnownOffset:
    offset: int
    file_name: str
    line: int
    region: str


def opcode_name(variant: str) -> str:
    return "OP_" + re.sub(r"[^A-Za-z0-9]", "", variant).upper()


def source_regions(source: pathlib.Path) -> tuple[list[Region], dict[int, str]]:
    lines = source.read_text(errors="replace").splitlines()
    execute_start = 1
    dispatch_start = 1
    match_start = 1
    match_end = len(lines)
    execute_end = len(lines)
    all_op_starts: list[tuple[int, str]] = []
    source_text: dict[int, str] = {}

    for idx, text in enumerate(lines, start=1):
        source_text[idx] = text.strip()
        if "fn execute(" in text:
            execute_start = idx
        elif "'dispatch: loop" in text:
            dispatch_start = idx
        elif re.search(r"^\s*match\s+op\s*\{", text):
            match_start = idx
        elif "} // end match opcode" in text:
            match_end = idx
        elif "} // end 'startfunc loop" in text:
            execute_end = idx

        match = OP_RE.search(text)
        if match:
            start = idx
            while start > 1:
                prev = lines[start - 2].strip()
                if prev == "" or prev.startswith("//"):
                    start -= 1
                else:
                    break
            all_op_starts.append((start, opcode_name(match.group(1))))

    op_starts = [
        (idx, name)
        for idx, name in all_op_starts
        if match_start <= idx <= match_end
    ]

    regions: list[Region] = [
        Region(execute_start, dispatch_start - 1, "FRAME_SETUP"),
        Region(dispatch_start, match_start - 1, "DISPATCH_FETCH"),
    ]

    for pos, (start, name) in enumerate(op_starts):
        next_start = op_starts[pos + 1][0] if pos + 1 < len(op_starts) else match_end
        regions.append(Region(start, next_start - 1, name))

    if match_end + 1 <= execute_end:
        regions.append(Region(match_end + 1, execute_end, "RETURN_REENTRY"))

    return regions, source_text


def region_for_line(regions: list[Region], file_name: str | None, line: int | None) -> str:
    if file_name is None or line is None:
        return "UNKNOWN"
    if line == 0:
        return "UNKNOWN_INLINED"
    if pathlib.PurePath(file_name).name != "vm.rs":
        return f"INLINED_{pathlib.PurePath(file_name).name}"
    for region in regions:
        if region.start <= line <= region.end:
            return region.name
    return "OUTSIDE_EXECUTE"


def is_opaque_region(region: str) -> bool:
    return (
        region == "UNKNOWN"
        or region == "UNKNOWN_INLINED"
        or region == "OUTSIDE_EXECUTE"
        or region.startswith("INLINED_")
    )


def parse_offsets(offsets: str) -> tuple[str, tuple[int, ...], bool]:
    compact = re.sub(r"\s+", "", offsets)
    values = tuple(int(match.group(0)) for match in re.finditer(r"\d+", compact))
    return compact, values, "..." in compact


def compact_offsets(offsets: str, limit: int = 64) -> str:
    if not offsets:
        return ""
    compact = re.sub(r"\s+", "", offsets)
    if len(compact) <= limit:
        return compact
    return compact[: limit - 3] + "..."


def nearest_known_offsets(
    known_offsets: list[KnownOffset],
    offset: int,
    limit: int = 3,
) -> list[KnownOffset]:
    if not known_offsets:
        return []
    values = [known.offset for known in known_offsets]
    pos = bisect_left(values, offset)
    candidates: list[KnownOffset] = []
    seen: set[tuple[int, str, int]] = set()
    for idx in range(max(0, pos - 3), min(len(known_offsets), pos + 4)):
        known = known_offsets[idx]
        key = (known.offset, known.file_name, known.line)
        if key not in seen:
            candidates.append(known)
            seen.add(key)
    candidates.sort(key=lambda known: (abs(known.offset - offset), known.offset))
    return candidates[:limit]


def format_offset_neighbors(known_offsets: list[KnownOffset], offset: int) -> str:
    neighbors = nearest_known_offsets(known_offsets, offset)
    if not neighbors:
        return ""
    parts = []
    for known in neighbors:
        delta = known.offset - offset
        parts.append(
            f"{known.offset}:{known.file_name}:{known.line}/"
            f"{known.region}({delta:+d})"
        )
    return ", ".join(parts)


def parse_call_graph(sample_text: str) -> list[Node]:
    in_call_graph = False
    roots: list[Node] = []
    stack: list[Node] = []

    for raw_line in sample_text.splitlines():
        if raw_line.startswith("Call graph:"):
            in_call_graph = True
            continue
        if not in_call_graph:
            continue
        if raw_line.startswith("Total number in stack"):
            break
        if not raw_line.strip():
            continue

        exec_match = EXECUTE_RE.match(raw_line)
        any_match = ANY_FRAME_RE.match(raw_line)
        if not any_match:
            continue

        prefix = any_match.group("prefix")
        indent = len(prefix)
        count = int(any_match.group("count"))
        frame = any_match.group("frame").strip()
        node = Node(indent=indent, count=count, frame=frame)

        if exec_match:
            node.file = exec_match.group("file")
            node.line = int(exec_match.group("line"))
            offset_match = OFFSET_RE.search(raw_line)
            if offset_match:
                raw_offsets, offsets, truncated = parse_offsets(offset_match.group("offsets"))
                node.offsets_text = compact_offsets(raw_offsets)
                node.offsets = offsets
                node.offsets_truncated = truncated

        while stack and indent <= stack[-1].indent:
            stack.pop()
        if stack:
            stack[-1].children.append(node)
        else:
            roots.append(node)
        stack.append(node)

    return roots


def walk(nodes: list[Node]):
    for node in nodes:
        yield node
        yield from walk(node.children)


def render(sample: pathlib.Path, source: pathlib.Path) -> str:
    sample_text = sample.read_text(errors="replace")
    regions, source_text = source_regions(source)
    roots = parse_call_graph(sample_text)

    region_self: dict[str, int] = defaultdict(int)
    line_self: dict[tuple[str, int, str], int] = defaultdict(int)
    opaque_self: dict[tuple[str, int, str, str, tuple[int, ...], bool], int] = defaultdict(int)
    region_inclusive: dict[str, int] = defaultdict(int)
    total_thread_samples = sum(root.count for root in roots) or 1
    known_offsets: list[KnownOffset] = []

    execute_nodes = 0
    execute_nodes_with_source = 0
    for node in walk(roots):
        if not node.frame.startswith("lua_vm::vm::execute"):
            continue
        execute_nodes += 1
        if node.file is not None and node.line is not None:
            execute_nodes_with_source += 1
        region = region_for_line(regions, node.file, node.line)
        region_self[region] += node.self_count
        region_inclusive[region] += node.count
        if node.self_count:
            file_name = pathlib.PurePath(node.file or "?").name
            line_self[(file_name, node.line or 0, region)] += node.self_count
            if is_opaque_region(region):
                opaque_self[
                    (
                        file_name,
                        node.line or 0,
                        region,
                        node.offsets_text,
                        node.offsets,
                        node.offsets_truncated,
                    )
                ] += node.self_count
            elif node.offsets and file_name == "vm.rs" and node.line:
                for offset in node.offsets:
                    known_offsets.append(
                        KnownOffset(
                            offset=offset,
                            file_name=file_name,
                            line=node.line,
                            region=region,
                        )
                    )

    known_offsets.sort(key=lambda known: known.offset)

    total_execute_self = sum(region_self.values()) or 1
    total_opaque_self = sum(opaque_self.values())
    opaque_rows_with_truncated_offsets = sum(1 for key in opaque_self if key[5])
    lines: list[str] = []
    lines.append(f"sample:                {sample}")
    lines.append(f"source:                {source}")
    lines.append(f"thread_samples:        {total_thread_samples}")
    lines.append(f"execute_nodes:         {execute_nodes}")
    lines.append(f"execute_source_nodes:  {execute_nodes_with_source}")
    lines.append(f"execute_self_samples:  {sum(region_self.values())}")
    lines.append(f"opaque_self_samples:   {total_opaque_self}")
    lines.append(f"opaque_offset_rows_with_ellipsis: {opaque_rows_with_truncated_offsets}")
    if execute_nodes > 0 and execute_nodes_with_source == 0:
        lines.append(
            "warning: no source-line data found for lua_vm::vm::execute; "
            "rebuild with CARGO_PROFILE_RELEASE_DEBUG=true and "
            'RUSTFLAGS="-C force-frame-pointers=yes" before profiling'
        )
    lines.append("")
    lines.append("VM execute self samples by source/opcode region:")
    lines.append(f"  {'count':>8}  {'vm_pct':>6}  {'thread_pct':>10}  region")
    for region, count in sorted(region_self.items(), key=lambda row: (-row[1], row[0])):
        vm_pct = 100.0 * count / total_execute_self
        thread_pct = 100.0 * count / total_thread_samples
        lines.append(f"  {count:>8}  {vm_pct:>5.1f}%  {thread_pct:>9.1f}%  {region}")

    if opaque_self:
        lines.append("")
        lines.append("Opaque VM execute self samples by source file:")
        lines.append(f"  {'count':>8}  {'vm_pct':>6}  {'thread_pct':>10}  location  region  offsets")
        for (file_name, line_no, region, offsets_text, _, _), count in sorted(
            opaque_self.items(), key=lambda row: (-row[1], row[0][0], row[0][1])
        )[:20]:
            vm_pct = 100.0 * count / total_execute_self
            thread_pct = 100.0 * count / total_thread_samples
            lines.append(
                f"  {count:>8}  {vm_pct:>5.1f}%  {thread_pct:>9.1f}%  "
                f"{file_name}:{line_no:<5}  {region:<18}  {offsets_text}"
            )

        offset_rows: list[tuple[int, str, int, str, int, str, bool]] = []
        for (file_name, line_no, region, _, offsets, truncated), count in opaque_self.items():
            for offset in offsets[:6]:
                offset_rows.append(
                    (
                        count,
                        file_name,
                        line_no,
                        region,
                        offset,
                        format_offset_neighbors(known_offsets, offset),
                        truncated,
                    )
                )
        if offset_rows:
            lines.append("")
            lines.append(
                "Visible opaque VM offset neighbors "
                "(row_count is the aggregate row count, not per-offset samples):"
            )
            lines.append(
                f"  {'row_count':>9}  location  region  "
                f"{'offset':>8}  {'ellipsis':>8}  nearest_known_offsets"
            )
            for count, file_name, line_no, region, offset, neighbors, truncated in sorted(
                offset_rows, key=lambda row: (-row[0], row[1], row[2], row[4])
            )[:30]:
                ellipsis = "yes" if truncated else "no"
                lines.append(
                    f"  {count:>9}  {file_name}:{line_no:<5}  {region:<18}  "
                    f"{offset:>8}  {ellipsis:>8}  {neighbors}"
                )

    lines.append("")
    lines.append("Top VM execute self samples by source line:")
    lines.append(f"  {'count':>8}  {'vm_pct':>6}  location  region  source")
    for (file_name, line_no, region), count in sorted(
        line_self.items(), key=lambda row: (-row[1], row[0][0], row[0][1])
    )[:40]:
        vm_pct = 100.0 * count / total_execute_self
        snippet = source_text.get(line_no, "") if file_name == "vm.rs" else ""
        lines.append(
            f"  {count:>8}  {vm_pct:>5.1f}%  {file_name}:{line_no:<5}  {region:<18}  {snippet}"
        )

    lines.append("")
    lines.append("VM execute inclusive context by region:")
    lines.append(f"  {'count':>8}  region")
    for region, count in sorted(region_inclusive.items(), key=lambda row: (-row[1], row[0])):
        lines.append(f"  {count:>8}  {region}")

    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("sample", type=pathlib.Path)
    parser.add_argument(
        "--source",
        type=pathlib.Path,
        default=pathlib.Path("crates/lua-vm/src/vm.rs"),
        help="current vm.rs source used to map line numbers to opcode regions",
    )
    parser.add_argument("-o", "--output", type=pathlib.Path)
    args = parser.parse_args()

    report = render(args.sample, args.source)
    if args.output:
        args.output.write_text(report)
    else:
        print(report, end="")


if __name__ == "__main__":
    main()
