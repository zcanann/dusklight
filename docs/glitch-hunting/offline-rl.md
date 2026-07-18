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
- a v1/v2/v3/v4 gameplay-trace bridge with explicit post-simulation boundaries,
  typed channel presence, and exact input provenance; and
- a closed-loop fitted-Q proposal layer that returns ordinary deterministic
  candidates to the native milestone evaluator; and
- authenticated potential-based proposal shaping over distance, corridor,
  phase, and event-progress facts, with mandatory per-transition component
  reports.

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

Every trace extraction also writes `<batch>.evidence.json` with schema
`dusklight-transition-evidence/v1`. This sidecar is bound to the canonical
corpus content, source trace, and source tape by SHA-256. For every compact
transition it retains the exact post-simulation observation before the action,
the complete four-port tape frame (or a typed `OptionExecution` when an option
producer supplies one), realized duration, the post-action observation,
goal-predicate progress, reward components and their source facts, and an
optional typed terminal reason. Exact event and selected-actor snapshots live
in interned side tables; each transition stores only its pre/post indices, so
unchanged world facts are serialized once while the dense state vectors remain
in the compact corpus. Actor order, finite positions, bounded slots,
availability, observed count, and truncation are validated. Boundary indices,
simulation ticks, tape frames, reference kinds, and reference digests are
checked together, so a one-tick phase shift or detached sidecar is rejected.

Manual `--terminal` extraction records `declared_extraction_boundary`; it does
not invent objective proof from frame bounds. Anchored search can record
`objective_reached` because its terminal transition is already backed by the
native milestone verdict and boundary fingerprint. Older traces without the
goal-progress channel remain explicit `unrequested` predicate evidence rather
than silently acquiring zero-valued facts.

The corpus is a view, not the raw-data authority. Its evidence sidecar keeps
the source DUSKTRCE and DUSKTAPE digests, and both manual and anchored reports
retain their paths. Running `learn extract-trace` again with a different
authenticated view re-featurizes the same immutable observations. The trace
preserves integer RNG state, raw procedure/timer/animation facts, exact camera
and collision geometry, scene-exit destinations, KCL/DZB indices and raw code
words, actor session IDs, and placed-actor `(stage, type, home room, set ID)`
references; these are not reconstructed from normalized `f32` features.

Inspect or fit compatible batches with:

```powershell
huntctl learn inspect --input build/search/episode.dtcz
huntctl learn fit --input build/search/a.dtcz --input build/search/b.dtcz `
  --query-transition 0 --iterations 24 --trees 31 --seed 1 --all-continuous `
  --model-output build/search/q-model.json
```

The ranking reports mean Q, ensemble disagreement, and observed support for
each action. Disagreement is a sampling hint, not a calibrated probability.
The movement schema's categorical map is selected by its authenticated digest.
Other schemas must explicitly declare `--all-continuous` or repeat
`--categorical-feature N`; the learner never guesses category ordering.
`--model-output` serializes the complete fitted forest, authenticated feature
and action schemas, and training configuration, then stores the same bytes by
SHA-256.

### FQI support and uncertainty contract

The fitted-Q core accepts any authenticated, fixed-width finite feature schema
and discrete action schema represented by a compatible transition corpus. The
CLI has owned categorical maps only for `movement-state/v1` with movement
actions v2 and the authenticated `movement-state/v2` observation spec. An
unknown feature schema is supported only when the caller explicitly declares
every categorical index or declares all features continuous. Corpora with
different feature digest, action digest, or width are never merged.

Training is seeded and deterministic. `learn fit` and anchored Q proposal
training assign every input episode corpus a group and bootstrap complete
episode groups, stratified by action, for each tree. Thus correlated frames are
not presented as independent bootstrap evidence. The lower-level `FittedQ::fit`
row-bootstrap entry point remains available for synthetic/tiny batches;
`FittedQ::fit_with_episode_groups` is the evidence-bearing path. Model schema
`dusklight-fitted-q-model/v2` and the ranking report record the bootstrap unit,
episode count, seed, forest configuration, and exact input corpus identities.

Forest variance measures disagreement among these seeded bootstrap/randomized
trees. It is not a posterior variance, confidence interval, calibrated error
bar, or proof that an unsupported action is safe. Action support is reported
separately, and every proposed action still requires a new native rollout and
cold replay. FQI is currently a discrete offline proposer: it does not infer
counterfactual transitions, restore checkpoints, handle continuous actions
directly, or establish predicate feasibility.

