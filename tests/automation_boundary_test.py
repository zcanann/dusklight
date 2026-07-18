#!/usr/bin/env python3
"""Static guardrails for fork-only observation and write-capable fidelity code."""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path


OBSERVER = "DUSK_ENABLE_AUTOMATION_OBSERVERS"
FIDELITY = "DUSK_ENABLE_AUTOMATION_FIDELITY_MODELS"
INTERVENTIONS = "DUSK_ENABLE_EXPERIMENTAL_INTERVENTIONS"
UNSAFE_LAB = "DUSK_ENABLE_UNSAFE_LAB_ADDRESS_WRITES"


@dataclass
class Conditional:
    inherited: frozenset[str]
    own: frozenset[str]

    @property
    def active(self) -> frozenset[str]:
        return self.inherited | self.own


def positive_guards(expression: str) -> frozenset[str]:
    guards: set[str] = set()
    for guard in (OBSERVER, FIDELITY, INTERVENTIONS):
        if re.search(rf"(?<![!A-Za-z0-9_]){guard}\b", expression):
            guards.add(guard)
    return frozenset(guards)


def guarded_lines(path: Path) -> list[tuple[int, str, frozenset[str]]]:
    result: list[tuple[int, str, frozenset[str]]] = []
    stack: list[Conditional] = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        stripped = line.lstrip()
        active = stack[-1].active if stack else frozenset()
        if stripped.startswith("#if ") or stripped.startswith("#ifdef "):
            expression = stripped.split(maxsplit=1)[1]
            frame = Conditional(active, positive_guards(expression))
            stack.append(frame)
            continue
        if stripped.startswith("#ifndef "):
            stack.append(Conditional(active, frozenset()))
            continue
        if stripped.startswith("#elif "):
            if not stack:
                raise AssertionError(f"{path}:{number}: unmatched #elif")
            expression = stripped.split(maxsplit=1)[1]
            stack[-1] = Conditional(stack[-1].inherited, positive_guards(expression))
            continue
        if stripped.startswith("#else"):
            if not stack:
                raise AssertionError(f"{path}:{number}: unmatched #else")
            stack[-1] = Conditional(stack[-1].inherited, frozenset())
            continue
        if stripped.startswith("#endif"):
            if not stack:
                raise AssertionError(f"{path}:{number}: unmatched #endif")
            stack.pop()
            continue
        result.append((number, line, active))
    if stack:
        raise AssertionError(f"{path}: unterminated conditional block")
    return result


