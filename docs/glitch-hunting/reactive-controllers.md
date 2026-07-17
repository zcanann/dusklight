# Reactive input controllers

Reactive controllers cover tactics that cannot be authored honestly as a
precomputed stick sequence: move toward a coordinate, track an actor with an
offset, or layer a timed correction over a base curve. They are a discovery
primitive. Their realized raw input remains the exact replay artifact.

## Safety and ownership boundary

The controller path has a deliberately narrow contract:

- Rust parses and validates the textual program and writes canonical binary
  `DUSKCTRL` v1.
- C++ decodes it once, captures bounded immutable observations, and evaluates
  one frame immediately before `JUTGamePad::read()`.
- The evaluator is pure and allocation-free. It receives a program plus a
  snapshot and returns one raw port-0 PAD state.
- Runtime integration reads player position, camera yaw, and actor identity and
  position. It never writes gameplay state.
- The only game-facing write is the existing exclusive virtual PAD injection.
  Everything else written is automation-owned trace, tape, and lifecycle state.
- Without an active controller the runtime does not scan actors, build an
  observation, or change PAD ownership.

This is an instrumentation rule, not permission for convenient game mutation.
A future timing fix belongs in the running substrate where possible and needs
an explicit determinism test; it must not silently alter gameplay state.

## Timeline language

Programs are bounded to 1,000,000 ticks and 32 layers. One replacement stick
layer may be active at a time. Any number of bounded additive layers may overlap
it; their X/Y results are summed and clamped once. Button layers overlap by
bitwise OR.

```text
duskcontrol 1
frames 120

bezier replace from 0 for 120 p0 0 127 p1 0 127 p2 127 0 p3 127 0
seek point add from 0 for 120 target 1 2 3 offset 0 0 0 magnitude 20 stop 5
seek actor add from 10 for 40 actor 123 offset 1 0 2 magnitude 30 stop 10
seek actor add from 50 for 20 actor 123 process 417 offset 0 0 0 magnitude 30 stop 10
seek actor add from 70 for 20 actor 123 set 14 room 0 stage F_SP103 offset 0 0 0 magnitude 30 stop 10
buttons from 5 for 1 B
```

`seek point` moves camera-relative toward `target + offset`. Plain `seek actor`
selects the nearest matching actor, breaking equal-distance ties by process ID.
`process ID` follows exactly that runtime instance and never falls back.
`set SETID room ROOM stage STAGE` selects the matching map-authored placement;
duplicate matches deterministically choose the lowest process ID. A missing
player, camera, or matching actor yields neutral output. The stop radius
suppresses movement near the target.

`DUSKCTRL` 1.1 writes these selectors while the decoder retains strict 1.0
nearest-selector compatibility. Runtime process IDs are session-local. The
placed selector is portable across sessions of the same validated build and
game data because it combines stage, actor type, home room, and the
stage-authored set ID. Actor catalogs embed the current build identity; a game-
data digest and automatic selector/catalog compatibility check remain future
guardrails.

Add `--actor-catalog build/actors.json` to an automation launch to capture a
read-only catalog at the automation endpoint, before any headful handoff. Each
record contains symbolic/numeric type, enemy classification, process and parent
IDs, set ID, parameters, home/current room and position, health, and status.
This is intended for a one-off probe whose placed identity is copied into a
later portable selector. A process ID may be used only in the same persistent
engine session or after explicitly proving that a replay recreated that ID; the
current process-per-run CLI cannot feed an exit catalog back into the run that
produced it.

Cubic Bezier layers use exact integer Bernstein evaluation with defined
ties-away rounding, including the one-million-frame boundary. This makes the
same compiled program portable across supported native compilers.

Compile and inspect without launching the game:

```powershell
cargo run --manifest-path tools/huntctl/Cargo.toml -- controller compile `
  tests/fixtures/automation/intro_seek_forward.duskctl build/intro-seek-forward.dctl
cargo run --manifest-path tools/huntctl/Cargo.toml -- controller inspect `
  build/intro-seek-forward.dctl
```

The VS Code `Glitch Hunt: Run Selected Test` selector includes
`input-controller` for the game-data-free native decoder/evaluator suite.