`learn fit --n-step N` selects 1–64 observed semi-Markov transitions per
Bellman target. Rewards are accumulated only within the same episode group;
each option's declared simulation-tick duration contributes its exact
`gamma^duration` factor. An episodic terminal zeros continuation immediately.
At a nonterminal truncated end, the target may bootstrap from that final
observed `next_state` using the prior fitted iteration, but it never consumes a
reward or state from the next episode. Terminal records split multiple episodes
inside a compacted input corpus; a file that ends without a terminal closes one
truncated episode. The model artifact and ranking report record the selected
backup length. Focused tests cover cumulative multi-tick option discounting,
terminal zeroing, truncated-model continuation, cross-episode isolation, and
bounded CLI validation.

### Held-out value and proposal calibration

Training Bellman loss is not evaluation evidence. Fit the deterministic FQI
proposer on the dataset's `train` split and measure it against one untouched
split with:

```text
huntctl learn calibrate --dataset build/dataset.json --split test \
  --output build/q-calibration.json --iterations 24 --trees 31
```

For an ad hoc content-disjoint check, repeat `--training TRAIN.dtcz` and
`--held-out TEST.dtcz` instead. The command rejects overlapping paths or corpus
digests, verifies dataset corpus content against its manifest, preserves
terminal-delimited episode groups, and derives duration-discounted simulator
return-to-go only from held-out rewards.

The versioned report includes signed error, MAE, RMSE, equal-frequency
prediction bins, and an exact-state proposal win rate. A proposal is comparable
only when its selected action was actually observed at that exact held-out
state; unsupported observed actions, unsupported proposals, and observed
regret remain separate diagnostics instead of being silently counted as wins
or losses. These numbers evaluate ranking quality but never replace a native
predicate hit or cold-replay proof.

### Fixed masked representation baseline

`learning/model_representation.rs` defines the first bounded input layout for
larger value and policy models. It is fitted only from the exact training-state
set described by `dusklight-normalization/v1`; continuous fields use those
training-only means and standard deviations, while categorical fields use
bounded deterministic embedding tables with an explicit unknown category.
Every output value has a parallel missingness bit derived from the authenticated
`movement-state/v2` mask policy.

The same fixed-width tensor appends the 64-value compiled objective vector, four
nearest semantic actor slots, and four nearest local geometry probes. Actor and
surface ordering is distance-first with stable identity tie-breaks, positions
are player-local and normalized, absent slots are zero-masked, and channel/query
availability remains explicit. Geometry probes are derived from authenticated
world point-query results rather than copying an inventory or BVH into each
sample. The complete normalization, category tables, widths, and ordering rules
produce one representation digest; reordered source actors/surfaces encode to
the same tensor.

`learning/history_critics.rs` tests whether that fixed representation still
aliases different return targets at byte-identical current states. It compares
a single-frame ridge critic, an episode-boundary-masked short stack, and a
deterministic recurrent-reservoir critic on content-disjoint episodes with the
same training and held-out row budgets. The authenticated report binds the
representation digest, aliasing counts, errors, configuration, and disposition;
it has no promotion authority. Controlled hidden-cue tests show both temporal
forms beating the current-state critic, while a fully observed fixture retains
the simpler baseline. No route adopts history or recurrence until an actual
held-out Trace-v2 corpus produces the same evidence.

Variable actor-set encoders are separately gated by
`dusklight-actor-set-readiness/v1`. The default gate requires content-disjoint
evaluation, at least 128 episodes and 4,096 effective decisions, at least 256
decisions that overflow the fixed slots, fixed-slot held-out MSE above its
declared ceiling, and overflow-conditioned error at least 1.25 times the global
error. Until all conditions hold, `ActorSetEncoder` cannot be constructed.
Qualified comparisons expose deterministic DeepSets summary features and
objective-query attention, both canonicalized so actor enumeration order cannot
change bytes or digests. The readiness artifact is non-promotional; native
held-out comparison still decides whether either encoder replaces fixed slots.

### Deterministic discrete Double-Q baseline

Train the bounded twin-critic baseline on the same immutable training split:

```text
huntctl learn double-q --dataset build/dataset.json \
  --model-output build/double-q-model.json --epochs 64 \
  --hidden-width 32 --target-sync-steps 256 --seed 1
```

