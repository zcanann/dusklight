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


def run(command: list[str], cwd: Path) -> None:
    print(f"+ {' '.join(command)}", flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def tracked_scratch_bundles() -> list[Path]:
    output = subprocess.check_output(
        ["git", "ls-files", "--", "*.json"],
        cwd=REPOSITORY_ROOT,
        text=True,
    )
    bundles: list[Path] = []
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
    return sorted(set(bundles))


def main() -> int:
    run([sys.executable, "ci/source_quality/check_rust_file_sizes.py"], REPOSITORY_ROOT)
    run(["cargo", "fmt", "--all", "--", "--check"], HUNTCTL_ROOT)
    run(["cargo", "check", "--workspace"], HUNTCTL_ROOT)
    run(["cargo", "test", "-p", "dusklight-orchestration"], HUNTCTL_ROOT)

    bundles = tracked_scratch_bundles()
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
    print(
        "Learning-framework audit passed "
        f"({len(bundles)} committed scratch evidence bundles validated)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
