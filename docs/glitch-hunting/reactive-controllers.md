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
- `InputControllerStepRequest` v1.0 labels that snapshot `pre_input` and binds
  its simulation tick, absolute input frame, and controller frame. The pure,
  allocation-free evaluator synchronously returns exactly one fixed-size
  `InputControllerStepResponse` carrying the echoed counters and raw port-0 PAD
  state.
- Unsupported versions, a non-pre-input phase, an out-of-range frame, or a
  stale/mismatched response cannot inject input. A runtime response-contract
  failure exits through the harness `protocol_failure` terminal.
- Runtime integration reads player position/yaw, camera yaw, and actor identity
  and position. It never writes gameplay state.
- The only game-facing write is the existing exclusive virtual PAD injection.
  Everything else written is automation-owned trace, tape, and lifecycle state.
- Without an active controller the runtime does not scan actors, build an
  observation, or change PAD ownership.

This is an instrumentation rule, not permission for convenient game mutation.
A future timing fix belongs in the running substrate where possible and needs
an explicit determinism test; it must not silently alter gameplay state.

## Timeline language

Programs are bounded to 1,000,000 ticks and 32 layers. Main-stick and
sub-stick/camera motion are independent writer surfaces. Each permits one
active replacement layer plus any number of bounded additive layers. Button
layers overlap by bitwise OR. One optional active safety clamp applies after
both stick surfaces are composed.

```text
duskcontrol 1
frames 120

bezier replace from 0 for 120 p0 0 127 p1 0 127 p2 127 0 p3 127 0
seek point add from 0 for 120 target 1 2 3 offset 0 0 0 magnitude 20 stop 5
seek coordinate add from 0 for 120 frame world target 1 2 3 offset 0 0 0 magnitude 20 stop 5
seek coordinate add from 0 for 120 frame player target 0 0 100 offset 0 0 0 magnitude 20 stop 5
seek coordinate add from 0 for 120 frame camera target 0 0 100 offset 0 0 0 magnitude 20 stop 5
seek plane add from 0 for 120 frame world point 0 0 100 normal 0 0 1 magnitude 20 stop 5
seek path-point add from 0 for 120 path 42 point 7 target 1 2 3 offset 0 0 0 magnitude 20 stop 5
seek opening add from 0 for 120 opening 99 target 1 2 3 offset 0 0 0 magnitude 20 stop 5
neutral replace from 0 for 5
turn add from 5 for 5 direction left magnitude 40
brake add from 10 for 10 stop-speed 0.5 magnitude 50
align add from 20 for 20 frame world heading 1.5707963 tolerance 0.1 magnitude 60
maintain heading add from 40 for 20 frame camera heading 0 magnitude 70
maintain distance add from 60 for 20 frame world target 1 2 3 distance 50 tolerance 5 magnitude 80
camera replace from 0 for 120 x 0 y 30
camera add from 20 for 20 x -10 y 0
clamp from 0 for 120 main 110 substick 90
seek actor add from 10 for 40 actor 123 offset 1 0 2 magnitude 30 stop 10
seek actor add from 50 for 20 actor 123 process 417 offset 0 0 0 magnitude 30 stop 10
seek actor add from 70 for 20 actor 123 set 14 room 0 stage F_SP103 offset 0 0 0 magnitude 30 stop 10
buttons from 5 for 1 B
```

`seek point` targets a fixed world coordinate and produces the camera-relative
stick needed to move toward `target + offset`. `seek coordinate` makes the
target frame explicit. A `world` target is fixed; `player` and `camera` targets
are local vectors re-resolved from the current read-only pose every tick. Thus
`frame player target 0 0 100` continuously means "forward of the player," not a
world point captured when the layer began. A missing player yaw or camera yaw
yields neutral output for a layer that requires it.

`seek plane` projects the current player X/Z position onto the plane and moves
toward that projection. The point and normal use the selected frame; normals
must have a nonzero horizontal component. Y values remain in the canonical
three-dimensional description but do not affect the current ground-plane stick
evaluator.

`seek path-point` and `seek opening` follow a statically resolved world
position. The program also carries the nonzero path/opening identity (and path
point index) that produced the position, so inspection and later compatibility
checks retain provenance. The native evaluator validates that identity but does
not query or mutate map data each tick. Authors should regenerate these layers
when the validated build or game-data digest changes.

