# Native offline reinforcement learning

Dusklight has a deliberately small fitted-Q path for finite, memory-backed
transition batches. It is not an end-to-end pixel DQN and does not own game
processes. Rust workers and proof tooling remain responsible for execution;
the learner consumes immutable observations and ranks discrete input actions.

## Current primitives

`huntctl` now provides:

- a canonical little-endian transition format with zstd storage, SHA-256
  integrity, bounded allocation, and authenticated feature/action schema IDs;
- deterministic fitted Q iteration using action-specific randomized regression
  forests, with equality splits for schema-declared categorical features;
- duration-aware Bellman targets and per-action ensemble variance;
- hard bounds on corpus fan-in, transitions, actions, iterations, trees, and
  depth so malformed batches cannot create unbounded training work;
- a fixed eight-transition shortest-path benchmark queried at held-out feature
  vectors; and
- an exploratory gameplay-trace bridge with exact post-tick alignment.

Run the implementation benchmark directly:

```powershell
cargo run --manifest-path tools/huntctl/Cargo.toml -- learn benchmark
```

Its pass condition is structural: both held-out states must select the known
shortest-path action. Seeded fitting and ranking are reproducible.

## Compact transition batches

The `.dtcz` file is a compressed binary artifact, not a JSON tape substitute.
Every batch declares fixed feature width plus SHA-256 identities for its exact
feature and action layouts. Merging mismatched batches is rejected. Each
transition stores:

- source and destination boundary/snapshot references;
- fixed-width finite `f32` state vectors;
- a discrete action ID and compact macro parameters;
- simulation-tick duration, reward, next state, and terminal flag.

Inspect or fit compatible batches with:

```powershell
huntctl learn inspect --input build/search/episode.dtcz
huntctl learn fit --input build/search/a.dtcz --input build/search/b.dtcz `
  --query-transition 0 --iterations 24 --trees 31 --seed 1 --all-continuous
```

The ranking reports mean Q, ensemble disagreement, and observed support for
each action. Disagreement is a sampling hint, not a calibrated probability.
The movement schema's categorical map is selected by its authenticated digest.
Other schemas must explicitly declare `--all-continuous` or repeat
`--categorical-feature N`; the learner never guesses category ordering.

## Exploratory trace extraction

Gameplay trace records are post-simulation observations. The correct primitive
transition for action frame `i` is:

```text
trace[i - 1] -- tape[i] --> trace[i]
```

The bridge enforces that relationship and accepts only absolute 30 Hz tapes.
It rejects reactive waits, discontinuous trace ticks, unsupported controller
fields, non-catalog stick vectors, tape/trace input mismatches, implicit
terminal state, exhausted traces, and missing episode identity. Stick matching
uses Aurora's exact integer `PADClamp` transform because the tape stores raw
values while the trace observes post-clamp input.

Reactive-controller trace records carry distinct provenance and are rejected
by this bridge. First record the run with `--realized-input-tape`, replay that
absolute tape, and extract transitions from the replay trace. This prevents an
observation-feedback policy from being mistaken for a self-contained action
sequence.

The v2 movement action catalog has 68 classes: four button states (none, A, B,
and A+B) crossed with neutral or 16 nearest post-`PADClamp` headings. Exact raw
stick coordinates and buttons remain attached as compact action parameters, so
human curves are not rounded out of the corpus and a proposal layer can reuse
or perturb the observed sample. The 49-field observed-state vector includes
stage, room, player procedure, position, velocity, facing, prior applied input,
event state, nearest-exit diagnostics, and finite-horizon time.

Anchored native farming retains a gameplay trace for the first proof repetition
of every candidate. After milestone validation, the evaluator automatically
extracts the source-to-terminal window into `transitions.dtcz`, pins its source
and successful terminal references to the native boundary evidence, and records
both paths and transition count in `evaluation.json`. Later identical proof
repetitions do not duplicate the learning episode. Trace or extraction failure
is explicit learning metadata but cannot turn valid milestone evidence into a
different gameplay result.

Example using an explicitly selected successful window:

```powershell
huntctl learn extract-trace `
  --trace build/test-results/run.gameplay.trace `
  --tape build/intro-first-exit.tape `
  --start-frame 440 --end-frame 827 --terminal `
  --output build/search/intro-first-exit.dtcz
```

One checked real route run produced 139 transitions from the 138-tick Ordon
Spring incumbent and fitted the forest-Q learner directly from the evaluator
artifact. This proves the automatic extraction and fitting path; a single
successful behavior trace still contains no evidence about counterfactual
actions.

## Promotion boundary

Trace v1 omits per-tick RNG, collision contacts, ground/wall polygons, camera
state, and several Link action timers. Explicit frame bounds are also weaker
than native milestone proof. Extracted batches are therefore labeled
non-authoritative and must not promote a learned route.

The next promotion gates are:

1. add gameplay trace v2 fields needed for a credible movement state;
2. collect whole-episode perturbed tapes across all supported actions;
3. split train/validation by episode and boundary fingerprint, never by frame;
4. use fitted Q and archive novelty to propose tapes; and
5. require exhaustive local golf plus repeated cold replay before promotion.

Snapshots and persistent engine sessions improve sample throughput, but they do
not change these evidence requirements.