The learner uses two independently initialized one-hidden-layer critics. On
alternating updates, the online critic selects the next discrete action and the
opposite critic's frozen target copy evaluates it. Target copies synchronize
only after the declared number of gradient updates. Semi-Markov transitions
use exact `discount^duration`; terminal transitions never bootstrap. Training
order, initialization, target synchronization, and artifacts are deterministic
under the recorded seed and corpus identities.

Training-only numeric mean/variance normalization and bounded gradients keep
this intentionally small baseline inspectable. The ranking artifact reports
both critics, their disagreement, observed action support, update count, and
target synchronization count. It also records that categorical embeddings,
calibrated uncertainty, and conservative OOD penalties are not yet provided;
Double-Q rankings remain native-rollout proposals, never promotion evidence.

### Discrete Conservative Q-Learning

Use the same immutable input path with a state-local discrete CQL penalty:

```text
huntctl learn cql --dataset build/dataset.json \
  --conservative-weight 1.0 --temperature 1.0 --seed 1 \
  --model-output build/cql-model.json
```

The twin critics retain Double-Q target selection and frozen target copies.
Every observed transition additionally minimizes
`alpha * (T * logsumexp(Q(s, all actions) / T) - Q(s, observed action))`.
This pushes down actions not represented at a sampled state while preserving
the globally authenticated discrete action set. Weight and temperature must be
finite and within `(0, 100]`; all Double-Q work and model-size bounds still
apply.

Schema `dusklight-conservative-q-model/v1` records the full critic pair,
training identities, base optimizer configuration, conservative weight, and
temperature. The ranking report includes observed global action support,
conservative update count, and the mean post-training conservative gap. That
gap is an objective diagnostic, not a calibrated OOD probability. CQL reduces
unsupported-action optimism but does not establish that an action is safe,
valid, or feasible; native rollout and cold replay remain mandatory.

### Implicit Q-Learning and advantage-weighted cloning

Train the dataset-constrained policy alternative with:

```text
huntctl learn iql --dataset build/dataset.json \
  --expectile 0.7 --advantage-beta 3 --max-advantage-weight 100 \
  --model-output build/iql-model.json --seed 1
```

Two Q critics learn only logged actions from duration-discounted targets. A
separate value network fits the upper expectile of the smaller target critic,
and the policy performs behavior cloning only on the logged action label,
weighted by `exp(beta * (min(Q1,Q2) - V))`. The weight is bounded before the
policy update. No maximization over unobserved actions enters the Q target or
creates a synthetic policy label.

The authenticated artifact records critic, value, and policy networks plus the
expectile, inverse temperature, weight cap, target synchronization, optimizer,
seed, dataset, and corpus identities. Rankings are ordered by the learned
behavior-policy probability and report Q, V, advantage, critic disagreement,
global support, mean weight, and clipped-weight count. Function approximation
can still generalize across states, and neither probabilities nor disagreement
are safety estimates; every proposal retains the native proof gates.

### Episode-bootstrapped twin-critic ensembles

Train several seeded Double-Q members on whole-episode bootstrap draws with:

```text
huntctl learn ensemble-q --dataset build/dataset.json --members 7 \
  --model-output build/ensemble-q-model.json --seed 1
```

Each member contains two critics and records the exact ordered episode-group
IDs drawn with replacement. If a draw omits a globally supported action, the
repair path appends an entire episode containing that action; it never injects
an isolated transition. Ensemble size, expanded transition count, and total
gradient work are bounded. Initialization, episode draws, support repairs, and
member training are deterministic under the recorded ensemble and critic
seeds.

Rankings report the mean Q across members, between-member variance, mean
within-member twin disagreement, and observed global support. Both uncertainty
numbers are uncalibrated sampling diagnostics. The artifact contains every
member and draw manifest, and remains proposal evidence subject to native
rollout and cold replay.

### Bounded prioritized replay

Train the twin critic with deterministic TD-error prioritization using:

```text
huntctl learn prioritized-q --dataset build/dataset.json \
  --priority-alpha 0.6 --importance-beta-start 0.4 \
  --importance-beta-end 1 --importance-weight-cap 1 \
  --model-output build/prioritized-q-model.json --replay-seed 1
```

