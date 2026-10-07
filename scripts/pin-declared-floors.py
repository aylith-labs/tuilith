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


def read_metadata():
    return json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1"],
            capture_output=True,
            check=True,
            text=True,
        ).stdout
    )


def root_of(metadata):
    return next(
        package
        for package in metadata["packages"]
        if package["id"] in metadata["workspace_members"]
    )


def package_spec(metadata, name):
    """The `cargo update -p` spec for a direct dependency of the root.

    The bare name while the lock holds one version of the crate. When a transitive dependency brings a
    second, the bare name is ambiguous and cargo refuses it, so the spec names the version the root
    itself resolved to — the one its requirement governs. None when that is not a single version.
    """
    versions = {
        package["version"] for package in metadata["packages"] if package["name"] == name
    }
    if len(versions) <= 1:
        return name
    by_id = {package["id"]: package for package in metadata["packages"]}
    root_id = root_of(metadata)["id"]
    node = next(node for node in metadata["resolve"]["nodes"] if node["id"] == root_id)
    direct = {
        by_id[dependency["pkg"]]["version"]
        for dependency in node["deps"]
        if by_id[dependency["pkg"]]["name"] == name
    }
    if len(direct) != 1:
        return None
    return f"{name}@{direct.pop()}"


def main():
    root = root_of(read_metadata())

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
        # Read afresh for each pin: the previous one can move what the lock holds.
        spec = package_spec(read_metadata(), name)
        if spec is None:
            print(f"cannot tell which {name} the root depends on, floor unchecked", file=sys.stderr)
            return 1
        result = subprocess.run(
            ["cargo", "update", "-p", spec, "--precise", floor],
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
