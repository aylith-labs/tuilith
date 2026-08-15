"""Rewrite Cargo.lock to the lowest version each direct dependency's requirement actually allows.

A caret requirement is a promise: `ratatui = "0.30"` says a consumer already holding 0.30.0 can take
this crate. Nothing checks that promise, because resolution always picks the newest match — so the
crate is only ever built against versions far above its own floor, and the floor rots silently until
somebody's build fails at the requirement rather than at the code.

Run before a build to make the floor the version under test. Restore the lock afterwards.
"""

import json
import re
import subprocess
import sys

CARET = re.compile(r"^\^?(\d+)(?:\.(\d+))?(?:\.(\d+))?$")


def floor_of(requirement):
    """The lowest version a requirement admits, or None when its shape is not a simple caret."""
    match = CARET.match(requirement.strip())
    if not match:
        return None
    major, minor, patch = (part or "0" for part in match.groups())
    return f"{major}.{minor}.{patch}"


def main():
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1"],
            capture_output=True,
            check=True,
            text=True,
        ).stdout
    )
    root = next(
        package
        for package in metadata["packages"]
        if package["id"] in metadata["workspace_members"]
    )

    pins, skipped = [], []
    for dependency in root["dependencies"]:
        floor = floor_of(dependency["req"])
        if floor is None:
            skipped.append(f"{dependency['name']} {dependency['req']}")
            continue
        pins.append((dependency["name"], floor))

    # A requirement this script cannot read is a gap in the check, so it is reported rather than
    # dropped — a floor gate that silently covers less than it claims is worse than none.
    for entry in skipped:
        print(f"not a simple caret requirement, floor unchecked: {entry}", file=sys.stderr)

    for name, floor in pins:
        result = subprocess.run(
            ["cargo", "update", "-p", name, "--precise", floor],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            print(f"cannot pin {name} to its declared floor {floor}:", file=sys.stderr)
            print(result.stderr, file=sys.stderr)
            return 1
        print(f"{name} pinned to {floor}")

    return 1 if skipped else 0


if __name__ == "__main__":
    sys.exit(main())