The sampler maintains an online Fenwick tree over `(absolute TD error +
epsilon)^alpha`. Its seeded draw stream is deterministic, beta anneals over
the bounded training budget, and importance weights scale each critic update.
The explicit weight cap prevents rare rows from producing unbounded updates;
because clipping introduces bias, both the cap and clipped-sample count are
recorded rather than treated as an exact correction.

The report includes total and per-row sample counts, unique rows sampled,
effective sample size, final priority range, mean and maximum importance
weight, clipped-weight count, and final beta. Priorities indicate where the
current critic has error; they are neither calibrated uncertainty nor proof of
an action's validity. The model remains a proposal artifact subject to the
same native predicate and cold-replay gates.

### Controlled Rainbow-component ablations

`learn ablate-q` compares exactly one experimental component against the
deterministic Double-Q baseline on content-disjoint held-out corpora. Supported
treatments are `dueling-heads`, `n-step`, `distributional-values`, and
`noisy-exploration`; the command has no syntax for combining them into a
Rainbow configuration.

```sh
huntctl learn ablate-q --component distributional-values \
  --dataset build/dataset.json --split test \
  --output build/ablation/distributional.json
```

The report authenticates both corpus sets, rejects overlapping files or
digests, and records held-out Bellman error, observed simulator return MAE and
RMSE, calibration bins, exact-state proposal win rate and regret, unsupported
held-out actions, logged-action agreement, gradient updates, and
component-specific diagnostics. Baseline and treatment must have the same
gradient-update budget. No component is adopted automatically: these metrics
are not native objective success, and every proposal still requires
equal-budget native evaluation and cold replay proof.

The experimental implementations are isolated under `learning/double_q/` and
do not alter the production learner. Dueling heads factor value and centered
advantages; n-step returns preserve terminal versus truncated episode ends;
categorical values use a bounded projected support; and noisy exploration uses
learned factorized parameter noise during training while deterministic mean
weights are used for held-out ranking.

The compact checked-in tests prove deterministic mechanics, schema behavior,
budget equality, and OOD accounting only. They are not evidence for adopting a
component. Run all four treatments separately on a frozen corpus that meets the
RL readiness gates before considering any combined Rainbow configuration.

### Semi-Markov option values

Learn the high-level choice between realized options before attempting any
per-frame neural controller. An option-value batch authenticates its feature
schema, objective, complete typed option catalog, episode groups, duration-aware
returns, and the exact raw tape digest emitted by every realization:

```sh
huntctl learn option-values --input build/learning/options.json \
  --model-output build/learning/option-values.json \
  --query-sample 0 --iterations 24 --trees 31 --seed 1
```

The model ranks `OptionActionDescriptor` values, including their option type
and typed parameters. It deliberately exposes no raw-PAD ranking API. Selection
therefore ends at the option boundary, the chosen option is realized into a
deterministic tape, and raw frame edits are reserved for downstream last-mile
tape golf. Each sample already represents one semi-Markov transition, so the
command fixes FQI backup length to one while discounting by its simulation-tick
duration. The resulting model and ranking are proposal artifacts only; they
cannot promote a route without native evaluation and cold replay proof.

`learning::factorized_actions` provides the low-data action representation used
before considering a larger neural policy. Every candidate explicitly separates
its tactic, optional heading, optional magnitude, duration, intended target,
and 16-bit button overlay. The authenticated encoder assigns independent
feature blocks to those axes: tactic and target use bounded canonical catalogs,
heading uses presence/sine/cosine, magnitude and duration are normalized
numeric features, and overlay buttons are individual bits.

The accompanying ridge critic is intentionally small and additive. Evidence
for (for example) a roll tactic, a heading, and an actor target can therefore
contribute to a new combination even when that full Cartesian action was never
observed. A focused regression test fits six one-factor variations and ranks
their unseen combined action. Unknown categorical tactic/target factors are
rejected instead of silently mapped to an "other" bucket; this representation
is a low-data proposal baseline, not promotion evidence or a claim that action
factors are independent in the game.

Goal conditioning uses `dusklight-compiled-objective-vector/v1`, never a route
segment label or an unchecked objective name. The encoder re-decodes canonical
compiled DMSP bytes and rejects any program/definition identity mismatch. Its
fixed 64-value vector includes the exact definition digest plus bounded phase,
stability, sequence, projection, expression, query, comparison, and value-type
structure. Editing predicate semantics therefore changes the vector and its
authenticated identity.

