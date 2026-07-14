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
