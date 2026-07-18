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

## Separate leaderboards

Boot and movement are deliberately scored on two leaderboards:

1. **Boot to route control** starts at process tick 0 and ends at the first
   memory-backed controllable `F_SP103`, room 1, point 1 state. It includes the
   title, file/name menus, and opening skip. This is where title-streaming and
   readiness-barrier work competes.
2. **`F_SP103` to `F_SP104` point 0** starts from the tape-declared
   `F_SP103,1,1,3` stage fixture and scores the first source-exit activation that
   is followed by an `F_SP104` point-0 transition. This is where movement,
   turns, rolls, splines, and route search compete.

Do not add the two scores or let title I/O variance decide which movement tape
wins. A route champion is promoted into a full process-boot tape for end-to-end
proof, but it retains its route-local score and ancestry.

The checked route-local scenario is `fsp103-next-map-seed`. Stage creation
happens before tape tick zero; its 180-frame neutral prefix covers only the
short automatic event. Candidate movement begins at its `candidate-start`
marker. On the current
baseline, one native milestone run observed exit activation at tape tick 571
and fully entered `F_SP104` room 1 point 0 at tick 640. It is a functional
seed, not a speed claim.

Evaluate any compiled candidate with:

```powershell
.\tools\glitch-hunting\evaluate-candidate.ps1 `
  -CandidateId cem-generation-3-member-17 `
  -CandidateTape build\search\g003-m017.tape
```

The stable result document contains `candidate_id`, tape path/hash, the exact
source and destination goal, `success`, native source-exit `first_hit_tick`,
entered-map goal tick, deepest milestone, process status, and trace/result
paths. Success requires the native `entered-f-sp104` goal, while ranking uses
the earlier native `exit-f-sp103-to-f-sp104` hit so uncontrollable loading time
does not masquerade as movement quality. The deepest milestone values are
`none`, `gameplay-ready-f-sp103`, `exit-f-sp103-to-f-sp104`, and
`entered-f-sp104`. This is suitable as the leaf
evaluation record for a Rust search round or candidate ancestry DAG.

## Current route and oracle

The first area starts at `F_SP103`, room 1, point 1. The tape takes the right
corner and follows the path to scene exit 1, which loads `F_SP104`, room 1,
point 0. The longer route follows the ranch path, angles left through the fence
with a held forward-left vector, then enters the event volume near
`(-1600, 200, -9050)`.

Success is semantic, not a screenshot or a sleep:

- first control: `F_SP103`, room 1, point 1, Link present, opening event ended;
- first exit: the exit event begins, followed by the `F_SP104` point-0 load;
- intro: `F_SP104` reloads at point 26 with map-tool event 9. The legacy event
  name hash identified this as `demo01_04`, but Trace v2 does not call the
  non-const event-manager name query merely to reproduce that diagnostic.

The native `--gameplay-trace` v4 stream records explicit post-simulation
boundaries and channel status, the current/pending stage tuple, all four
post-clamp pads, Link motion/action/animation state, event control, both global
RNG streams, realized camera, exact live `SCENE_EXIT` actor volumes, and Link's
already-resolved background-collision cache on every completed tick. Optional
channel 10 also decodes the six cached surface identities directly from bounded
DZB/KCL backing tables, including raw material words, source geometry indices,
and ground-exit SCLS resolution. It never issues a fresh collision query.
V4 additionally authenticates the exact canonical scenario descriptor after the channel region.
`huntctl trace inspect` extracts the milestones; `timeline` exposes state
changes and input frames; `compare` ranks several traces by milestone depth and
tick. Immutable v1-v3 files remain decodable.

The July 17, 2026 channel-10 matrix produced three byte-identical 925-record
traces (SHA-256
`6684fe14c53e6fbc1daa351f6c1149f3d06d5a2226c11dd5d021aa66b30bd4c0`). At
tick 826, cached ground identity `bg 0/poly 2217` resolves as KCL prism 2217,
PLC attribute 19, raw exit 1, and room-1 SCLS destination `F_SP104`, room 1,
point 0. Its source indices are position 672 and normals 7913 through 7916.
The offline archive reader independently joins it to KCL SHA-256
`6c0170b0b2bb7edf72a76e3a7b4c4c99c3d59c8aae27c59cdeaa708e90ff16be` and
PLC SHA-256
`3bd424f6fc4509d6c5e36872a7dc248d48a43d0ae122b0b1fc83b3001523c91b`, then
reconstructs the exact triangle and point-distance query.

The earlier matrix also disproved an assumption about the first load zone. At tick 826,
the pending transition becomes `F_SP104`, room 1, point 0, while the only live
`SCENE_EXIT` actor is 5,215.09 units from Link, has realized signed distance
3,846.9988, is not latched, and resolves to `F_SP103`, room 0, point 102. The
actual transition is driven by exit metadata on Link's cached ground-collision
polygon. Consequently, `SCENE_EXIT` actor telemetry and collision-polygon exit
telemetry are separate facts and are never substituted for one another.

