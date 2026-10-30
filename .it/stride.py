#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///
"""Check tests/SECURITY-MATRIX.md against the threat models (RFC 0015 §6.4).

Every `T<n>` row of a `### 7.1` section of `.rfc/NNNN-*.md`, for the RFCs in
`RFCS`, needs a row. A `test` row names the negative tests that prove it, as
`path/from/root.rs::function`, each of which must exist; `hors suite` needs a
reason or a pointer; `à écrire` is refused except for the RFCs in `PENDING`,
whose code (or whose backfill) is not written yet.

Tests are found by reading the sources, not by building them, so this runs in
`task lint` in a second. shortcut: a `fn name(` in a comment counts as a test;
switch to `cargo nextest list` if that ever lets a dead reference through.

`--self-test` runs the checks on fixtures.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MATRIX = ROOT / "tests/SECURITY-MATRIX.md"
# The RFCs whose threat models are backfilled (RFC 0015 decision 10).
RFCS = ("0010", "0011", "0012", "0013", "0014")
# 0010: its code is not written; it leaves this set in the PR that lands it.
PENDING = {"0010"}
STATUSES = {"test", "hors suite", "à écrire"}


def threat_rows(text: str) -> list[str]:
    """The `T<n>` identifiers of the `### 7.1` section of one RFC."""
    found, inside = [], False
    for line in text.splitlines():
        if line.startswith("### 7.1"):
            inside = True
            continue
        if inside and line.startswith(("### ", "## ")):
            break
        if inside and (m := re.match(r"\|\s*(T\d+[a-z]?)\s*\|", line)):
            if m.group(1) not in found:
                found.append(m.group(1))
    return found


def matrix_rows(text: str) -> list[tuple[int, str, str, str, str]]:
    """(line number, rfc, threat, status, proof) for every data row."""
    rows = []
    for number, line in enumerate(text.splitlines(), 1):
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) != 4 or not re.fullmatch(r"\d{4}", cells[0]):
            continue
        rows.append((number, *cells))
    return rows


def test_exists(root: Path, reference: str) -> bool:
    path, _, function = reference.rpartition("::")
    file = root / path
    return bool(path and function and file.is_file()
                and re.search(rf"\bfn {re.escape(function)}\s*\(", file.read_text()))


def check(threats: dict[str, list[str]], matrix: str, exists) -> list[str]:
    errors = []
    rows = matrix_rows(matrix)
    covered = {(rfc, threat) for _, rfc, threat, _, _ in rows}
    for rfc, ids in sorted(threats.items()):
        for threat in ids:
            if (rfc, threat) not in covered:
                errors.append(f"stride: RFC {rfc} {threat} has no row in SECURITY-MATRIX.md")
    for line, rfc, threat, status, proof in rows:
        where = f"stride: SECURITY-MATRIX.md:{line} (RFC {rfc} {threat})"
        if threat not in threats.get(rfc, []):
            errors.append(f"{where}: no such threat row in the RFC's §7.1")
        if status not in STATUSES:
            errors.append(f"{where}: unknown status {status!r}")
        elif status == "test":
            references = re.findall(r"`([^`]+::[^`]+)`", proof)
            if not references:
                errors.append(f"{where}: `test` names no `path.rs::function`")
            for reference in references:
                if not exists(reference):
                    errors.append(f"{where}: names a test that does not exist: {reference}")
        elif status == "hors suite" and not proof:
            errors.append(f"{where}: `hors suite` without a pointer or a reason")
        elif status == "à écrire" and rfc not in PENDING:
            errors.append(f"{where}: `à écrire` for an RFC whose backfill is due")
    return errors


def self_test() -> None:
    rfc = "### 7.1 Threat model\n\n| # | Threat |\n| --- | --- |\n| T1 | a |\n| T2 | b |\n\n### 7.2\n| T9 | not one |\n"
    assert threat_rows(rfc) == ["T1", "T2"]
    good = ("| RFC | T | Statut | Preuve |\n| --- | --- | --- | --- |\n"
            "| 0011 | T1 | test | `libs/a.rs::t1_refused` |\n"
            "| 0011 | T2 | hors suite | gitleaks |\n")
    exists = {"libs/a.rs::t1_refused"}.__contains__
    assert check({"0011": ["T1", "T2"]}, good, exists) == []
    errors = check({"0011": ["T1", "T2"]}, good.replace("t1_refused", "gone"), exists)
    assert errors == ["stride: SECURITY-MATRIX.md:3 (RFC 0011 T1): names a test that does not exist: libs/a.rs::gone"], errors
    errors = check({"0011": ["T1", "T2", "T3"]}, good, exists)
    assert errors == ["stride: RFC 0011 T3 has no row in SECURITY-MATRIX.md"], errors
    assert check({"0011": ["T1", "T2"]}, good.replace("gitleaks", ""), exists)
    todo = good.replace("| test | `libs/a.rs::t1_refused` |", "| à écrire | |")
    assert check({"0011": ["T1", "T2"]}, todo, exists)
    assert check({"0010": ["T1", "T2"]}, todo.replace("0011", "0010"), exists) == []
    print("stride self-test: ok")


def main() -> int:
    if "--self-test" in sys.argv:
        self_test()
        return 0
    threats = {}
    for rfc in RFCS:
        # The RFC itself, not an annex such as 0014-…-implementation.md.
        path = min((ROOT / ".rfc").glob(f"{rfc}-*.md"), key=lambda p: len(p.name))
        threats[rfc] = threat_rows(path.read_text())
    errors = check(threats, MATRIX.read_text(), lambda ref: test_exists(ROOT, ref))
    for error in errors:
        print(error, file=sys.stderr)
    rows = matrix_rows(MATRIX.read_text())
    counts = {s: sum(1 for r in rows if r[3] == s) for s in sorted(STATUSES)}
    print("stride: " + ", ".join(f"{n} {s}" for s, n in counts.items()))
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
