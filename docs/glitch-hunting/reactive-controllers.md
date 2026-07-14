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
buttons from 5 for 1 B
```

`seek point` moves camera-relative toward `target + offset`. `seek actor`
selects the nearest matching actor, breaking equal-distance ties by stable ID,
then moves toward its position plus the offset. A missing player, camera, or
matching actor yields neutral output for that layer. The stop radius suppresses
movement near the target.

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

- Observation capture retains at most 256 actors deterministically.
- Actor selection uses numeric process names; symbolic catalogs can be added
  without changing the wire evaluator.
- Programs are timed layers, not state machines yet. Conditions and transitions
  should be introduced only with explicit bounds and deterministic semantics.
- Controller continuations and realized-tape output reject conditional prefixes.
  Convert the prefix to an absolute tape before composing it with a controller;
  this prevents waited or skipped gates from changing handoff timing on replay.
- The controller trace is discovery evidence. The realized-tape cold replay and
  its native milestones are the promotion evidence.