`GoalConditionedInputEncoder` defines one reusable model boundary. Policy input
is `state + compiled objective`; value input is `state + compiled objective +
factorized option action`. The serialized layout authenticates the state schema,
factor schema, widths, and block order. This lets one model consume objectives
from multiple route segments without making an objective string a categorical
shortcut. It supplies model inputs, not evidence that a proposed action reaches
the selected objective; native predicate evaluation and cold replay remain the
promotion gates.

Hindsight relabeling is behind the compiled-predicate semantic gate
`dusklight-hindsight-relabel-decision/v1`. The only admitted class is a
single-snapshot, post-simulation predicate with one-tick stability. Pre-input
predicates, multi-tick stability, ordered sequences/windows, value projections,
and tape-frame or boundary-position comparisons are rejected with typed reasons.
Actor, flag, region, plane, and ordinary state-field predicates remain eligible
when they satisfy those history-free constraints. Admission also fixes
`copied_reward_allowed` to false: reward and shaping must be recomputed from the
authenticated pre/post observations under the relabeled compiled predicate.
The relabeling API enforces that policy: evidence carries the compiled program,
definition and objective-vector identities; exact pre/post observation and
compact-state digests; and the realized raw-tape digest. It accepts only a
native false-to-true predicate result, rejects already-satisfied or detached
transitions, retains the original reward for audit, and writes a separately
configured finite achievement reward while setting `reward_recomputed: true`.
Relabeled samples always retain `promotion_authority: false`.

`select_and_execute` closes the high-level/low-level hierarchy without giving
the learner raw-frame authority. `TacticOptionCandidate::new` derives each typed
descriptor from its deterministic `GameTacticPlan`, so the action catalog and
executor cannot be authored independently. Selection fails unless the model's
entire option catalog has a unique exact executor. It then takes the top option,
appends the plan's deterministic frames to the canonical prefix, and captures
the realized range as an `OptionExecution`.

An executed policy step exists only when the selected ID, option type, and
complete parameter map equal the realized descriptor and that execution
validates against the complete output tape. A missing executor, different
tactic or parameter set, emitted-frame change, or unrelated prefix-tape change
invalidates the proof. The serialized step includes the selected ranking, full
tape, execution, and explicit descriptor/frame checks; it still has no
promotion authority until the surrounding native objective and cold-replay
gates pass.

### Nearest-neighbor and tabular return baselines

For small objective-specific state spaces, compare FQI against empirical
nonparametric baselines:

```text
huntctl learn baseline --method nearest-neighbor --input episode-a.dtcz \
  --input episode-b.dtcz --neighbors 8 \
  --feature 17:0.03125:continuous --feature 16:1:categorical

huntctl learn baseline --method tabular --input episode-a.dtcz \
  --axis 17:0:0.03125 --axis 16:0:1
```

Both methods first calculate duration-discounted observed return-to-go within
whole input episode groups. A terminal transition zeros continuation. The end
of a nonterminal/truncated episode also stops return accumulation; it never
bootstraps from the next corpus. Nearest-neighbor ranking uses a caller-declared
scaled distance with exact mismatch distance for categorical features, selects
at most 256 same-action neighbors, and reports support plus nearest squared
distance. The two built-in movement schemas provide their authenticated
categorical maps when no feature list is supplied; unknown schemas require an
explicit feature declaration.

The tabular baseline accepts at most eight declared quantization axes and
100,000 observed `(cell, action)` entries. It averages returns only for actions
observed in the query's exact cell. An unseen cell returns no ranking rather
than borrowing a neighbor or inventing zero support. Schema
`dusklight-low-data-baseline/v1` records input corpora, episode count, discount,
query, configuration, support, and the explicit limitation that these are
observed-return proposal heuristics requiring native rollout proof.

## Potential-based proposal shaping

`learn fit` can derive a denser proposal signal without changing the terminal
predicate or any leaderboard score:

```powershell
huntctl learn fit --input build/search/a.dtcz --all-continuous `
  --discount 0.995 --shaping build/search/shaping.json `
  --shaping-report build/search/reward-components.json
```

The shaping file uses schema `dusklight-potential-shaping/v1` and names the
exact feature-schema SHA-256 digest whose indices and units it interprets. A
mismatch is rejected. Its bounded term list supports four explicit potentials:

