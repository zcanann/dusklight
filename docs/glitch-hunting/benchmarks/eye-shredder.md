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

The source corpus entry is Skybook's `file-name-cursor-breakout` post. The page
itself is sparse, but its linked videos and `Flags (USA)` spreadsheet provide
the exact first target. Those primary sources are transcribed below and checked
against the retail `dName_c` layout.

## Authoritative first target

Starting from the fresh NTSC-U default name `Link`:

1. press D-pad Right three times to move the name cursor from 4 to 7;
2. insert any character, advancing the cursor to 8;
3. press D-pad Right 106 times, exploiting the retail `mCurPos != 7` check to
   reach cursor 114;
4. press B to decrement to cursor 113;
5. select uppercase `M` and press A; and
6. press Start to confirm the corrupted file.

`mChrInfo` begins at `dName_c + 0x2CC` and each entry is eight bytes. Position
113 therefore writes at `dName_c + 0x654`. For a fresh USA allocation the
retail cached address is `0x81457688`, and uppercase `M` produces:

```text
0C 00 02 01 00 00 00 4D
```

The fields are character-grid column `12`, row `0`, English set `2`, active
flag `1`, and big-endian character `0x0000004D`. The spreadsheet describes the
first byte as causing an emulator diagnostic of “Mismatched configuration
between XF and BP stages.” The other documented positions are 130, 147, 164,
181, 198, 215, 232, and 249. A prior J2D Save Leak shifts the USA addresses by
`+0x80`; the first checked artifact explicitly declares `j2d_leak=false`.

The first byte is now mapped exactly: `0x81457688` is
`J2DMaterial[0].mColorBlock.mColorChanNum`, normally `1`, overwritten with
`12`. The remaining seven bytes replace the padding and first three channel
controls. Retail `J2DColorBlock::setGX` then loops 24 times over fixed
four-entry mapping/channel arrays. GX writes the full count `12` to XF but only
`12 & 7 == 4` to BP, creating the observed XF/BP mismatch and repeatedly
rewriting the four XF channel controls. The affected material stride is `0x88`,
or 17 filename entries, which explains every documented position above.

## Native fidelity boundary

The source port does not place the native `dName_c` object at GameCube address
`0x81457034`, and its host class layout contains 64-bit pointers. Performing the
retail out-of-bounds write against native memory would therefore corrupt an
unrelated host allocation and would not be console fidelity.

The `cursor_breakout_shadow` profile restores the excluded retail cursor bug
and applies writes to a bounded original-layout memory model through position
249. It never performs host out-of-bounds access. The autonomous oracle samples
that model after every game tick and fails unless position 113, offset `0x654`,
fresh-USA address `0x81457688`, and all eight bytes match exactly.

This makes Eye Shredder a compatibility probe with two separately labeled
outcomes:

1. `retail_write_reproduced`: the deterministic input and corruption signature
   match the GameCube model; and
2. `console_renderer_effect`: Aurora reproduces the XF/BP mismatch and visible
   artifact.

Both outcomes are automated. The renderer stage passes only after Aurora sees
a draw boundary with raw XF count `12`, raw BP count `4`, and the exact Eye
Shredder mismatch latch. A stale register value or the memory write alone
cannot satisfy it.

Aurora retains separate raw XF and BP counts and evaluates them only at real
draw boundaries. The native J2D bridge emits all 24 retail control calls while
cycling the four legal host entries with `i & 3`, preserving command
multiplicity without host out-of-bounds reads. Headful playback latches a
magenta/cyan shredded diagnostic with the live XF/BP values. That overlay is an
explicit diagnostic visualization, not a claim of pixel-perfect Flipper
rasterization.

The tape also contains state-gated frames. It waits neutrally while retaining
ownership of all controller ports until file select or name entry reports an
input-ready handler. A bounded timeout fails the process and oracle. This keeps
async resource speed from turning a guessed sleep into misplaced input while
fixed-step simulation runs faster than realtime.

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

Run the checked integration test with:

```powershell
.\tools\glitch-hunting\run-eye-shredder.ps1
```

It performs three isolated headless runs by default and requires identical
memory and renderer signatures. Use `-Runs 100` for the full determinism gate
or `-Visual` for paced headful playback. Visual mode continues after the oracle
passes, holds the diagnostic for six seconds, then consumes the final Start
inputs. Every run writes a versioned oracle result and a tick-stamped
name-entry trace under `build/test-results/eye-shredder`, while its temporary
config/card/cache state is deleted.

The test passes only when every requested run reports the same tape frame,
cursor position, original offset, fresh-USA cached address, eight-byte write,
and exact XF/BP mismatch draw. Pixel-accurate reproduction of the console's
triangular/rainbow artifact remains a later renderer-fidelity milestone.

Do not treat Aurora's separately allocated MEM1 buffer, or a GC-looking low
32-bit host pointer, as proof that native C++ game objects are retail-addressed.
