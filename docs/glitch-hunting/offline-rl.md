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
- a v1/v2 gameplay-trace bridge with explicit post-simulation boundaries,
  typed channel presence, and exact input provenance; and
- a closed-loop fitted-Q proposal layer that returns ordinary deterministic
  candidates to the native milestone evaluator.

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
It rejects pre-input or contradictory boundaries, missing/unavailable required
channels, reactive/controller provenance, discontinuous trace ticks,
unsupported controller fields, non-catalog stick vectors, tape/trace input
mismatches, implicit terminal state, exhausted traces, and missing episode
identity. Stick matching uses Aurora's exact integer `PADClamp` transform
because the tape stores raw values while the trace observes post-clamp input.

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

### Authenticated movement-state/v2 view

`movement-state/v2` is a canonical objective-specific observation artifact,
not another undocumented fixed vector. Its serialized specification includes
the F_SP103-to-F_SP104 objective and target tuple, post-simulation phase, exact
Trace v2 channel versions and strides, per-channel status policy, and 98
ordered features with stable field IDs, types, units, coordinate spaces,
transforms, categorical flags, and missingness rules. Changing the objective,
target, channel contract, or any feature metadata changes the SHA-256 feature
schema stored by the transition corpus.

The view consumes stage and pending-transition state, Link motion/procedure,
applied input, event state, exact scene-exit geometry, collision correction,
cached ground exit metadata, both global RNG streams, camera state, and Link
action timers. Semantic absence has a separate mask and zero payload: no scene
exit, no cached ground identity, and a genuinely unavailable event-name hash
cannot alias a present zero-valued fact. `Unavailable`, `Truncated`, and
`NotSampled` required channels remain extraction errors.

Emit or inspect the exact specification with:

```powershell
huntctl observe spec movement-state/v2 --output build/movement-state-v2.json
huntctl observe inspect build/movement-state-v2.json
```

Extracting this view writes the compact transition corpus plus a canonical
`.observation.json` sidecar whose digest is the corpus feature-schema digest:

```powershell
huntctl learn extract-trace `
  --trace build/run.gameplay.trace --tape build/run.tape `
  --start-frame 440 --end-frame 827 --terminal `
  --view movement-state/v2 --output build/run-v2.dtcz
```

The v1 bridge and digest remain unchanged for old corpora. Q search is not
silently migrated: a v2 corpus is a distinct authenticated feature space, and
the learner derives its categorical map from the matching specification.

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

Anchored search now accumulates content-deduplicated episode corpora across
generations. It fits Q on compatible batches, scores tape-aligned states from
repeat-proved elites, and alternates mean-Q exploitation with
ensemble-disagreement exploration. Proposed one-, two-, and four-frame action
windows become normal candidates and cannot bypass cold replay, predicate
proof, or determinism checks. A generation-local `q-proposals.json` makes the
sample budget and proposal ancestry inspectable.

A bounded behavior archive prevents fastest-first selection from erasing every
different route. It retains one best episode per coarse map, procedure, path,
position, and exit-distance descriptor, then reserves a small population budget
for entries farthest from the current elites. Those retained episodes are also
eligible Q parents. `behavior-archive.json` records the selected candidate IDs
and exact descriptors.

## Promotion boundary

Trace v2 adds an explicit channel directory/status stream, four-port applied
PAD, current/pending stage, both global RNG streams, realized camera state,
full Link motion/procedure context, timers, six animation lanes, cached
background collision/correction, and resolved collision-exit surfaces. It
remains non-Markov: actor/push/attack contacts and broad local geometry are
absent, RNG coverage is incomplete, and process/build identity is not yet
embedded. `movement-state/v1` intentionally retains its legacy event-name-hash
requirement; `movement-state/v2` represents the hash's availability explicitly
under a different authenticated schema.

Explicit frame bounds are also weaker than native milestone proof. Extracted
batches remain non-authoritative and cannot promote a learned route.

The next promotion gates are:

1. add actor/push/attack contacts and broader local geometry to a successor
   observation view without changing the v2 digest;
2. collect whole-episode perturbed tapes across all supported actions;
3. split train/validation by episode and boundary fingerprint, never by frame;
4. add larger temporal options such as waypoint-seek and deterministic
   roll-spacing policies to the learned action hierarchy; and
5. require exhaustive local golf plus repeated cold replay before promotion.

Snapshots and persistent engine sessions improve sample throughput, but they do
not change these evidence requirements.