Plain `seek actor` selects the nearest matching actor, breaking equal-distance
ties by process ID.
`process ID` follows exactly that runtime instance and never falls back.
`set SETID room ROOM stage STAGE` selects the matching map-authored placement;
duplicate matches deterministically choose the lowest process ID. A missing
`SETID` value from the game is represented by the native unsigned value
`65535` and remains valid in an explicitly tagged placed selector. A missing
player or camera yields neutral output. A missing nearest-kind actor also stays
neutral because there is no exact identity to declare lost. By contrast, loss
of a process-ID or placed selector produces the typed native terminal reason
`TargetLost` before input for that tick; it never switches to nearest. A
truncated actor snapshot cannot prove absence, so it stays neutral until a
complete snapshot or the controller duration bound resolves the run. The stop
radius suppresses movement near the target.

`DUSKCTRL` 1.2 adds explicit coordinate frames, planes, resolved path points,
and inferred openings. The decoder retains strict 1.0 nearest-actor and 1.1
exact-actor compatibility. Runtime process IDs are session-local. The placed
selector is portable across sessions of the same validated build and game data
because it combines stage, actor type, home room, and the stage-authored set ID.
Actor catalogs embed the current build identity; a game-data digest and
automatic selector/catalog compatibility check remain future guardrails.

`DUSKCTRL` 1.3 adds six bounded motion primitives:

- `neutral` is an explicit replacement layer that writes a neutral main stick.
  It cannot be additive, because an additive neutral layer has no observable
  effect and would disguise an authoring mistake.
- `turn` writes a camera-relative lateral main stick (`left` is negative X,
  `right` is positive X) for the declared interval. It is a raw directional
  turn command, not a promise that every player procedure can turn in place.
- `brake` reads the current horizontal player velocity and commands the exact
  opposite world heading until speed is at or below `stop-speed`. Missing or
  non-finite velocity/camera observations produce neutral output.
- `align` resolves a heading in the world, current player, or current camera
  frame and drives that heading until the player's wrapped yaw error is within
  `tolerance`. Headings and angular tolerances are radians. Because local frames
  are re-resolved every tick, player-frame heading zero is already aligned.
- `maintain heading` continuously drives the resolved heading without an
  angular dead band.
- `maintain distance` resolves a coordinate each tick, moves toward it outside
  `distance + tolerance`, moves directly away inside `distance - tolerance`,
  and emits neutral input inside the inclusive band. Distance tolerance must
  not exceed the requested distance.

All six remain ordinary bounded controller layers: replace-layer overlaps are
rejected, additive results compose under the existing clamp, and every realized
raw frame must still be recorded before the result can become proof input.
Versions 1.0 through 1.2 remain strictly decodable.

`DUSKCTRL` 1.4 makes composition across the complete movement surface
explicit. `camera` writes exact signed raw sub-stick X/Y values in
`[-128, 127]`; it has the same `replace` and `add` modes as main-stick motion,
but a main replacement and camera replacement may overlap because they own
different fields. Main additions, camera additions, and button OR are
commutative, so changing layer order cannot change output.

`clamp` declares inclusive per-axis magnitude limits in `[0, 127]` for the main
and sub sticks. The evaluator first combines the replacement and all additions,
applies the canonical raw signed-byte clamp, and then applies the active safety
limits. Overlapping main replacements, overlapping camera replacements, and
overlapping safety clamps are rejected during both Rust and native decoding.
Thus no result depends on which ambiguous writer happened to appear last in the
file. Versions 1.0 through 1.3 remain strictly decodable.

## Bounded control flow and terminal results

Every currently executable adaptive path has a finite structural bound.
Controller programs have an exact duration no greater than 1,000,000 ticks and
contain no runtime jump or loop instruction. Tape `wait` and `pulse` gates carry
a required nonzero 16-bit timeout; timeout is a typed playback failure rather
than another wait cycle. Timeline branches are resolved before execution and
cyclic ancestry is rejected, while milestone bytecode has no jumps or mutable
control flow.

Exact actor loss is sampled before input. `evaluateDetailed` returns
`TargetLost` plus the responsible layer index, the runtime emits no raw frame
for that tick, releases controller ownership at the ordinary completion
boundary, and exits with the deterministic search-miss status. The realized
tape therefore remains exactly the prefix of frames the game consumed.

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

