#!/usr/bin/env python3
"""Reject reverse or undeclared dependencies between huntctl domain crates."""

from pathlib import Path
import json
import subprocess


REPOSITORY = Path(__file__).resolve().parents[1]
WORKSPACE = REPOSITORY / "tools" / "huntctl"
ROOT_MANIFEST = WORKSPACE / "Cargo.toml"

EXPECTED_MEMBERS = {
    ".",
    "crates/contracts",
    "crates/control",
    "crates/evidence",
    "crates/harness-contracts",
    "crates/interventions",
    "crates/learning",
    "crates/objectives",
    "crates/oracles",
    "crates/routes",
    "crates/search",
    "crates/semantic-novelty",
    "crates/trace",
    "crates/worker",
    "crates/world",
}

ALLOWED_INTERNAL_DEPENDENCIES = {
    "dusklight-automation-contracts": set(),
    "dusklight-control": {"dusklight-automation-contracts"},
    "dusklight-evidence": {
        "dusklight-automation-contracts",
        "dusklight-control",
        "dusklight-trace",
    },
    "dusklight-harness-contracts": {
        "dusklight-automation-contracts",
        "dusklight-control",
        "dusklight-objectives",
    },
    "dusklight-interventions": {"dusklight-automation-contracts"},
    "dusklight-learning": {
        "dusklight-automation-contracts",
        "dusklight-control",
        "dusklight-evidence",
        "dusklight-objectives",
        "dusklight-trace",
        "dusklight-world",
    },
    "dusklight-objectives": {
        "dusklight-automation-contracts",
        "dusklight-trace",
    },
    "dusklight-oracles": {
        "dusklight-automation-contracts",
        "dusklight-trace",
    },
    "dusklight-routes": {
        "dusklight-automation-contracts",
        "dusklight-control",
        "dusklight-objectives",
        "dusklight-search",
    },
    "dusklight-search": {
        "dusklight-automation-contracts",
        "dusklight-control",
    },
    "dusklight-semantic-novelty": {
        "dusklight-automation-contracts",
        "dusklight-trace",
    },
    "dusklight-trace": {"dusklight-automation-contracts"},
    "dusklight-worker-protocol": {"dusklight-automation-contracts"},
    "dusklight-world": {"dusklight-automation-contracts"},
    "dusklight-huntctl": {
        "dusklight-automation-contracts",
        "dusklight-control",
        "dusklight-evidence",
        "dusklight-harness-contracts",
        "dusklight-interventions",
        "dusklight-learning",
        "dusklight-objectives",
        "dusklight-oracles",
        "dusklight-routes",
        "dusklight-search",
        "dusklight-semantic-novelty",
        "dusklight-trace",
        "dusklight-worker-protocol",
        "dusklight-world",
    },
}


metadata = json.loads(
    subprocess.check_output(
        [
            "cargo",
            "metadata",
            "--manifest-path",
            str(ROOT_MANIFEST),
            "--no-deps",
            "--format-version",
            "1",
        ],
        text=True,
    )
)
packages_by_id = {package["id"]: package for package in metadata["packages"]}


def member_path(package: dict) -> str:
    directory = Path(package["manifest_path"]).parent.resolve()
    relative = directory.relative_to(WORKSPACE.resolve())
    return "." if relative == Path(".") else relative.as_posix()


members = {
    member_path(packages_by_id[package_id]) for package_id in metadata["workspace_members"]
}
default_members = {
    member_path(packages_by_id[package_id])
    for package_id in metadata["workspace_default_members"]
}
assert members == EXPECTED_MEMBERS, (
    f"huntctl workspace members changed without updating the boundary policy: {members}"
)
assert default_members == members, "every huntctl crate must run under default workspace tests"

seen = set()
for package_id in metadata["workspace_members"]:
    manifest = packages_by_id[package_id]
    package = manifest["name"]
    seen.add(package)
    dependencies = {
        dependency["name"]
        for dependency in manifest["dependencies"]
        if dependency["name"].startswith("dusklight-")
        and dependency["source"] is None
    }
    expected = ALLOWED_INTERNAL_DEPENDENCIES.get(package)
    assert expected is not None, f"new huntctl crate {package!r} has no dependency policy"
    assert dependencies == expected, (
        f"{package} internal dependencies violate the one-way boundary: "
        f"expected {sorted(expected)}, got {sorted(dependencies)}"
    )

assert seen == set(ALLOWED_INTERNAL_DEPENDENCIES), "crate policy contains stale packages"
print("huntctl crate boundary tests passed")