def require_guard(path: Path, needles: tuple[str, ...], guard: str) -> list[str]:
    failures: list[str] = []
    for number, line, active in guarded_lines(path):
        if any(needle in line for needle in needles) and guard not in active:
            failures.append(f"{path}:{number}: {guard} does not guard: {line.strip()}")
    return failures


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    failures: list[str] = []

    gameplay_hooks = {
        root / "src/d/d_name.cpp": (
            "dusk/automation/",
            "dusk::automation::",
            "automationObserve()",
            "automationCursorMove()",
        ),
        root / "src/d/d_file_select.cpp": (
            "dusk/automation/",
            "dusk::automation::",
        ),
    }
    for path, needles in gameplay_hooks.items():
        failures.extend(require_guard(path, needles, OBSERVER))

    # Existing private-state integration declarations are also fork-only. A
    # future friend/read adapter must use the same dedicated observer gate;
    # TARGET_PC or an ordinary runtime branch is not an observation boundary.
    name_header = root / "include/d/d_name.h"
    failures.extend(require_guard(name_header, ("automationObserve()",), OBSERVER))
    failures.extend(require_guard(name_header, ("automationCursorMove()",), FIDELITY))
    observer_headers = tuple((root / "include").rglob("*.h")) + tuple(
        (root / "include").rglob("*.hpp")
    )
    legacy_native_automation_members = {
        (root / "include/d/d_name.h", "bool automationCursorMove();"),
        (root / "include/d/d_name.h", "void automationObserve();"),
    }
    for path in observer_headers:
        source = path.read_text(encoding="utf-8")
        # Native headers may grant a narrowly gated friendship, but they must
        # never contain the adapter implementation. Keeping the body in
        # src/dusk/automation makes the code visibly fork instrumentation and
        # prevents query logic from becoming gameplay implementation.
        if "dusk/automation" not in path.as_posix() and re.search(
            r"\b(?:class|struct)\s+\w*ReadAdapter\b[^;{]*\{", source
        ):
            failures.append(f"{path}: native header contains a read-adapter body")
        if "dusk/automation" not in path.as_posix():
            for number, line in enumerate(source.splitlines(), 1):
                declaration = line.strip()
                if not re.search(r"\bautomation[A-Z]\w*\s*\(", declaration):
                    continue
                if (path, declaration) not in legacy_native_automation_members:
                    failures.append(
                        f"{path}:{number}: native gameplay class adds an automation/query member"
                    )
        if not re.search(r"friend.*(?:dusk::automation|ReadAdapter)", source):
            continue
        for number, line, active in guarded_lines(path):
            stripped = line.strip()
            if "friend" not in stripped:
                continue
            if "dusk::automation" in stripped or "ReadAdapter" in stripped:
                if OBSERVER not in active:
                    failures.append(
                        f"{path}:{number}: automation friend/read adapter is not {OBSERVER}-guarded"
                    )
                source_lines = source.splitlines()
                aperture_context = "\n".join(source_lines[max(0, number - 3) : number])
                if "DUSKLIGHT OBSERVATION-ONLY APERTURE" not in aperture_context:
                    failures.append(
                        f"{path}:{number}: automation friend lacks observation-only aperture marker"
                    )

    # Every live-state mutation used to emulate an original-console memory
    # consequence must be impossible in read-only observer builds.
    failures.extend(
        require_guard(root / "src/d/d_name.cpp", ("setColorChanNum(12)",), FIDELITY)
    )

    observer_sources = (
        root / "src/dusk/automation/actor_catalog.cpp",
        root / "src/dusk/automation/game_state_observer.cpp",
        root / "src/dusk/automation/gameplay_trace_observer.cpp",
    )
    native_read_needles = (
        '#include "d/',
        '#include "f_op/',
        "dComIf",
        "fopAcIt_Executor",
        "fopAcM_Get",
    )
    for path in observer_sources:
        failures.extend(require_guard(path, native_read_needles, OBSERVER))

    forbidden_live_effects = (
        "const_cast<",
        "dComIfGp_set",
        "dComIfGs_set",
        "fopAcM_onSwitch",
        "fopAcM_offSwitch",
        "setSceneChangeOK(",
        "dStage_changeScene(",
        "checkArea(",
        "checkWork(",
        ".GetTriPla(",
        ".getPrismData(",
        ".getPolyCode(",
        ".GetExitId(",
        ".GetSpecialCode(",
        ".GetTriPnt(",
        ".GroundCross(",
        ".LineCross(",
        ".CrrPos(",
        ".GroundCheck(",
        ".LineCheck(",
        ".ClrGround",
        ".SetGround",
        ".ClrWall",
        ".SetWall",
        ".ClrWater",
        ".SetWater",
        "cM_rnd(",
        "cM_rndF(",
        "cM_rndFX(",
    )
    for path in observer_sources:
        source = path.read_text(encoding="utf-8")
        for needle in forbidden_live_effects:
            if needle in source:
                failures.append(f"{path}: observer contains forbidden live effect {needle}")

    main_source = (root / "src/m_Do/m_Do_main.cpp").read_text(encoding="utf-8")
    forbidden_main_reads = (
        "dComIfGp_getStartStage",
        "dComIfGp_roomControl_getStayNo",
        "dComIfGp_getPlayer",
        "dComIfGp_getCamera",
        "dComIfGp_getEvent",
        "dComIfGp_event_runCheck",
        "dComIfGp_isEnableNextStage",
        "dComIfGp_getNextStage",
        "dComIfG_play_c::getLayerNo",
        "fopAcIt_Executor",
        "getRunEventName",
    )
    for needle in forbidden_main_reads:
        if needle in main_source:
            failures.append(f"src/m_Do/m_Do_main.cpp contains native observation read {needle}")

    for path in (root / "src/dusk/automation").glob("*.cpp"):
        if ".getRunEventName(" in path.read_text(encoding="utf-8"):
            failures.append(f"{path}: non-const event-name query is forbidden")

    cmake = (root / "CMakeLists.txt").read_text(encoding="utf-8")
    for option in (OBSERVER, FIDELITY, INTERVENTIONS):
        if not re.search(rf"option\({option}\s+.*?\sOFF\)", cmake, re.DOTALL):
            failures.append(f"CMake option {option} must exist and default OFF")
    target_definitions = re.search(
        r"target_compile_definitions\(dusklight\s+PRIVATE(?P<body>.*?)\)", cmake, re.DOTALL
    )
    if target_definitions is None:
        failures.append("dusklight target compile definitions are missing")
    else:
        body = target_definitions.group("body")
        for option in (OBSERVER, FIDELITY, INTERVENTIONS):
            expected = f"{option}=$<BOOL:${{{option}}}>"
            if expected not in body:
                failures.append(f"dusklight target-wide compile gate omits {option}")

    raw_address_markers = (
        "InterventionOperation::WriteAddress",
        "InterventionOperation::RawAddress",
        "pub struct RawAddress",
        "pub enum RawAddress",
        "fn write_address(",
        "fn raw_address(",
    )
    intervention_sources = tuple((root / "tools/huntctl/src/intervention").rglob("*.rs")) + tuple(
        (root / "src/dusk/automation").glob("*intervention*.cpp")
    ) + tuple((root / "include/dusk/automation").glob("*intervention*.hpp"))
    unsafe_lab_sources = []
    for path in intervention_sources:
        source = path.read_text(encoding="utf-8")
        if not any(marker in source for marker in raw_address_markers):
            continue
        if "unsafe_lab" not in path.as_posix():
            failures.append(f"{path}: raw-address writes escape the separately named unsafe lab")
        else:
            unsafe_lab_sources.append(path)
    if unsafe_lab_sources:
        if not re.search(rf"option\({UNSAFE_LAB}\s+.*?\sOFF\)", cmake, re.DOTALL):
            failures.append(f"CMake option {UNSAFE_LAB} must exist and default OFF")
        if target_definitions is None or f"{UNSAFE_LAB}=$<BOOL:${{{UNSAFE_LAB}}}>" not in target_definitions.group("body"):
            failures.append(f"dusklight target-wide compile gate omits {UNSAFE_LAB}")

    if failures:
        print("Automation boundary violations:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("automation boundary tests passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