```json
{
  "schema": "dusklight-potential-shaping/v1",
  "feature_schema": "FEATURE_SCHEMA_SHA256",
  "terms": [
    {
      "kind": "distance", "name": "exit-distance", "feature": 46,
      "goal": 0.0, "scale": 1.0, "weight": 1.0,
      "unavailable_value": -0.0001220703125
    },
    {
      "kind": "corridor_progress", "name": "tunnel", "feature": 17,
      "start": 0.0, "end": 1.0, "weight": 2.0
    },
    {
      "kind": "phase_progress", "name": "load-phase", "feature": 8,
      "ordered_values": [0.0, 1.0, 2.0], "weight": 1.0
    },
    {
      "kind": "event_progress", "name": "event-step", "feature": 41,
      "ordered_values": [0.0, 1.0, 2.0, 3.0], "weight": 1.0
    }
  ]
}
```

Distance is negative absolute distance from `goal`, normalized by `scale`.
Corridor progress is clamped from `start` to `end`, including decreasing
corridors. Phase and event terms accept only their exact declared ordered
values; an unlisted or explicitly unavailable value is an error rather than an
invented progress value.

For a transition of `d` simulation ticks the learner receives
`base_reward + gamma^d * Phi(next) - Phi(source)`. The effective next potential
is forced to zero at an episodic terminal boundary, so discounted shaping
telescopes to a start-state constant. The external terminal predicate remains
the sole feasibility authority, and shaping is never read by
`LexicographicScore`.

`--shaping` is accepted only together with a new `--shaping-report` path. The
versioned report authenticates the shaping spec and records, for every input
transition and named term, the feature index, source and next fact, both
potentials, terminal adjustment, component reward, base reward, and final
training reward. This makes unavailable facts and sign/scale mistakes directly
inspectable.

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

Manual imports also require an explicit `dusklight-episode-context/v1` file.
The executable and objective digests must be the real SHA-256 values for the
run; huntctl rejects zero or omitted identities instead of manufacturing
`unknown` provenance. A minimal manually sourced context is:

```json
{
  "schema": "dusklight-episode-context/v1",
  "run_build": {
    "executable_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  },
  "objective": {
    "id": "fsp103-exit",
    "digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  },
  "producer": {
    "kind": "manual_import",
    "name": "huntctl",
    "version": "0.1.0"
  },
  "seed": { "kind": "not_applicable" },
  "worker_id": "manual-workstation",
  "lineage": { "generation": 0 },
  "outcome": { "class": "successful", "reason": "objective proof supplied" }
}
```

```powershell
huntctl learn extract-trace `
  --trace build/run.gameplay.trace --tape build/run.tape `
  --episode-context build/run.episode-context.json `
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

Every extraction also writes `<batch>.episode.json` (or `episode.json` in an
anchored attempt). It binds scenario and full stage fixture, first parent
boundary, absolute tape, executable build, query and action schemas, objective,
producer, deterministic seed, worker, candidate lineage, structured absolute
frame intervention, terminal outcome, corpus, trace, and evidence. Its
`input_identity_sha256` deliberately excludes worker and realized output so
independent repetitions group together; `episode_sha256` includes those facts
so distinct runs remain distinct evidence.

Anchored evaluations additionally write `episodes.json`. It groups by input
identity, collapses byte-identical episode identities to an occurrence count,
and retains each independently hashed `attempt.json` as repetition evidence.
Deduplication therefore reduces storage/index noise without converting repeated
runs—or correlated frames inside one run—into extra independent samples.

Online anchored search also writes `evaluation-generation-seal.json` before it
exposes any newly collected corpus to a learner. The seal authenticates the
evaluation generation, complete planned/completed attempt count, evaluation-only
worker identities, and the exact corpus digest retained from attempt 1 of each
candidate. It is rejected if an attempt is missing, an infrastructure fault is
present, or a later proof repetition carries a corpus. The admitted digests have
`minimum_training_generation = evaluation_generation + 1`; consequently the
current evaluation cannot train on its own observations, and no proof repetition
can become a training episode.

Large immutable outputs use `dusklight-content-blob/v1` references. Gameplay
traces are streamed into `content/blobs/sha256/<prefix>/<suffix>` and every
attempt reports the digest, exact byte count, media type, and relative blob
path. Non-successful attempts preserve nonempty stdout, stderr, partial trace,
and milestone files as deduplicated crash artifacts. Manual extraction accepts
`--artifact-store ROOT`; otherwise its source trace is stored under the output
directory's `content/` store. World inventories and spatial indices use the
same store and optional argument. Route Workbench keeps semantic thumbnail
cache keys for stable URLs while also mirroring every valid PNG by image bytes.