The first-control milestone also requires event ID `-1`. Checking only
`eventRunning == false` is insufficient because player construction briefly
precedes the opening event; that transient gap is not durable controller
handoff. Results captured with the older predicate are invalid for boot golf
and must be rescored.

The current curated boot frontier reaches durable control at simulation/tape
frame 439 with boundary fingerprint
`bae78db89b3e5e4e5b9ae61c4ca43867` under the v2 fingerprint contract. It was
obtained by reducing an 800-frame
alternating A/Start baseline while holding that tick and fingerprint immutable.
The resulting absolute tape has eight non-neutral frames and reproduced the
same proof in 20/20 independent cold boots. Generated populations and evidence
remain ignored under `build/`; the compact TAS source is checked in at
`routes/intro/segments/golf439.tas`.

An exhaustive timing pass then evaluated every legal earlier timestamp for
each surviving pulse. The locally minimal timestamp vector is
`115, 147, 227, 235, 237, 268, 335, 337`; all 332 coordinate candidates used
three exact repetitions, and the final artifact passed a separate 20/20 cold
proof. It remains a one-coordinate minimum for this button-order family, not a
global optimality claim over different input structures.

The fixed `intro-first-exit` selector now uses this same prefix. A three-run
end-to-end regression reached control at tick 439, the first exit trigger at
827, and the `F_SP104` load at 858 on every run. That validates composition
with the existing movement tail; it is not a claim that the movement tail is
golfed.

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

## Timing boundary

The observed cold-boot title variance was traced to `JASDvd` completion work
crossing from the host audio task thread into fixed-step simulation. Automation
now dispatches those DVD-backed audio loads inline, as it already does for the
other deterministic I/O paths. The checked boot then reproduced its exact
strict trace 20/20 times in one batch and 45/45 independent cold runs. The tape
still contains only absolute input; no reactive readiness check masks drift.

An older matrix showed a four-tick band from apparently identical route state;
that is framework-bug evidence, not an acceptable robustness distribution. The
current three-run absolute tape reached control 439, trigger 827, and load 858
with zero spread and byte-identical complete traces. Larger repeated matrices
remain required as the observation and reset substrate evolves.

The first authenticated `movement-state/v2` offline pass re-featurized frames
440 through 827 of `run-001.gameplay.trace` from that exact matrix without
rerunning the game. It produced 388 aligned transitions with 98 features,
observation-spec/feature digest
`9c9055d7fc8d92b91191799fe5ad11cd620ea3f3acc420d44a7575b341e504dd`,
and corpus content digest
`188679959a26d9d78fc023a25c09e2304be0bdc95f2156ab5efb656ce10348e5`.
The matching canonical observation sidecar is independently inspectable, and
the learner accepts its authenticated categorical map without a manual schema
override.

This result closes the measured title-side scheduling leak, not every possible
PC-port side channel. New asynchronous loaders must either be made part of the
logical timing model or fail exact replay tests. Route frame golf runs the
immutable prefix unpaced for throughput, then repeats useful candidates from a
cold process-boot lineage as the promotion gate.

## Tunnel crawl frontier

The first exact downstream golf target is authored as
`tunnel_crawl_start`: post-simulation stage `F_SP104`, room 1, spawn 0, Link
present, and procedure `PROC_CRAWL_START` (53). It is intentionally the first
crawl-start frame rather than the later crawl-move state.

Two independent human recordings both reach it at absolute tape frame 860,
which is suffix-local frame 420 after the 440-frame `link_control` prefix. Each
was reproduced 3/3 from a clean process and trimmed inclusively to a 421-frame
segment. They remain sibling segments because their destination fingerprints
differ (`0aee9b9dfc150ff0c0b44c36408922a3` versus
`56b236160fbdff1ead7fed80441b7c99`); equal local time does not make their RNG
and downstream state interchangeable.

Mining this segment must prepend the immutable checked-in boot prefix and run
the complete tape from a clean process. A direct `--stage` launch is not valid
evidence for this objective. Candidate results are bound to the prefix tape,
authored milestone program and definitions, source fingerprint, and exact
source boundary before they can compete in the same population.

## Running and watching

```powershell
.\tools\glitch-hunting\run-intro-route.ps1 -Goal first-exit -Runs 10
.\tools\glitch-hunting\run-intro-route.ps1 -Goal intro-cutscene -Runs 10
```

In VS Code, run the single **Glitch Hunt: Route Workbench** launch. It displays
checked-in segments and pinned paths, plays a complete segment from its tree
entry, and hands live controller input back when playback ends. The fixed test
selector still exposes `intro-first-exit`, `intro-cutscene`, and
`fsp103-next-map-seed` for semantic regression runs.

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
