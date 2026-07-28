#!/usr/bin/env python3
"""Reject oversized production Rust files and growth in known source debt."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
BASELINE_PATH = Path(__file__).with_name("rust_file_size_baseline.txt")
NEW_FILE_LIMIT = 1_500


def production_rust_files() -> list[Path]:
    output = subprocess.check_output(
        [
            "git",
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "*.rs",
        ],
        cwd=REPOSITORY_ROOT,
        text=True,
    )
    files = []
    for raw_path in output.splitlines():
        path = Path(raw_path)
        if (
            "target" in path.parts
            or "tests" in path.parts
            or path.name == "tests.rs"
            or path.name.endswith("_tests.rs")
        ):
            continue
        files.append(path)
    return sorted(files)


def load_baseline() -> dict[Path, int]:
    baseline: dict[Path, int] = {}
    for line_number, raw_line in enumerate(
        BASELINE_PATH.read_text(encoding="utf-8").splitlines(), start=1
    ):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        try:
            raw_path, raw_limit = line.split("\t", maxsplit=1)
            path = Path(raw_path)
            limit = int(raw_limit)
        except ValueError as error:
            raise ValueError(
                f"{BASELINE_PATH}:{line_number}: expected PATH<TAB>LINES"
            ) from error
        if path in baseline:
            raise ValueError(f"{BASELINE_PATH}:{line_number}: duplicate path {path}")
        if limit <= NEW_FILE_LIMIT:
            raise ValueError(
                f"{BASELINE_PATH}:{line_number}: obsolete exception for {path}"
            )
        baseline[path] = limit
    return baseline


def physical_line_count(path: Path) -> int:
    with path.open(encoding="utf-8") as source:
        return sum(1 for _ in source)


def main() -> int:
    baseline = load_baseline()
    files = production_rust_files()
    file_set = set(files)
    failures: list[str] = []

    for path in files:
        lines = physical_line_count(REPOSITORY_ROOT / path)
        limit = baseline.get(path, NEW_FILE_LIMIT)
        if lines > limit:
            kind = "grandfathered file grew" if path in baseline else "oversized file"
            failures.append(f"{path}: {lines} lines; limit {limit} ({kind})")
        elif path in baseline and lines <= NEW_FILE_LIMIT:
            failures.append(
                f"{path}: now {lines} lines; remove its obsolete baseline exception"
            )

    for path in sorted(set(baseline) - file_set):
        failures.append(f"{path}: stale baseline exception for a missing or test-only file")

    if failures:
        print("Rust source-size quality gate failed:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    print(
        f"Rust source-size quality gate passed for {len(files)} production files "
        f"({len(baseline)} shrinking debt exceptions)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
