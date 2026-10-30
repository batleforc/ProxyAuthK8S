#!/usr/bin/env python3
"""Fail if the release build enables `common/test-util` (RFC 0015 §6.1, T1).

Under that feature `Kv::Memory` exists: a server holding its state in memory
would give each pod its own throttles and revocations. The image is built with
`cargo build --release --workspace` (deploy/build/back/Containerfile), where
features are unified across every member, so the check reads the whole
workspace, not `-p server`, and leaves out dev-dependency edges (the test
crates enable the feature there on purpose).

`--self-test` runs the detector on a `cargo tree` output captured on
2026-10-30 with `common = { …, features = ["test-util"] }` added to
`apps/swaggergen`'s `[dependencies]`.
"""

import re
import subprocess
import sys

COMMAND = [
    "cargo", "tree", "--workspace", "--locked",
    "-e", "normal,build,features", "-i", "common",
]
FEATURE_NODE = 'common feature "test-util"'
CRATE = re.compile(r"([a-z0-9_-]+) v\d")

CAPTURED_LEAK = """\
common v0.4.2 (/repo/libs/server/common)
├── common feature "default" (command-line)
│   ├── api v0.4.2 (/repo/libs/server/api)
│   │   └── server v0.4.2 (/repo/apps/server)
└── common feature "test-util"
    └── swaggergen v0.4.2 (/repo/apps/swaggergen)
        └── swaggergen feature "default" (command-line)
"""
CAPTURED_CLEAN = CAPTURED_LEAK.split("└── common feature")[0]


def culprit(tree: str) -> str | None:
    """The first crate pulling `common/test-util`, or None when nothing does."""
    lines = tree.splitlines()
    for index, line in enumerate(lines):
        if FEATURE_NODE in line:
            for after in lines[index + 1:]:
                match = CRATE.search(after)
                if match:
                    return match.group(1)
            return "an unknown crate"
    return None


def self_test() -> int:
    assert culprit(CAPTURED_LEAK) == "swaggergen", culprit(CAPTURED_LEAK)
    assert culprit(CAPTURED_CLEAN) is None
    print("release-features: self-test passed")
    return 0


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()
    tree = subprocess.run(COMMAND, check=True, capture_output=True, text=True).stdout
    crate = culprit(tree)
    if crate is not None:
        print(
            f"release-features: common/test-util reaches the release build through {crate}; "
            "enable it from [dev-dependencies] only (RFC 0015 T1)",
            file=sys.stderr,
        )
        return 1
    print("release-features: common/test-util is absent from the release build")
    return 0


if __name__ == "__main__":
    sys.exit(main())
