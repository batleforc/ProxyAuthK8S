#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///
"""Check tests/integration/MATRIX.md against the RFCs (RFC 0009 §6.5).

Every numbered point of a `### 6.P` section of `.rfc/NNNN-*.md` needs a row;
a `suite` row must name tests that exist; `hors suite` needs a reason or a
pointer; `quarantaine` needs an issue number; `à écrire` is refused except
for the RFCs whose code is not written yet (PENDING). The suite crate must
not depend on the workspace by path (§5.3).

Tests are found by reading the sources, not by building them, so this runs in
`task lint` in a second. shortcut: a test name in a comment or a string
counts as existing; switch to `cargo nextest list` and `playwright --list` if
that ever lets a dead reference through.

`--self-test` runs the checks on fixtures (RFC 0009 §6.P points 5 and 6).
"""

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MATRIX = ROOT / "tests/integration/MATRIX.md"
# RFCs whose rows may still say `à écrire`: 0010, whose code is not written
# yet; each leaves this set in the PR that lands its code and tests.
PENDING = {"0010", "0014"}
STATUSES = {"suite", "hors suite", "quarantaine", "à écrire"}


def rfc_points(text: str) -> list[int]:
    """The point numbers of the §6.P section of one RFC."""
    points, inside = [], False
    for line in text.splitlines():
        if line.startswith("### 6.P"):
            inside = True
            continue
        if inside and (line.startswith("##") or line.startswith("**Deliberately")):
            break
        if inside and (m := re.match(r"(\d+)\. ", line)):
            points.append(int(m.group(1)))
    return points


def matrix_rows(text: str) -> list[tuple[int, str, int, str, str]]:
    """(line number, rfc, point, status, proof) for every data row."""
    rows = []
    for number, line in enumerate(text.splitlines(), 1):
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) != 4 or not re.fullmatch(r"\d{4}", cells[0]):
            continue
        rfc, point, status, proof = cells
        rows.append((number, rfc, int(point) if point.isdigit() else -1, status, proof))
    return rows


def rust_tests(root: Path) -> set[str]:
    """`rfcNNNN::pN_name` for every test function of tests/integration/tests."""
    found = set()
    for path in (root / "tests/integration/tests").glob("*.rs"):
        for name in re.findall(r"fn (p\d+_\w+)\s*\(", path.read_text()):
            found.add(f"{path.stem}::{name}")
    return found


def playwright_tests(root: Path) -> set[str]:
    """`e2e-it/<file> › <title>` for every test of apps/front/e2e-it."""
    found = set()
    for path in (root / "apps/front/e2e-it").glob("*.spec.ts"):
        for title in re.findall(r"""\btest\(\s*(['"`])(.+?)\1""", path.read_text()):
            found.add(f"e2e-it/{path.name} › {title[1]}")
    return found


def path_dependencies(root: Path) -> list[str]:
    manifest = tomllib.loads((root / "tests/integration/Cargo.toml").read_text())
    tables = [manifest.get(k, {}) for k in ("dependencies", "dev-dependencies", "build-dependencies")]
    return [name for table in tables for name, spec in table.items()
            if isinstance(spec, dict) and "path" in spec]