## Prefixes, handoff, and promotion

A tape and controller may be combined. The tape has exact priority through its
final frame; controller frame zero executes on the next simulation tick. When a
headful controller ends, exclusive automation input is released so a person can
take over. A headless run can terminate with `--exit-after-controller`.

```powershell
dusklight --headless --dvd game.iso `
  --input-tape build/prefix.tape `
  --input-controller build/route.dctl `
  --exit-after-controller `
  --realized-input-tape build/route-realized.tape `
  --gameplay-trace build/route-controller.trace
```

The realized tape contains the prefix followed by the raw, pre-`PADClamp`
controller output. Cold-replay that tape and require the same native milestones
before promoting a result. Gameplay traces mark controller-driven frames
separately from tape-driven frames, so offline tooling cannot accidentally
treat a reactive run as an absolute tape.

## Live continuation recording

Headful playback can record the human continuation after an exact automation
handoff:

```powershell
dusklight --dvd game.iso `
  --input-tape build/prefix.tape `
  --record-input-tape build/manual-continuation.tape `
  --record-input-capacity 1080000 `
  --record-input-session 0123456789abcdef0123456789abcdef `
  --record-input-start-milestone gameplay-ready-f-sp103 `
  --record-input-start-fingerprint 0123456789abcdef0123456789abcdef
```

The output is continuation-only: its first frame is the first live PAD read
after automation releases ownership. Recording is armed during the prefix but
does not sample it. The native hook captures host-mapped PAD state before the
game's non-idempotent `PADClamp`; replay injects at the same boundary and
therefore clamps exactly once. Mouse camera, gyro, and Dusklight's physical and
virtual custom action bindings remain suppressed for the whole recording
session because DUSKTAPE cannot represent those side channels. Action-binding
press state still advances to release any stale hold rather than freezing it.

The default capacity is 1,080,000 frames, or ten hours at 30 Hz. Reaching it is
a hard failed draft, but the bounded partial tape is retained on clean exit.
The destination and its sibling `<tape>.status.json` must not already exist.
Both are written through same-directory temporary files and renamed only after
flush; the status sidecar is the final native artifact action. Consumers must
still wait for a successful child-process exit before marking a draft ready.

The optional 32-lowercase-hex session token is echoed as `session_token` in the
status. The start milestone and fingerprint must be supplied together. When
present, the first hit must carry the expected fingerprint at the exact final
automation tape frame or recording never begins. They may be omitted only for
an exploratory child of a manual draft; that ancestry remains unverified and
cannot be promoted until a native boundary is established.

Status schema `dusklight.input-recording/v2` reports `success`, `zero_frames`,
`never_reached_handoff`, `start_boundary_mismatch`, `capacity_exhausted`, or
`write_failure`, along with frame count/capacity, handoff and exhaustion flags,
session token, start-boundary fields, an optional error, and the authoritative
`process_success` boolean. That boolean is true only when every native failure
term used for the final process exit is false, including the recording result.
`status` remains recording-local: a successfully written recording may still
report `process_success: false` when another requested artifact, milestone,
oracle, or runtime check failed. Promotion requires recording status `success`,
`process_success: true`, and a zero child-process exit. A status-write failure
leaves no usable sidecar and exits nonzero. Other non-success states also exit
nonzero. A normal window close is the intended way to finalize a headful
recording; headless and exit-at-prefix modes are rejected.

### Recording from authored Boot boundary zero

An authored `pre_input`, `stable 1` milestone can start a new human recording
without a dummy tape or controller frame:

```powershell
dusklight --dvd game.iso --fixed-step `
  --record-input-from-boot `
  --record-input-tape build/from-boot.tape `
  --milestone-program build/automation-state/milestones/intro.dmsp `
  --milestones process_boot `
  --milestone-result build/from-boot.milestones.json `
  --record-input-start-milestone process_boot
