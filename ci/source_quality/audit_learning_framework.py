#!/usr/bin/env python3
"""Run the clean-checkout learning-framework quality and evidence audit."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
HUNTCTL_ROOT = REPOSITORY_ROOT / "tools" / "huntctl"
SCRATCH_BUNDLE_SCHEMA = "dusklight-native-tactic-scratch-evidence-bundle/v2"
SCRATCH_BUNDLE_SCHEMA_PREFIX = "dusklight-native-tactic-scratch-evidence-bundle/"
LAUNCH_SMOKE_SCHEMA = "dusklight-native-tactic-launch-smoke-bundle/v1"
LAUNCH_SMOKE_SCHEMA_PREFIX = "dusklight-native-tactic-launch-smoke-bundle/"
THROUGHPUT_BUNDLE_SCHEMA = "dusklight-native-tactic-throughput-evidence-bundle/v1"
THROUGHPUT_BUNDLE_SCHEMA_PREFIX = "dusklight-native-tactic-throughput-evidence-bundle/"
COLD_REPLAY_BUNDLE_SCHEMA = "dusklight-native-tactic-cold-replay-evidence-bundle/v1"
COLD_REPLAY_BUNDLE_SCHEMA_PREFIX = (
    "dusklight-native-tactic-cold-replay-evidence-bundle/"
)
SUBSYSTEM_PARITY_BUNDLE_SCHEMA = (
    "dusklight-native-subsystem-parity-evidence-bundle/v1"
)
SUBSYSTEM_PARITY_BUNDLE_SCHEMA_PREFIX = (
    "dusklight-native-subsystem-parity-evidence-bundle/"
)


def run(command: list[str], cwd: Path) -> None:
    print(f"+ {' '.join(command)}", flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def tracked_evidence_bundles() -> tuple[
    list[Path], list[Path], list[Path], list[Path], list[Path]
]:
    output = subprocess.check_output(
        ["git", "ls-files", "--", "*.json"],
        cwd=REPOSITORY_ROOT,
        text=True,
    )
    bundles: list[Path] = []
    launch_smokes: list[Path] = []
    throughput_bundles: list[Path] = []
    cold_replay_bundles: list[Path] = []
    subsystem_parity_bundles: list[Path] = []
    for relative in output.splitlines():
        path = REPOSITORY_ROOT / relative
        try:
            document = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError):
            continue
        if not isinstance(document, dict):
            continue
        schema = document.get("schema")
        if schema == SCRATCH_BUNDLE_SCHEMA:
            bundles.append(path.parent)
        elif isinstance(schema, str) and schema.startswith(SCRATCH_BUNDLE_SCHEMA_PREFIX):
            raise RuntimeError(
                f"unsupported committed scratch evidence bundle schema in {relative}: {schema}"
            )
        elif schema == LAUNCH_SMOKE_SCHEMA:
            launch_smokes.append(path.parent)
        elif isinstance(schema, str) and schema.startswith(LAUNCH_SMOKE_SCHEMA_PREFIX):
            raise RuntimeError(
                f"unsupported committed launch smoke bundle schema in {relative}: {schema}"
            )
        elif schema == THROUGHPUT_BUNDLE_SCHEMA:
            throughput_bundles.append(path.parent)
        elif isinstance(schema, str) and schema.startswith(THROUGHPUT_BUNDLE_SCHEMA_PREFIX):
            raise RuntimeError(
                f"unsupported committed throughput evidence bundle schema in {relative}: {schema}"
            )
        elif schema == COLD_REPLAY_BUNDLE_SCHEMA:
            cold_replay_bundles.append(path.parent)
        elif isinstance(schema, str) and schema.startswith(COLD_REPLAY_BUNDLE_SCHEMA_PREFIX):
            raise RuntimeError(
                f"unsupported committed cold replay evidence bundle schema in {relative}: {schema}"
            )
        elif schema == SUBSYSTEM_PARITY_BUNDLE_SCHEMA:
            subsystem_parity_bundles.append(path.parent)
        elif isinstance(schema, str) and schema.startswith(
            SUBSYSTEM_PARITY_BUNDLE_SCHEMA_PREFIX
        ):
            raise RuntimeError(
                f"unsupported committed subsystem parity bundle schema in {relative}: {schema}"
            )
    return (
        sorted(set(bundles)),
        sorted(set(launch_smokes)),
        sorted(set(throughput_bundles)),
        sorted(set(cold_replay_bundles)),
        sorted(set(subsystem_parity_bundles)),
    )


def main() -> int:
    run([sys.executable, "ci/source_quality/check_rust_file_sizes.py"], REPOSITORY_ROOT)
    run(["cargo", "fmt", "--all", "--", "--check"], HUNTCTL_ROOT)
    run(["cargo", "check", "--workspace"], HUNTCTL_ROOT)
    run(["cargo", "test", "-p", "dusklight-orchestration"], HUNTCTL_ROOT)

    (
        bundles,
        launch_smokes,
        throughput_bundles,
        cold_replay_bundles,
        subsystem_parity_bundles,
    ) = tracked_evidence_bundles()
    for bundle in bundles:
        run(
            [
                "cargo",
                "run",
                "--quiet",
                "--",
                "learn",
                "validate-tactic-scratch-bundle",
                "--bundle",
                str(bundle),
            ],
            HUNTCTL_ROOT,
        )
    for bundle in launch_smokes:
        run(
            [
                "cargo",
                "run",
                "--quiet",
                "--",
                "learn",
                "validate-tactic-launch-smoke",
                "--bundle",
                str(bundle),
            ],
            HUNTCTL_ROOT,
        )
    for bundle in throughput_bundles:
        run(
            [
                "cargo",
                "run",
                "--quiet",
                "--",
                "learn",
                "validate-tactic-throughput-curve-bundle",
                "--bundle",
                str(bundle),
            ],
            HUNTCTL_ROOT,
        )
    for bundle in cold_replay_bundles:
        run(
            [
                "cargo",
                "run",
                "--quiet",
                "--",
                "learn",
                "validate-tactic-cold-replay-bundle",
                "--bundle",
                str(bundle),
            ],
            HUNTCTL_ROOT,
        )
    for bundle in subsystem_parity_bundles:
        run(
            [
                "cargo",
                "run",
                "--quiet",
                "--",
                "benchmark",
                "validate-native-subsystem-parity-bundle",
                "--bundle",
                str(bundle),
            ],
            HUNTCTL_ROOT,
        )
    print(
        "Learning-framework audit passed "
        f"({len(bundles)} committed scratch evidence bundles and "
        f"{len(launch_smokes)} native launch smoke bundles and "
        f"{len(throughput_bundles)} throughput evidence bundles and "
        f"{len(cold_replay_bundles)} cold replay evidence bundles and "
        f"{len(subsystem_parity_bundles)} subsystem parity bundles validated)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
