#!/usr/bin/env python3
"""Fail when a dependency in [workspace.dependencies] is unused by all members."""

from __future__ import annotations

from pathlib import Path
import glob
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[2]
MANIFEST_FILENAME = "Cargo.toml"
ROOT_MANIFEST = ROOT / MANIFEST_FILENAME


def read_toml(path: Path) -> dict:
    with path.open("rb") as f:
        return tomllib.load(f)


def dep_keys_with_workspace_true(table: dict | None) -> set[str]:
    if not isinstance(table, dict):
        return set()

    keys: set[str] = set()
    for dep_name, dep_spec in table.items():
        if isinstance(dep_spec, dict) and dep_spec.get("workspace") is True:
            keys.add(dep_name)
    return keys


def collect_workspace_dependency_uses(manifest: dict) -> set[str]:
    used: set[str] = set()

    used |= dep_keys_with_workspace_true(manifest.get("dependencies"))
    used |= dep_keys_with_workspace_true(manifest.get("dev-dependencies"))
    used |= dep_keys_with_workspace_true(manifest.get("build-dependencies"))

    target_table = manifest.get("target")
    if isinstance(target_table, dict):
        for target_spec in target_table.values():
            if not isinstance(target_spec, dict):
                continue
            used |= dep_keys_with_workspace_true(target_spec.get("dependencies"))
            used |= dep_keys_with_workspace_true(target_spec.get("dev-dependencies"))
            used |= dep_keys_with_workspace_true(target_spec.get("build-dependencies"))

    return used


def resolve_member_manifests(members: list[str]) -> list[Path]:
    manifests: list[Path] = []

    for member in members:
        pattern = str(ROOT / member)
        matches = glob.glob(pattern)
        if not matches:
            continue

        for match in matches:
            candidate = Path(match)
            manifest_path = candidate / MANIFEST_FILENAME if candidate.is_dir() else candidate
            if manifest_path.name == MANIFEST_FILENAME and manifest_path.exists():
                manifests.append(manifest_path)

    # Keep deterministic order and remove duplicates while preserving order.
    seen: set[Path] = set()
    unique: list[Path] = []
    for manifest in sorted(manifests):
        if manifest not in seen:
            seen.add(manifest)
            unique.append(manifest)
    return unique


def main() -> int:
    root = read_toml(ROOT_MANIFEST)
    workspace = root.get("workspace")
    if not isinstance(workspace, dict):
        print("No [workspace] section found in root Cargo.toml")
        return 0

    workspace_dependencies = workspace.get("dependencies")
    if not isinstance(workspace_dependencies, dict):
        print("No [workspace.dependencies] section found in root Cargo.toml")
        return 0

    members = workspace.get("members")
    if not isinstance(members, list):
        print("No workspace members declared")
        return 0

    member_manifests = resolve_member_manifests([str(m) for m in members])

    used_workspace_deps: set[str] = set()
    for member_manifest in member_manifests:
        parsed = read_toml(member_manifest)
        used_workspace_deps |= collect_workspace_dependency_uses(parsed)

    declared_workspace_deps = set(workspace_dependencies.keys())
    unused_workspace_deps = sorted(declared_workspace_deps - used_workspace_deps)

    if unused_workspace_deps:
        print("Unused entries in [workspace.dependencies]:")
        for dep in unused_workspace_deps:
            print(f"  - {dep}")
        print("\nRemove them or reference them from member Cargo.toml using `{ workspace = true }`.")
        return 1

    print("All [workspace.dependencies] entries are used by at least one workspace member.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