Inspection also emits `dusklight-controller-observation-provenance/v1`. Each
reactive layer names its exact declared observation families—player position,
yaw or velocity, camera yaw, stage name, actor identity/position, and bounded
snapshot completeness—and retains an exact actor selector where applicable.
This is the portable schema/selector provenance for the execution; the
realized input tape remains the consumed action proof.

If inspection reports `static_tape_compilable: true`, flatten the program
without launching the game:

```text
huntctl controller flatten build/static.dctl build/static.tape
```

Flattening uses the same exact integer Bézier rounding, independent main and
camera composition, button OR, raw clamp, and safety-clamp order as native
evaluation. Any observation-reading layer rejects flattening and returns its
provenance manifest rather than silently substituting neutral or guessed state.

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

### Trace-derived and compact waypoint policies

`derive-waypoint-policy` turns a successful player-motion trace into a bounded
reactive controller. It samples future world positions at an explicit interval
and lookahead, then layers either a periodic roll schedule or an explicit list
of roll ticks. The latter is the optimization surface: each rising edge can be
moved independently instead of forcing the whole route to share one cadence.

```text
huntctl controller derive-waypoint-policy success.trace route.dctl \
  --source-frame 440 --duration 160 --interval 10 --waypoint-phase 0 \
  --lookahead 15 --roll-frames 0,20,40,60,80,100,120,140 \
  --roll-hold 1 --magnitude 127 --terminal-extension 0
```

For a route already reduced to a few meaningful turns, `waypoint-policy`
builds the same ordinary `DUSKCTRL` artifact without retaining every sampled
trace point. Each `START:X:Y:Z` entry owns movement until the next start tick.
This compact representation is suitable for finite-sample continuous search
over switch ticks and waypoint coordinates.

```text
huntctl controller waypoint-policy route.dctl --duration 160 \
  --waypoints "0:-360:800:-2260;40:-1130:800:-2600;70:-1460:800:-2900;90:-1740:800:-4250" \
  --roll-frames 0,20,40,60,80,100,120,140 --roll-hold 1 --magnitude 127 \
  --button-layers "12:1:0x0200,32:1:0x0040"
```

`--button-layers` adds bounded button masks over the generated movement and
roll layers. This makes one-frame B (`0x0200`) or L (`0x0040`) experiments a
declared policy input rather than an ad hoc tape rewrite.

Neither command promotes a controller as proof. After a controller run,
`search candidate-from-tape` can import a selected realized suffix into the
ordinary learned-search candidate format. `--normalize-port-one` explicitly
discards secondary-port ownership/connectivity emitted by the runtime while
preserving the consumed port-zero PAD stream; it is never implicit. The
resulting candidate still has to pass repeated clean-boot milestone proof.

```text
huntctl search candidate-from-tape --input realized.tape \
  --output seed.candidate.json --segment fsp103_to_fsp104 \
  --start 440 --frames 128 --normalize-port-one
```

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

### Windowless exact-tape fast-forward

For route-tree fast playback, a headful absolute tape can suppress desktop
presentation for its complete run without skipping simulation:

```powershell
dusklight --dvd game.iso `
  --input-tape build/selected-chain.tape `
  --input-tape-fast-forward-frames 12345
```

The workbench uses `N = tape frame count`, after verifying the complete tape's
exact structural chain. Dusklight starts a real presentation-capable backend
with its window and swapchain presentation disabled, mutes audio, and runs full
GX work and simulation in unpaced fixed-step mode. Once terminal frame `N - 1`
has completed, Aurora enables presentation and submits one retained EFB frame
without a VI retrace, emulated tick, PAD read, or tape advancement. Only then
does Dusklight show the window and release live input at normal pacing. The
null backend is not used because its live GPU state cannot become graphical.
Conditioned tapes, looping/holding end behavior, controller continuations,
headless/unpaced launches, and exit-after-tape are rejected for this exact live
handoff. Partial `0 < N < tape frame count` acceleration remains available to
native callers. Workbench **Playback** is visible and paced from its declared
boot; **Resume (accelerated)** always uses the complete tape length.

For authoritative **Record child**, `N` is likewise the absolute prefix tape
length. Recording additionally verifies its authored start boundary before
exposing the retained image:

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
window remains hidden. Without `--record-input-tape`, the same equality performs
a direct live handoff rather than beginning a child recorder.
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
