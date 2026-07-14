# Intro route benchmarks

The normal-file intro is split into two named, branch-free TAS scenarios:

- `intro-first-exit` boots from process start, accepts the default Link and
  Epona names, skips the opening events, and reaches the first `F_SP103` scene
  exit;
- `intro-cutscene` continues through `F_SP104`, follows the ranch path, crosses
  the fence, and enters the first real cutscene (`demo01_04`).

Both sources are compact `dusktape 1` programs. Their compiled tapes contain no
conditional/wait frames. Headful playback releases all controller ports when
the tape ends, so a person can take over from the reached state.

## Current route and oracle

The first area starts at `F_SP103`, room 1, point 1. The tape takes the right
corner and follows the path to scene exit 1, which loads `F_SP104`, room 1,
point 0. The longer route follows the ranch path, angles left through the fence
with a held forward-left vector, then enters the event volume near
`(-1600, 200, -9050)`.

Success is semantic, not a screenshot or a sleep:

- first control: `F_SP103`, room 1, point 1, Link present, opening event ended;
- first exit: the exit event begins, followed by the `F_SP104` point-0 load;
- intro: `F_SP104` reloads at point 26 with map-tool event 9 and FNV-1a event
  name hash `783959030` (`demo01_04`).

The native `--gameplay-trace` stream records the stage tuple, actual post-clamp
pad state, Link procedure/position/velocity/facing, event state, and nearest
scene-exit actor on every completed simulation tick. `huntctl trace inspect`
extracts the milestones; `timeline` exposes state changes and input frames;
`compare` ranks several traces by milestone depth and tick.

## Baseline and scoring

One checked cold matrix on July 13, 2026 completed `intro-cutscene` 10/10 times
with isolated config, card, and cache roots:

- route control: tick 682–684;
- first exit: tick 1084–1088;
- first `F_SP104` transition: tick 1115–1119;
- `demo01_04`: tick 1818–1824.

The `route-to-first-exit` marker is fixed at tape tick 694, so the current
route-local score is 390–394 ticks. Report both values:

1. absolute process tick measures the whole boot path;
2. marker-relative tick measures movement quality without pretending that host
   startup latency is route skill.

The trigger tick is a better golf objective than completion of the following
load. Load completion contains host-I/O latency after the player has already
completed the route.

## Known timing side channel

Synchronous DVD and memory-card dispatch, fixed OS time, and absolute input do
not yet make the whole PC port deterministic. In cold isolated runs, the title
event has ended anywhere from tick 181 to beyond tick 400. Advancing the
simulation while host work is outstanding changes which UI receives a fixed
press. The checked normal boot puts its first context-sensitive A press at tick
400, but a later cold run proved this is only a useful baseline, not a durable
barrier. No larger guessed frame number can prove an unbounded host task is
finished.

Small physics populations also remain: identical input from identical reported
control coordinates reaches the first exit within a four-tick band. The runner
records min/median/max/spread and requires every run to reach the semantic
milestones. It does not claim false single-tick determinism.

The durable fix is an engine-level loading/readiness barrier that stalls logical
time while deterministic automation work is outstanding. Until that exists,
the console-boot scenarios are timing-leak probes rather than non-flaky CI
tests. Route frame golf and roll-spacing exploration should begin from an
explicit stage/save/checkpoint seed, then promote a candidate back to cold
process-boot replay after the barrier exists.

## Running and watching

```powershell
.\tools\glitch-hunting\run-intro-route.ps1 -Goal first-exit -Runs 10
.\tools\glitch-hunting\run-intro-route.ps1 -Goal intro-cutscene -Runs 10
```

In VS Code, run **Glitch Hunt: Play Visual Scenario** and select
`intro-first-exit` or `intro-cutscene`. The test selector exposes the same two
names for headless checked runs.

Each run writes its compact trace and JSON milestone summary beneath
`build/test-results/<scenario>/<timestamp>`. The matrix also writes
`matrix.summary.json`, finishes all requested runs before failing, and never
reuses the temporary memory card or cache. A failed run also copies its isolated
writable state beside the trace before cleanup so logs, card, and cache evidence
are available for diagnosis.

## Local search primitives

The current trace/marker combination is enough for manual A/B work:

```powershell
cargo run --manifest-path tools/huntctl/Cargo.toml -- trace compare `
  build/a.gameplay.trace build/b.gameplay.trace
```

The next high-value additions are:

1. a scenario manifest binding boot seed, tape, oracle, trace channels, and
   fidelity requirements into one artifact;
2. marker-addressed tape mutation (angle, magnitude, duration, roll spacing)
   with automatic compile/run/rank loops in Rust;
3. marker-relative objectives such as distance to a trigger, milestone depth,
   speed, collision state, and event entry tick;
4. delta debugging that removes or shrinks tape regions while preserving an
   oracle;
5. stage/save checkpoints for warmed search, followed by cold process-boot
   replay as the promotion gate;
6. corpus and branch-DAG metadata so candidate ancestry, build identity,
   seed, and score distributions remain reproducible;
7. spline/path authoring that compiles to ordinary analog frames—the raw tape,
   not the spline, remains replay authority.

Fast search and proof should remain separate populations. A warmed long-lived
worker is appropriate for mining thousands of route mutations. A hermetic cold
matrix is appropriate for declaring a tape robust.
