#!/usr/bin/env python3
"""Reject reverse or undeclared dependencies between huntctl domain crates."""

from pathlib import Path
import json
import subprocess


REPOSITORY = Path(__file__).resolve().parents[1]
WORKSPACE = REPOSITORY / "tools" / "huntctl"
ROOT_MANIFEST = WORKSPACE / "Cargo.toml"
ROOT_SOURCE = WORKSPACE / "src"

EXPECTED_ROOT_RUST_FILES = {
    "corpus_ops.rs",
    "lib.rs",
    "main.rs",
}

EXPECTED_ROOT_MODULE_DIRECTORIES = {
    "benchmark",
    "cli",
}

# The executable is an adapter, not a domain-owner escape hatch. These are
# ratchets: lower them as commands move into their domain modules, and never
# raise them to accommodate another flat implementation.
ROOT_FILE_LINE_BUDGETS = {
    "corpus_ops.rs": 1_000,
    "lib.rs": 200,
    "main.rs": 2_500,
}
ROOT_MODULE_FILE_LINE_BUDGET = 2_000
CRATE_ENTRYPOINT_LINE_BUDGET = 2_500
CRATE_IMPLEMENTATION_LINE_BUDGET = 3_000

# These integration-heavy crates are deliberately closed inventories. Adding a
# new sibling module requires an explicit ownership-policy edit instead of
# quietly recreating a general-purpose dumping ground.
EXPECTED_COORDINATION_SOURCE_FILES = {
    "evaluation": {
        "lib.rs",
        "search_evaluator.rs",
        "search_evaluator/boot_optimization.rs",
        "search_evaluator/native_result.rs",
        "search_evaluator/search_runs.rs",
        "search_evaluator/tests.rs",
        "search_evaluator/tournament.rs",
        "search_evaluator/trial.rs",
    },
    "harness-runtime": {"execution.rs", "inspection.rs", "lib.rs"},
    "orchestration": {
        "harness/campaign.rs",
        "harness/conformance.rs",
        "harness/mod.rs",
        "lib.rs",
    },
    "proposals": {"behavior_archive.rs", "lib.rs", "q_search.rs"},
    "workbench": {
        "draft_store.rs",
        "graph_projection.rs",
        "lib.rs",
        "milestone_program.rs",
        "playback.rs",
        "server.rs",
        "tests.rs",
    },
}

EXPECTED_MEMBERS = {
    ".",
    "crates/contracts",
    "crates/control",
    "crates/evidence",
    "crates/evaluation",
    "crates/harness-contracts",
    "crates/harness-runtime",
    "crates/interventions",
    "crates/learning",
    "crates/objectives",
    "crates/oracles",
    "crates/orchestration",
    "crates/proposals",
    "crates/routes",
    "crates/workbench",
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
    "dusklight-evaluation": {
        "dusklight-automation-contracts",
        "dusklight-control",
        "dusklight-evidence",
        "dusklight-harness-contracts",
        "dusklight-harness-runtime",
        "dusklight-learning",
        "dusklight-objectives",
        "dusklight-proposals",
        "dusklight-search",
        "dusklight-semantic-novelty",
        "dusklight-trace",
    },
    "dusklight-harness-contracts": {
        "dusklight-automation-contracts",
        "dusklight-control",
        "dusklight-objectives",
        "dusklight-search",
    },
    "dusklight-harness-runtime": {
        "dusklight-automation-contracts",
        "dusklight-control",
        "dusklight-harness-contracts",
        "dusklight-objectives",
        "dusklight-trace",
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
    "dusklight-orchestration": {
        "dusklight-automation-contracts",
        "dusklight-evaluation",
        "dusklight-harness-contracts",
        "dusklight-harness-runtime",
        "dusklight-objectives",
        "dusklight-search",
        "dusklight-trace",
    },
    "dusklight-proposals": {
        "dusklight-automation-contracts",
        "dusklight-control",
        "dusklight-evidence",
        "dusklight-learning",
        "dusklight-search",
    },
    "dusklight-routes": {
        "dusklight-automation-contracts",
        "dusklight-control",
        "dusklight-objectives",
        "dusklight-search",
    },
    "dusklight-route-workbench": {
        "dusklight-automation-contracts",
        "dusklight-control",
        "dusklight-evidence",
        "dusklight-harness-contracts",
        "dusklight-objectives",
        "dusklight-routes",
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
        "dusklight-evaluation",
        "dusklight-harness-contracts",
        "dusklight-interventions",
        "dusklight-learning",
        "dusklight-objectives",
        "dusklight-oracles",
        "dusklight-orchestration",
        "dusklight-proposals",
        "dusklight-routes",
        "dusklight-route-workbench",
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

root_rust_files = {path.name for path in ROOT_SOURCE.glob("*.rs")}
assert root_rust_files == EXPECTED_ROOT_RUST_FILES, (
    "huntctl root modules changed without an explicit orchestration ownership decision: "
    f"{sorted(root_rust_files)}"
)
root_module_directories = {
    relative.parts[0]
    for path in ROOT_SOURCE.rglob("*.rs")
    if len((relative := path.relative_to(ROOT_SOURCE)).parts) > 1
}
assert root_module_directories == EXPECTED_ROOT_MODULE_DIRECTORIES, (
    "huntctl root module directories changed without an explicit ownership decision: "
    f"{sorted(root_module_directories)}"
)

for name, budget in ROOT_FILE_LINE_BUDGETS.items():
    path = ROOT_SOURCE / name
    lines = len(path.read_text().splitlines())
    assert lines <= budget, (
        f"huntctl root adapter {name} grew past its {budget}-line architecture budget: {lines}"
    )

for directory in EXPECTED_ROOT_MODULE_DIRECTORIES:
    for path in (ROOT_SOURCE / directory).rglob("*.rs"):
        lines = len(path.read_text().splitlines())
        assert lines <= ROOT_MODULE_FILE_LINE_BUDGET, (
            f"huntctl CLI adapter {path.relative_to(ROOT_SOURCE)} grew past its "
            f"{ROOT_MODULE_FILE_LINE_BUDGET}-line architecture budget: {lines}"
        )

for crate_source in (WORKSPACE / "crates").glob("*/src"):
    entrypoint = crate_source / "lib.rs"
    if entrypoint.exists():
        lines = len(entrypoint.read_text().splitlines())
        assert lines <= CRATE_ENTRYPOINT_LINE_BUDGET, (
            f"crate entry point {entrypoint.relative_to(WORKSPACE)} grew past its "
            f"{CRATE_ENTRYPOINT_LINE_BUDGET}-line architecture budget: {lines}"
        )
    for path in crate_source.rglob("*.rs"):
        if path == entrypoint or path.name == "tests.rs":
            continue
        lines = len(path.read_text().splitlines())
        assert lines <= CRATE_IMPLEMENTATION_LINE_BUDGET, (
            f"crate module {path.relative_to(WORKSPACE)} grew past its "
            f"{CRATE_IMPLEMENTATION_LINE_BUDGET}-line architecture budget: {lines}"
        )

for crate_name, expected in EXPECTED_COORDINATION_SOURCE_FILES.items():
    source = WORKSPACE / "crates" / crate_name / "src"
    actual = {
        path.relative_to(source).as_posix() for path in source.rglob("*.rs")
    }
    assert actual == expected, (
        f"{crate_name} source ownership changed without updating the architecture policy: "
        f"expected {sorted(expected)}, got {sorted(actual)}"
    )

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
