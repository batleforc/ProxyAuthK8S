#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///
"""Compare two `task perf:http` reports (RFC 0016 §6.3): the p95 of every
scenario summary (`NN_name.json`, k6's --summary-export) of NEW against BASE.

A scenario slower than BASE by more than MARGIN percent is a regression; one
missing from either side is reported, never silently skipped. `--fail` exits
1 on a regression. The output is a markdown table for the run summary.

`--self-test` runs the comparison on fixtures.
"""

import json
import re
import sys
import tempfile
from pathlib import Path

SCENARIO = re.compile(r"\d{2}_\w+\.json")


def p95s(directory: Path) -> dict[str, float]:
    found = {}
    for path in sorted(directory.glob("*.json")):
        if not SCENARIO.fullmatch(path.name):
            continue
        metrics = json.loads(path.read_text()).get("metrics", {})
        value = metrics.get("http_req_duration", {}).get("p(95)")
        if value is not None:
            found[path.stem] = float(value)
    return found


def compare(base: Path, new: Path, margin: float) -> tuple[list[str], int]:
    old, now = p95s(base), p95s(new)
    lines = [
        f"| Scenario | p95 base (ms) | p95 new (ms) | change | verdict (margin {margin:g} %) |",
        "| --- | --- | --- | --- | --- |",
    ]
    regressions = 0
    for name in sorted(old.keys() | now.keys()):
        if name not in now or name not in old:
            side = "new" if name not in now else "base"
            lines.append(f"| {name} | {old.get(name, '-')} | {now.get(name, '-')} | - | missing from {side} |")
            regressions += name not in now
            continue
        change = (now[name] - old[name]) / old[name] * 100 if old[name] else 0.0
        verdict = "regression" if change > margin else "ok"
        regressions += verdict == "regression"
        lines.append(f"| {name} | {old[name]:.1f} | {now[name]:.1f} | {change:+.1f} % | {verdict} |")
    return lines, regressions


def self_test() -> None:
    def write(directory: Path, values: dict[str, float]) -> None:
        directory.mkdir()
        for name, p95 in values.items():
            (directory / f"{name}.json").write_text(json.dumps({"metrics": {"http_req_duration": {"p(95)": p95}}}))
        (directory / "resources.json").write_text("{}")

    with tempfile.TemporaryDirectory() as tmp:
        base, new = Path(tmp, "base"), Path(tmp, "new")
        write(base, {"01_a": 100.0, "02_b": 100.0, "03_c": 50.0})
        write(new, {"01_a": 120.0, "02_b": 130.0, "04_d": 10.0})
        lines, regressions = compare(base, new, 25)
        text = "\n".join(lines)
        assert "| 01_a | 100.0 | 120.0 | +20.0 % | ok |" in text, text
        assert "| 02_b | 100.0 | 130.0 | +30.0 % | regression |" in text, text
        assert "03_c" in text and "missing from new" in text, text
        assert "04_d" in text and "missing from base" in text, text
        assert "resources" not in text, text
        # 02_b slower, 03_c gone: two regressions; a new scenario is none.
        assert regressions == 2, regressions
    print("perf_compare self-test: ok")


def main(argv: list[str]) -> int:
    if argv[1:] == ["--self-test"]:
        self_test()
        return 0
    args = [a for a in argv[1:] if not a.startswith("--")]
    margin = 25.0
    if "--margin" in argv:
        margin = float(argv[argv.index("--margin") + 1])
        args.remove(argv[argv.index("--margin") + 1])
    if len(args) != 2:
        print("usage: perf_compare.py BASE NEW [--margin N] [--fail] | --self-test", file=sys.stderr)
        return 2
    base, new = Path(args[0]), Path(args[1])
    for directory in (base, new):
        if not p95s(directory):
            print(f"perf_compare: no scenario summary in {directory}", file=sys.stderr)
            return 2
    lines, regressions = compare(base, new, margin)
    print("\n".join(lines))
    return 1 if regressions and "--fail" in argv else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