Example using an explicitly selected successful window:

```powershell
huntctl learn extract-trace `
  --trace build/test-results/run.gameplay.trace `
  --tape build/intro-first-exit.tape `
  --episode-context build/test-results/run.episode-context.json `
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
repeat-proved elites and diverse behavior-archive routes, and allocates the
bounded non-elite budget across five named lanes: action-mask-guided mean-Q
exploitation over observed actions, ensemble-disagreement probes, structured
least-supported counterfactuals, rare behavior-context archive probes, and
blind coverage that interleaves seeded random with Latin-hypercube probes.
Equal shares are used when possible; remainder shares rotate by generation, so
small budgets do not permanently starve the same lane. Available lanes then
emit in generation-rotated round-robin order rather than proposer-sized blocks.
The report retains that exact `collection_schedule`, making the alternation
auditable even when duplicate candidates or an empty learned lane are skipped.
Proposed one-, two-, and four-frame action windows become normal candidates and
cannot bypass cold replay, predicate proof, or determinism checks. Ensemble
disagreement is recorded as a sampling heuristic, never a calibrated confidence
estimate.

Online fitted-Q passes `dusklight-online-training-health/v1` before any learned
lane can emit a candidate. A fitted Bellman pass counts one update per data row,
and the update-to-data ratio is capped at 32 with an early rejection before an
oversized fit. After fitting, a bounded, evenly strided state sample is checked
across every trained action for non-finite estimates, absolute-value explosion,
and excessive ensemble disagreement. The report records update count, ratio,
snapshot count, maxima, configured limits, and the exact healthy or rejected
disposition. This is a numerical circuit breaker, not a calibration claim;
native evaluation remains mandatory.

Before each online refit, anchored search writes
`online-dataset-generation.json`. Its content identity covers the cumulative
corpus digests and schemas, the prior dataset-generation identity, and the
evaluation seal that admitted the current delta. A generation is accepted only
when it is exactly its immutable parent union those sealed corpora; dropped,
silently added, or modified corpora fail validation.

The learner deliberately resumes by deterministic full refit over that exact
cumulative generation, rather than claiming an incremental weight warm start.
When a fitted model is available, `online-model-lineage.json` binds its
serialized bytes to the dataset-generation digest, exact FQI configuration,
model schema, and previous model/lineage digests. The same immutable inputs must
reproduce the same model lineage; a changed dataset, config, or model fails the
resume check. `q-proposals.json` embeds both identities.

Learned lanes additionally require `dusklight-online-coverage-gate/v1` to pass.
The default gate requires at least four effective decisions spanning four
semantic state bins, with at least two actions supported by two or more
decisions each. If action support, state coverage, or both are inadequate, no Q
model is fitted and exploit/disagreement receive zero budget. Their budget is
rotated across structured counterfactual, archive-novelty, and blind-coverage
fallbacks instead. The v7 proposal report records the measured counts, limits,
fallback reason and policy, so sparse data cannot silently produce a learned
proposal claim.

The generation-local `q-proposals.json` reports requested, available, and
generated counts per proposer, the generation's cycle offset and actual lane
schedule, plus coverage by stage/room, spatial cell, player procedure, option,
parameter/action bin, duration, goal phase, terminal outcome, and observed
action support. Its collapse audit reports unique parents and
proposed actions. This makes failures and blind-spot probes first-class
collection evidence rather than repeatedly perturbing only successful
headings.

## Dataset splits and withheld evaluation

Every manual or anchored extraction emits a `dusklight-dataset-source/v1`
descriptor pointing at its verified episode manifest, corpus, absolute tape,
and transition evidence. Build an immutable dataset with:

```powershell
huntctl learn dataset --source build/a.dataset-source.json `
  --source build/b.dataset-source.json --output build/dataset.json `
  --withheld-objective frozen-route-benchmark
huntctl learn fit --dataset build/dataset.json `
  --model-output build/q-model.json
```

The splitter unions related episodes before assigning a split. Scenario,
parent boundary, route family, exact tape, tape-prefix relationships,
checkpoint or screenshot digests, and candidate/continuation ancestry can
therefore never cross train, validation, test, or frozen-withheld boundaries.
Withheld objective components receive a separate stable digest and `learn fit`
loads only `train` corpus entries, so model selection cannot consume the frozen
suite.