def check(rfcs: dict[str, list[int]], matrix: str, tests: set[str],
          path_deps: list[str]) -> tuple[list[str], list[str]]:
    """(errors, warnings)."""
    errors, warnings = [], []
    rows = matrix_rows(matrix)
    covered = {(rfc, point) for _, rfc, point, _, _ in rows}
    for rfc, points in sorted(rfcs.items()):
        for point in points:
            if (rfc, point) not in covered:
                errors.append(f"RFC {rfc} §6.P point {point} has no row in MATRIX.md")
    quarantined = 0
    for line, rfc, point, status, proof in rows:
        where = f"MATRIX.md:{line} ({rfc} point {point})"
        if point not in rfcs.get(rfc, []):
            errors.append(f"{where}: no such §6.P point")
        if status not in STATUSES:
            errors.append(f"{where}: unknown status {status!r}")
        elif status == "suite":
            for test in re.findall(r"`([^`]+)`", proof) or [proof]:
                if test not in tests:
                    errors.append(f"{where}: test {test!r} does not exist")
        elif status == "hors suite" and not proof:
            errors.append(f"{where}: `hors suite` without a pointer or a reason")
        elif status == "quarantaine":
            quarantined += 1
            if not re.search(r"#\d+", proof):
                errors.append(f"{where}: quarantine without an issue number")
        elif status == "à écrire" and rfc not in PENDING:
            errors.append(f"{where}: `à écrire` for an RFC whose code is written")
    if quarantined:
        warnings.append(f"{quarantined} row(s) in quarantine")
    for name in path_deps:
        errors.append(f"tests/integration depends on {name!r} by path (RFC 0009 §5.3)")
    return errors, warnings


def self_test() -> None:
    rfc = "### 6.P Proof\n\n1. one\n2. two\n\n**Deliberately untouched**\n\n3. not a point\n"
    assert rfc_points(rfc) == [1, 2]
    good = ("| RFC | Point | Statut | Preuve |\n| --- | --- | --- | --- |\n"
            "| 0003 | 1 | suite | `rfc0003::p1_ok` |\n"
            "| 0003 | 2 | hors suite | job front-lint |\n")
    tests = {"rfc0003::p1_ok", "e2e-it/login.spec.ts › full login"}
    assert check({"0003": [1, 2]}, good, tests, []) == ([], [])
    # Point 5: a row naming a missing test fails, and names the row.
    dead = good.replace("p1_ok", "p99_missing")
    errors, _ = check({"0003": [1, 2]}, dead, tests, [])
    assert errors == ["MATRIX.md:3 (0003 point 1): test 'rfc0003::p99_missing' does not exist"], errors
    # Point 6: a point added to an RFC without its row fails.
    errors, _ = check({"0003": [1, 2, 3]}, good, tests, [])
    assert errors == ["RFC 0003 §6.P point 3 has no row in MATRIX.md"], errors
    assert check({"0003": [1, 2]}, good.replace("job front-lint", ""), tests, [])[0]
    quarantine = good.replace("hors suite | job front-lint", "quarantaine | flaky")
    assert check({"0003": [1, 2]}, quarantine, tests, [])[0]
    ok, warnings = check({"0003": [1, 2]}, quarantine.replace("flaky", "flaky #312"), tests, [])
    assert ok == [] and warnings == ["1 row(s) in quarantine"]
    todo = good.replace("| suite | `rfc0003::p1_ok` |", "| à écrire | |")
    assert check({"0001": [1, 2]}, todo.replace("0003", "0001"), tests, [])[0]
    assert check({"0010": [1, 2]}, todo.replace("0003", "0010"), tests, [])[0] == []
    assert check({"0003": [1, 2]}, good, tests, ["api"])[0]
    pw = good.replace("`rfc0003::p1_ok`", "`e2e-it/login.spec.ts › full login`")
    assert check({"0003": [1, 2]}, pw, tests, [])[0] == []
    print("matrix self-test: ok")


def main() -> int:
    if "--self-test" in sys.argv:
        self_test()
        return 0
    rfcs = {}
    for path in sorted((ROOT / ".rfc").glob("[0-9][0-9][0-9][0-9]-*.md")):
        rfcs[path.name[:4]] = rfc_points(path.read_text())
    errors, warnings = check(
        rfcs,
        MATRIX.read_text(),
        rust_tests(ROOT) | playwright_tests(ROOT),
        path_dependencies(ROOT),
    )
    for warning in warnings:
        print(f"it:matrix: warning: {warning}")
    for error in errors:
        print(f"it:matrix: {error}", file=sys.stderr)
    rows = matrix_rows(MATRIX.read_text())
    counts = {s: sum(1 for r in rows if r[3] == s) for s in sorted(STATUSES)}
    print("it:matrix: " + ", ".join(f"{n} {s}" for s, n in counts.items()))
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
