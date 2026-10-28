#!/usr/bin/env python3
"""The world update's schedule in `docs/ENGINE_SYSTEMS.md` (FD-045-D13), generated from
genie-core's `world/schedule.rs`:

    scripts/schedule.py SRC [--out FILE] [--check]

The section between `<!-- schedule -->` and `<!-- /schedule -->` lists the update's steps in
their order: the steps before the players' (`BEFORE_PLAYERS`), each player's steps
(`EACH_PLAYER`) and the steps after them (`AFTER_PLAYERS`), each with its variant's doc. A
variant the arrays leave out, or hold twice, is an error. With `--check` it fails when the
section is not current.
"""

import re
import sys
from pathlib import Path

import readmes

MARKS = ("<!-- schedule -->", "<!-- /schedule -->")
ARRAY = re.compile(
    r"^pub const (BEFORE_PLAYERS|EACH_PLAYER|AFTER_PLAYERS): \[(WorldStep|PlayerStep); \d+\] = \[(.*?)\];",
    re.S | re.M,
)
VARIANT = re.compile(r"^([A-Z][A-Za-z0-9]*),$")
RUNS = {"WorldStep": "once", "PlayerStep": "for each player"}


def variants(text: str, name: str) -> dict[str, str]:
    """Each variant of enum `name`, in order, with its doc on one line."""
    lines = text.split("\n")
    start = lines.index(f"pub enum {name} {{")
    out: dict[str, str] = {}
    doc: list[str] = []
    for line in lines[start + 1 :]:
        s = line.strip()
        if s == "}":
            break
        if s.startswith("///"):
            doc.append(s[3:].strip())
        elif m := VARIANT.match(s):
            out[m.group(1)] = " ".join(doc)
            doc = []
    return out


def arrays(text: str) -> list[tuple[str, str, list[str]]]:
    """The schedule's arrays in the file's order: name, the steps' enum, the variants."""
    return [
        (m.group(1), m.group(2), re.findall(rf"{m.group(2)}::([A-Za-z0-9]+)", m.group(3)))
        for m in ARRAY.finditer(text)
    ]


def plain(doc: str) -> str:
    """A doc as table text: rustdoc's intra-doc links as plain text, `|` escaped."""
    doc = re.sub(r"\[(`[^`]*`)\](?!\()", r"\1", doc)
    doc = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", doc)
    return doc.replace("|", "\\|")


def problems_of(text: str) -> list[str]:
    """Variants the arrays leave out or hold twice, steps that name no variant, a missing array."""
    out = []
    found = arrays(text)
    names = [name for name, _, _ in found]
    for name in ("BEFORE_PLAYERS", "EACH_PLAYER", "AFTER_PLAYERS"):
        if name not in names:
            out.append(f"no array {name}")
    for kind in RUNS:
        declared = variants(text, kind)
        scheduled = [s for _, k, steps in found if k == kind for s in steps]
        out += [f"{kind}::{v} is not scheduled" for v in declared if v not in scheduled]
        out += [f"{kind}::{v} is scheduled twice" for v in declared if scheduled.count(v) > 1]
        out += [f"{kind}::{v} is no variant" for v in scheduled if v not in declared]
    return out


def table(text: str) -> str:
    """The steps in their order: number, name, how often an update runs it, its doc."""
    docs = {kind: variants(text, kind) for kind in RUNS}
    rows = ["| # | Step | Runs | What it does |", "| ---: | --- | --- | --- |"]
    for _, kind, steps in arrays(text):
        for step in steps:
            rows.append(f"| {len(rows) - 1} | `{step}` | {RUNS[kind]} | {plain(docs[kind][step])} |")
    return "\n".join(rows)


def main(argv: list[str]) -> int:
    args = [a for a in argv if not a.startswith("--")]
    check = "--check" in argv
    out = Path(argv[argv.index("--out") + 1]) if "--out" in argv else None
    if out is not None:
        args.remove(str(out))
    if len(args) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    source = Path(args[0]) / "world" / "schedule.rs"
    out = out or Path("docs/ENGINE_SYSTEMS.md")
    text = source.read_text()
    problems = problems_of(text)
    for p in problems:
        print(f"schedule: {source}: {p}", file=sys.stderr)
    if problems:
        return 1
    doc = out.read_text()
    new = readmes.replace_section(doc, MARKS, table(text))
    if new is None:
        print(f"schedule: {out}: no {MARKS[0]} … {MARKS[1]} section", file=sys.stderr)
        return 1
    if check:
        if new != doc:
            print(f"schedule: {out}: not current with {source}; run `just schedule`", file=sys.stderr)
            return 1
        return 0
    out.write_text(new)
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