`dusklight-dataset-manifest/v1` reports unique episodes and inputs, effective
decisions, action support, quantized state coverage, explicit
present/absent/unavailable/truncated/unrequested evidence counts, outcome
imbalance, and parent-boundary diversity. Versioned means and standard
deviations list the exact training episode identities and are computed from
training states only. The content-addressed dataset and fitted-model artifacts
bind the ordered corpus digests, dataset digest, schemas, and complete learner
configuration.

For a successful/failing sibling pair, run:

```powershell
huntctl learn diff-episodes --success-trace success.trace `
  --failure-trace failure.trace --success-evidence success.evidence.json `
  --failure-evidence failure.evidence.json --output sibling-diff.json
```

The typed report identifies the first different boundary for phase, event,
selected actors, collision/contact state, core flags, RNG state/draw counts,
selected-actor process population, and objective/reward components. Domain
coverage is complete, partial, or unavailable; current traces explicitly note
that selected actors are not full heap-allocation evidence.

## Corpus lifecycle operations

Transition batches can be inspected and transformed without weakening their
schema or content identities:

```powershell
huntctl corpus query --input a.dtcz --action 7 --minimum-reward 0
huntctl corpus compare --left a.dtcz --right b.dtcz
huntctl corpus merge --input a.dtcz --input b.dtcz --output merged.dtcz
huntctl corpus compact --input merged.dtcz --output compact.dtcz
huntctl corpus shard --input compact.dtcz --output-directory shards `
  --maximum-transitions 100000
huntctl corpus validate-transitions --input compact.dtcz
```

Merge and compact reject incompatible feature/action schemas and deduplicate by
the complete authenticated transition value. Sharding preserves schemas and
transition order. `corpus refeature --source episode.dataset-source.json`
replays immutable trace/tape evidence through a named observation view and
emits a new corpus, evidence bundle, episode manifest, and dataset-source
descriptor; it does not attempt to invert an older feature vector. Invalid
batches can be previewed and then moved, never overwritten, with `corpus
quarantine --quarantine-root DIR [--apply]`.

Large-artifact collection is also non-destructive by default:

```powershell
huntctl corpus gc-content --store build/content `
  --manifest build/dataset.json --manifest build/q-model.json `
  --trash-root build/recoverable-trash
# Review the JSON report, then repeat with --apply.
```

Every 64-hex identity referenced by a supplied JSON root is retained, as is the
root document's own byte digest; exact roots can also be supplied with repeated
`--reference SHA256`. GC verifies every live blob before classification,
reports referenced-but-missing identities, refuses trash inside the live blob
tree, and moves unreachable blobs to the same digest-shaped path under the
explicit trash root. It never permanently deletes user route data.

For notebooks, DuckDB, Polars, or other analysis tools, export a bounded Apache
Arrow IPC file outside the collection/training hot path:

```powershell
huntctl corpus export-arrow --input compact.dtcz `
  --output build/analysis/transitions.arrow
```

The typed table retains source-corpus identity, transition index, state and
next-state fixed-size vectors, reference kinds/digests, action ID/family and
signed parameter list, duration, reward, and terminal status. Arrow schema
metadata binds the feature/action digests and explicitly sets
`dusklight.replay_authority=false`. A `dusklight-analysis-export/v1` sidecar
binds every authoritative input digest and the resulting Arrow-file digest.
The export is capped at 64 compatible corpora and one million rows; it is never
accepted as tape, replay, episode, or learner-corpus authority.

A bounded MAP-Elites archive prevents fastest-first selection from erasing every
different route or terminal state. Archive schema v3 retains one
native-quality elite per coarse map, procedure sequence, deduplicated route-bin
sequence, position, and exit-distance descriptor. It additionally binds
available named RNG and actor-population projections, the portable
run-deduplicated collision/contact trajectory, terminal boundary fingerprints,
and complete downstream boundary/value state. A distinct RNG, actor
population, contact path, route, procedure, boundary, or downstream digest
therefore occupies a separate cell rather than silently replacing another
behavior. Farthest-first novelty selection reserves a small population budget
for cells farthest from the current elites. Those retained episodes are also
eligible Q parents. `behavior-archive.json` records the policy, selected
candidate IDs, and exact descriptors.

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
