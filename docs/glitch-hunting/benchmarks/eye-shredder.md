# Eye Shredder boot-tape benchmark

## Purpose

Eye Shredder is the first boot-tape benchmark because it requires no in-game
movement while exercising nearly the entire automation foundation:

- deterministic startup and menu navigation;
- exact D-pad/button timing;
- UI-state observations;
- an out-of-range cursor or memory-write path;
- headless event detection;
- promotion into a headful replay; and
- a clear boundary between gameplay/memory fidelity and console rendering
  fidelity.

The source corpus entry is Skybook's `file-name-cursor-breakout` post. Its
written description establishes the broad mechanism and links demonstrations,
but it does not currently provide a frame-by-frame input sequence or a complete
memory analysis. The first tape must therefore be transcribed and independently
verified rather than inferred from the summary.

## Important current finding

In `src/d/d_name.cpp`, the directional cursor code that can increment beyond
the normal final name position is compiled only under `#if !TARGET_PC`. The
native PC path therefore does not currently expose the same route through that
block. Other safety changes and native class-layout differences may also affect
the resulting overwrite.

This makes Eye Shredder a compatibility probe, not a regression we can assume
will pass. The fork must choose and record one of these outcomes:

1. the safe profile intentionally rejects the breakout;
2. a fidelity profile reproduces the original cursor behavior with explicit,
   bounded memory semantics;
3. the cursor breakout reproduces but its target/layout differs;
4. the overwrite reproduces but Aurora cannot reproduce the console visual;
   or
5. the full visible result reproduces.

All five are meaningful test results when labeled correctly.

The current native implementation under audit is
[`src/d/d_name.cpp`](../../../src/d/d_name.cpp), with its associated layout in
[`include/d/d_name.h`](../../../include/d/d_name.h).

## Benchmark stages

### Stage 0: deterministic boot

- Boot from a declared clean save fixture and settings profile.
- Reach the expected file/name UI semantic state.
- Replay at least 100 times with identical state hashes and event sequence.
- Do not use screenshots or fixed loading delays as the ready condition.

### Stage 1: exact menu tape

- Record the authoritative raw controller bytes for each tick.
- Replay with physical input fully disconnected from the automated port.
- Emit UI screen, selection, cursor position, entered characters, and accepted
  input events.
- Minimize neutral frames while retaining the same semantic outcome.

### Stage 2: cursor breakout

- Observe the logical cursor position separately from its clamped visual
  position.
- Trace each attempted cursor move and character write.
- Report the containing object, field-relative offset, MEM1 offset where
  applicable, and whether the write is within the native object.
- Compare safe and fidelity profiles without relying on an unguarded native
  out-of-bounds write.

### Stage 3: corruption signature

- Define the expected overwritten bytes/fields from an authoritative console
  trace or documented memory analysis.
- Emit a sparse write trace and a stable oracle for the intended corruption.
- Distinguish a different native-layout target from failure to escape the
  cursor range.

### Stage 4: headful rendering

- Replay the verified artifact in a headful worker.
- Begin presentation shortly before the first relevant write.
- Capture screenshots or a short video plus the same event/hash checkpoints.
- Classify the result as native-visible, console-only, or unresolved.

## Required instrumentation

- current and previous logical name cursor positions;
- visual cursor index and character-table selection;
- name buffer and neighboring field writes;
- UI screen/process transitions;
- canonical controller state consumed by the game;
- GC-relative and native-relative addresses for watched memory;
- profile/capability flags for the `TARGET_PC` compatibility path; and
- render-backend warnings or anomalous resource accesses.

## Acceptance criteria

The first milestone is complete when a single versioned artifact can be run
headless 100 times, produces identical input/UI/hash traces, and can be promoted
to a verified headful replay. Full reproduction of the visual symptom is not
required for that milestone; the artifact must instead say precisely which
fidelity stage was reached and why it stopped.

Before claiming console equivalence, obtain the exact retail version, settings,
input sequence, and memory target from an authoritative trace. Do not treat a
GC-looking low 32-bit host pointer as proof of retail address equivalence.