```

The game begins with host input quarantined and the recorder armed. Immediately
before the first `mDoCPd_c::read`, native code evaluates and verifies the
authored Boot hit at boundary index zero with no tape frame, begins recording,
then releases quarantine. Consequently recorded frame zero is the first human
PAD read. `--record-input-start-fingerprint` is optional in this mode; when
supplied it remains an exact guardrail.

Boot recording is headful and may use `--fixed-step`, but rejects `--headless`,
`--unpaced`, `--input-tape`, `--input-controller`, and `--milestone-goal`.
Reaching the Boot milestone is the start condition rather than a request to
stop the process.

The v2 status object additively reports `expected_start_fingerprint`,
`start_boundary_kind`, `start_boundary_index`, `start_program_digest`, and
`start_definition_digest`. A verified Boot recording reports kind `boot`, index
`0`, a null `start_tape_frame`, and the actual predicate digests and boundary
fingerprint used to authorize the first live read.

### Hidden prefix fast-forward

For route-tree “play from parent,” a headful absolute tape can skip rendering
its already-reviewed prefix to the desktop without skipping simulation:

```powershell
dusklight --dvd game.iso `
  --input-tape build/selected-chain.tape `
  --input-tape-fast-forward-frames 12345
```

The frame count is the exact composed length of the selected segment or draft's
direct parent and must satisfy `0 < N < tape frame count`. The workbench first
verifies that those `N` frames byte-match the current parent chain, including
for generated search nodes. Dusklight starts a real
presentation-capable backend hidden, mutes host audio, and runs full rendering
and simulation in unpaced fixed-step mode. Once frame `N - 1` has completed its
simulation tick, the completed-frame count is exactly `N`, but the window stays
hidden until `aurora_end_frame` has submitted that parent-boundary image. Audio
output is then restored and the same deterministic fixed-step clock continues
with a paced 30 Hz outer loop; that limiter paces the just-completed parent
frame. The first exposed buffer is therefore the parent boundary, and the next
PAD read consumes child frame `N`. The tape is neither seeked nor truncated and
no gameplay state is snapshotted or restored. Conditioned tapes,
looping/holding end behavior, headless/unpaced launches, and exit-after-tape
are rejected so the reveal boundary and eventual live handoff remain
unambiguous.

For authoritative **Record child from parent**, `N` is instead exactly the
absolute prefix tape length. This equality is accepted only for a direct
recording handoff with release end behavior and no controller continuation:

```powershell
dusklight --dvd game.iso --fixed-step `
  --input-tape build/exact-parent-chain.tape `
  --input-tape-end release `
  --input-tape-fast-forward-frames 12345 `
  --record-input-countdown-seconds 3 `
  --record-input-tape build/child-continuation.tape
```

On the final prefix tick, native code arms reveal but keeps the submitted
parent-boundary frame hidden. Dusklight verifies and binds that exact boundary,
submits it, then reveals it. An optional 0-10 second host-only countdown redraws
only its overlay over the retained parent image: it admits no VI retrace,
emulated time, PAD read, game/audio tick, or tape frame. At zero, Dusklight
begins the already armed recorder, prepares the physical controller handoff,
releases input quarantine, restores audio, and enters the next outer loop.
That loop's PAD read is therefore simultaneously the first live child input
and recorded frame zero. A missing or failed boundary proof aborts while the
window remains hidden. Without `--record-input-tape`, equality remains an error
so ordinary Play from parent always retains a visible tape continuation.
The recording status binds this unnamed handoff with `start_boundary_kind` set
to `"tick"`, `start_boundary_index` set to `N`, and `start_tape_frame` set to
`N - 1`; milestone and fingerprint remain null when the parent draft did not
require them.

## Current limits

- Observation and catalog capture retain the same 256 lowest process IDs and
  report catalog truncation explicitly.
- Actor selectors currently use the numeric actor name emitted by the catalog;
  symbolic DSL names can be layered over that value without changing the wire
  evaluator.
- Programs are timed layers, not state machines yet. Conditions and transitions
  should be introduced only with explicit bounds and deterministic semantics.
- Controller continuations and realized-tape output reject conditional prefixes.
  Convert the prefix to an absolute tape before composing it with a controller;
  this prevents waited or skipped gates from changing handoff timing on replay.
- The controller trace is discovery evidence. The realized-tape cold replay and
  its native milestones are the promotion evidence.
