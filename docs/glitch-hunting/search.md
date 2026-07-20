# Milestone-backed route search

Route search is a finite-sample hybrid optimizer over deterministic controller
programs. Structured waypoint, roll-timing, heading, duration, and deletion
operators remain strong specialists. A native fitted-Q layer learns from the
authenticated state/action transitions produced by completed trials and uses
its finite-batch estimates to propose additional local interventions. This is
deliberately not an end-to-end pixel DDQN: with tens of clients, deterministic
specialists provide useful priors while the learned layer can test movements
outside their hand-authored neighborhood.

The fitted-Q proposer includes temporally extended consensus actions. For each
successful parent trace it ranks a single controller action over 8, 16, 32,
and 64-tick windows by mean fitted-Q advantage, then submits the resulting
held-action tape to the native evaluator. These are ordinary, fallible
counterfactual proposals: the model does not simulate their next states and
cannot declare success. The longer horizon is what lets the learner replace a
noisy series of one-frame steering corrections with a coherent straight or
turn instead of requiring dozens of independently lucky mutations.

Observation consumers resolve current movement-state features by their stable
semantic names. Behavior archive cells, Q coverage, procedures, positions,
and window phase must never use numeric offsets copied from an older feature
schema. Legacy movement-state/v1 corpora retain an explicit versioned layout;
unknown schemas fail closed.

Poor learned-proposal performance reduces subsequent learned allocation to a
three-candidate exploration floor (greedy, temporal consensus, and uncertainty)
rather than permanently switching learning off. Missing native facts,
insufficient corpus coverage, or unproved determinism still disable learned
proposals. This distinction prevents one weak batch from trapping the search
at the incumbent while preserving the framework's evidence gates.

C++ is the scoring authority: it reports the first simulation tick and complete
boundary fingerprint for each memory-backed milestone. Rust owns candidates,
compact tape compilation, native process scheduling, evidence, ranking, and
evolution. Python and PowerShell are not in the execution path.

## Candidate IR

A candidate uses schema dusklight-search-candidate/v2. Its typed macros compile
to DUSKTAPE, which remains replay authority:

    {
      "schema": "dusklight-search-candidate/v2",
      "segment": "fsp103_to_fsp104",
      "boot": {
        "kind": "stage", "stage": "F_SP103", "room": 1, "point": 1, "layer": 3
      },
      "actions": [
        { "op": "neutral", "frames": 180 },
        { "op": "move", "angle_degrees": 0, "magnitude": 127, "frames": 30 },
        {
          "op": "roll", "angle_degrees": 4, "magnitude": 127,
          "button_frame": 1, "recovery_frames": 12,
          "spacing": { "period_ticks": 4, "phase_tick": 2 }
        },
        {
          "op": "game_tactic",
          "plan": {
            "schema": "dusklight-game-tactic/v1",
            "tactic": {
              "kind": "crawl", "direction_degrees": 0,
              "magnitude": 127, "frames": 20, "action_held": true
            },
            "cancellation_conditions": []
          }
        }
      ],
      "ancestry": { "generation": 0 }
    }

Zero degrees is forward and positive 90 degrees is right. A roll presses B on
its declared frame, holds its analog direction during recovery, and may require
an absolute modulo spacing phase. Missing timing fields preserve the legacy
first-frame, unconstrained-phase behavior. Press supports typed A, B, and Start
pulses for boot-menu optimization. `game_tactic` exposes bounded combat, item,
interaction, traversal, and mount recipes; static candidates reject reactive
cancellation conditions. `motion_path` exposes exact-duration integer waypoint,
rail, Catmull–Rom spline, and cubic Bézier stick paths with rational sampling
phase. Neutral makes startup and inter-input waits explicit and evolvable.

An existing absolute boot tape can be imported without hand-authoring JSON:

    huntctl search import-tape --segment boot_to_fsp103 --tape build/boot.tape --output build/boot.candidate.json

Boot import is lossless and deliberately narrow. It accepts neutral frames and
zero-stick typed A/B/Start pulses. Both anchored movement profiles accept an
absolute port-one movement tape: they run-length encode the complete raw pad
state as `pad_run` actions, including analog samples and trigger values, and
preserve the tape's boot origin while verifying that compilation reproduces
every source byte. Reactive waits and
noncanonical secondary-port state remain rejected.

The segment profiles are:

- boot_to_fsp103: process boot through restored control in F_SP103;
- fsp103_to_fsp104: tape-declared direct F_SP103 start through entry into F_SP104;
- link_control_to_tunnel_crawl_start: an anchored suffix from the checked-in
  Link-control boundary to `crawl_start` in F_SP104 room 1 spawn 0.

## Anchored clean-boot suffix search

Movement objectives beneath a proved parent segment use the anchored library
evaluator. Route selection first identifies an exact
segment occurrence and its structural parent. A goal attached to that segment
then selects the predicate used for acceptance. `AnchoredObjectiveConfig`
adapts this into the native evaluator's existing milestone protocol: immutable
absolute prefix tape, compiled DMSP, source predicate and boundary fingerprint,
and goal predicate. `AnchoredEvaluateConfig` and `AnchoredSearchRunConfig` are
the public wiring surfaces for the CLI and route workbench.

The observed segment tape imports losslessly and becomes the initial seed; an
anchored run fails configuration validation rather than silently substituting a
synthetic route. Pass `--candidate FILE` to `search run-route` to repeat or
continue from an exact previously mined suffix candidate; its segment profile
must match the selected timeline segment.

Every trial concatenates the same immutable prefix with one candidate suffix
and boots that complete tape in a clean process. It does not pass `--stage`;
any fixture origin comes from the composed tape itself.
The native run receives the compiled milestone program in the exact order
selected by the command: source, each repeatable `--progress-goal`, then goal.
Progress goals must be attached to or proved by the searched segment. A
source-only miss is an ordinary failure, a miss that reaches one or more
progress milestones is a near miss, and only the goal is success. A
result is accepted only when all of the following match:

- DMSP program and source/progress/goal definition digests;
- the source milestone's final prefix frame, boundary index, and pinned
  boundary fingerprint;
- the exact source and goal predicates selected from the timeline.

The content-derived objective digest covers the prefix bytes, DMSP bytes,
game executable and DVD SHA-256 identities, source proof, and goal. Anchored
mode rejects extra game arguments entirely, so stage, timing, and CVar changes
cannot escape that contract. The identity is stored beside the population and
in anchored results, preventing results from being reused after proof or
execution inputs change.
Ranking records goal time relative to the source boundary. The winner emits
both `champion.suffix.tape` for continuation work and a composed
`champion.tape` for clean-boot visual playback.

Each generation seals all repeated evaluation outcomes before any first
attempt corpus becomes training-eligible. Online learned proposals remain
unavailable until the sealed generation contains at least one success, near
miss, and ordinary failure; proof repetitions never enter training.

The route-aware command derives the prefix, source fingerprint, source goal,
target goal, program, and observed seed from the checked-in timeline and
lineage. When either segment has several attached goals, pass `--source-goal`
or `--goal` explicitly. Add repeatable `--progress-goal` options in the order
that defines increasing progress; no implicit timeline-map ordering is used:

    huntctl search run-route --timeline "routes/Glitch Exhibition/intro.timeline" --lineage main --segment to_ordon_spring_q129 --source-goal link_control --progress-goal ordon_spring_exit_approach --goal ordon_spring_load_committed --game build/windows-clang-debug/dusklight.exe --dvd game.iso --output build/search/ordon-spring --generations 4 --size 16 --elites 4 --workers 8 --repetitions 3 --rng-seed 1

The loose executable inputs may be replaced with `--run-request REQUEST.json`
and `--repository-root ROOT`. The sealed request must bind the exact
timeline-selected goal, compiled milestone-program digest, and
`movement-action/v2` schema. Route search, anchored tournaments, and anchored
reduction share this validation, so none can report one anchored objective
while executing another.

It refuses a timeline segment that is not immediately after the requested
lineage prefix. The compiled DMSP and materialized prefix are retained in the
sibling `build/search/tunnel.objective/` directory; attempt and champion
artifacts remain below the requested output root.

Completed route generations are also projected into the Route Workbench
automatically, including while a longer farm is still running. Up to four repeat-proved elites from a run appear as ordinary, uncommitted
siblings beneath the segment whose output fingerprint matches the anchored
source. Refreshing the graph discovers new results and removes deleted run
artifacts; no import step or separate search browser is required. Generated
nodes play from their exact clean-boot tape but cannot be renamed, deleted, or
used as recording parents until their compact suffix and proof are promoted to
the Git-owned timeline.

### Exact anchored finalist minimization

Use the dedicated reducer after an anchored search or tournament produces a
finalist whose terminal state must not drift:

    huntctl search minimize-route --candidate build/search/finalist.candidate.json --anchored-prefix build/harness/prefix.tape --milestones build/harness/objective.dmsp --segment fsp103_to_fsp104 --source-milestone link_control --source-boundary-fingerprint FINGERPRINT --goal-milestone ordon_spring_load_committed --game build/macos-default-debug/Dusklight.app/Contents/MacOS/Dusklight --dvd orig/GZ2E01/GZ2E01.iso --output build/search/route-minimized --workers 8 --repetitions 3 --candidate-budget 256

`minimize-route` prepares one content-bound anchored objective and proves the
source in repeated clean processes. It first truncates frames after the exact
goal boundary, then tests deterministic action partitions and one-frame macro
duration reductions. A proposal is eligible only when every repetition keeps
the source proof, relative first-hit tick, absolute goal simulation tick, goal
tape frame, and terminal boundary fingerprint exactly equal to the source.
Every candidate still executes as the immutable prefix plus proposed suffix
through the authenticated evaluator; the reducer has no separate or synthetic
acceptance path.

The execution authority may instead be supplied with `--run-request
REQUEST.json` and `--repository-root DIR`. The request must name the exact goal
milestone and bind the exact compiled milestone-program digest used by the
anchored objective; `--game`, `--dvd`, and other legacy execution inputs cannot
be mixed with it. That same sealed request is used for the source proof, every
reduction round, resume proof, and final proof.

The output retains `minimized.candidate.json`, the compact
`minimized.suffix.tape`, the full clean-boot `minimized.tape`,
`reduction-history.json`, source and final-proof results, all intermediate
attempt evidence, and `minimize.summary.json`. The candidate budget covers
reduction proposals; source and independent final proofs are always additional
and both require at least two repetitions.

Each completed reduction round also writes an immutable checkpoint below
`checkpoints/`. Resume an interrupted, incomplete output root by repeating the
same command with `--resume`. Resume re-hashes and re-authenticates the complete
objective, re-proves both the original source and the checkpoint's retained
candidate in fresh clean processes, and only then spends the remaining proposal
budget. It rejects changed inputs, source identity, terminal contract, candidate
budget, malformed history, or a completed output root. New proof and round
evidence uses fresh suffixed directories, so evidence left by the interrupted
process is retained rather than overwritten.
Authenticated runs write v2 checkpoints containing the sealed request digest,
and resume rejects a missing or changed authority. Legacy v1 checkpoints remain
resumable only through the legacy execution path.

### Predicate-bounded menu input golf

For short menu or dialogue segments, use the generic anchored input golfer
instead of a stochastic optimizer:

    huntctl search golf-route-inputs --timeline "routes/Glitch Exhibition/intro.timeline" --segment tolink_choose_play --anchor-segment tolink_title_ready --source-goal title_logo_skip_ready --goal data_select_ready --game build/windows-clang-debug/dusklight.exe --dvd "orig/GZ2E01/Legend of Zelda, The - Twilight Princess (USA).iso" --output build/search/choose-play-golf --workers 8 --repetitions 3 --candidate-budget 256

The route-aware form anchors to the immediate parent by default. Supply
`--anchor-segment` to materialize an earlier ancestor prefix and golf the whole
descendant window through the selected target. This is required when an earlier
intermediate boundary may poison downstream timings: the target predicate, not
an unchanged descendant suffix, decides whether the repair succeeds. Predicate
source files remain local to their own segments; the command compiles the
anchor and target definitions into an ephemeral two-boundary objective. If
either segment exposes multiple goals, select them explicitly with
`--source-goal` and `--goal`.

The lower-level form accepts those artifacts directly:

    huntctl search golf-inputs --candidate build/choose-play.candidate.json --anchored-prefix build/choose-play-prefix.tape --milestones build/tolink.dmsp --segment boot_to_fsp103 --source-milestone open_title --source-boundary-fingerprint FINGERPRINT --goal-milestone choose_play --game build/windows-clang-debug/dusklight.exe --dvd "orig/GZ2E01/Legend of Zelda, The - Twilight Princess (USA).iso" --output build/search/choose-play-golf --workers 8 --repetitions 3 --candidate-budget 256

`golf-inputs` edits only pure, zero-stick A/Start pulse frames in the candidate
suffix. Each round tries removing one pulse, then moving a surviving pulse to
an earlier free frame without changing pulse order, testing both its authored
button and the A/Start alternative. It also tests the alternate button at the
unchanged timestamp. Proposals are deterministic and bounded by
`--candidate-budget`; there is no random seed or model. Every
candidate replays the immutable prefix from a clean process and must reach the
selected goal predicate with identical evidence in every repetition.

Selection minimizes the goal milestone's simulation tick first, then pulse
count, suffix length, and pulse timestamps. This lets a useless press disappear
and lets a same-tick earlier press be retained as a repair that may unlock the
next coordinate. The goal predicate—not a hard-coded route name—defines
success, so the same command applies to each authored ToLink section and later
menu sequences. The winner is truncated at the goal observation and cold-proved
again. Outputs include `golfed.candidate.json`, the compact `golfed.tape`, the
full `golfed.realized.tape`, proof evidence, and an accepted-edit history.

## Native evaluation

Both the game executable and disc image are explicit. There is no saved-config
fallback:

    huntctl search evaluate --population build/search/g000/manifest.json --game build/windows-clang-debug/dusklight.exe --dvd orig/GZ2E01/GZ2E01.iso --output build/search/g000/evaluations --results build/search/g000/results.json --workers 8 --repetitions 3 --timeout-seconds 300

Rust starts at most the requested number of isolated Dusklight processes. Every
attempt receives its own automation state, stdout, stderr, native milestone
result, boundary fingerprints, and attempt evidence. Timeouts kill the child.
For a fixed population, repetition count, and worker count, planned trial index
maps to a stable strided worker lane; thread wakeup and completion order cannot
change the recorded worker identity. Before launch, `worker-schedule.json`
records every planned trial, candidate, repetition, and worker lane, and the
evaluation report links to and hashes that artifact. Completed evidence is
sorted by candidate and repetition, then checked against the schedule before
artifact addressing or aggregation; a missing plan entry, duplicate trial
identity, or different worker identity fails closed.
Any launch failure, timeout, missing result, malformed schema, contradictory
milestone sequence, or evidence-write failure cancels the population and makes
the command fail. A legitimate goal miss remains a valid partial sample.

For the current F_SP103 route, the objective is the first committed transition
to F_SP104 room 1 spawn 0. Shader compilation and host filesystem latency must
freeze simulation and therefore can never enter the score. If emulated DVD
latency advances guest simulation deterministically, those guest ticks may be
meaningful to a later load-complete objective, but they are downstream of this
load-zone golf. Boundary fingerprints remain in attempt evidence for lineage
compatibility decisions.

## Complete generation loop

The native command owns seed, evaluate, rank, evolve, and champion promotion:

    huntctl search run --segment fsp103_to_fsp104 --game build/windows-clang-debug/dusklight.exe --dvd orig/GZ2E01/GZ2E01.iso --output build/search/intro --generations 4 --size 16 --elites 4 --workers 8 --repetitions 3 --rng-seed 1

Each generation contains its manifest, candidates, compact tapes, isolated
attempt evidence, results, and leaderboard. The final root contains the exact
`champion.candidate.json`, its compiled `champion.tape`, and `run.summary.json`.
Population schema v3 and result schema v3 bind one exact tape boot origin at
the top level, and every leaderboard row repeats it. Population v3 also stores
the compiled tape's canonical input-complexity count. Construction rejects
candidates with mixed origins; collection and ranking reject results whose
origin differs from the population. Consequently a process-boot score can
never enter a stage-fixture leaderboard even when both use the same segment
label.
To continue mining from an existing candidate instead of restarting from the
built-in baseline, pass `--candidate FILE` to `search run`. The candidate is
validated and must match `--segment`.

For a successful boot tape, use the native reducer before spending samples on
more evolution:

    huntctl search minimize-boot --candidate build/dense.candidate.json --game build/windows-clang-debug/dusklight.exe --dvd orig/GZ2E01/GZ2E01.iso --output build/search/boot-minimized --workers 16 --repetitions 3

The reducer first proves the source against the corrected route-control oracle.
It neutralizes chunks of active button frames without shifting the timestamps
of surviving input, then removes individual pulse frames. A deletion is kept
only when every repetition produces identical milestone depth, goal outcome,
ticks, tape frames, and boundary fingerprints and still reaches the exact goal.
The source proof's goal simulation tick, tape frame, and boundary fingerprint
become an immutable reduction target; a deletion that succeeds later or reaches
a different boundary is rejected. Among exact-target equivalents, candidates
with fewer pulse frames win.

Finally, it truncates the tape to `goal tape_frame + 1` and proves that exact
artifact again. The output contains `minimized.candidate.json`,
`minimized.tape`, `proof.json`, and `minimize.summary.json`; intermediate ddmin
rounds remain under the output root for audit.

After reduction, golf the absolute timing of the surviving boot pulses without
changing their order:

    huntctl search golf-boot --candidate build/search/boot-minimized/minimized.candidate.json --game build/windows-clang-debug/dusklight.exe --dvd orig/GZ2E01/GZ2E01.iso --output build/search/boot-golfed --workers 16 --repetitions 3

This is exhaustive coordinate descent, not evolution or random sampling. Each
round tests every legal earlier absolute frame for every existing pulse,
starting with the final pulse. At each earlier coordinate it tests both the
authored button and its A/Start alternative, and it tests the alternate at the
current coordinate. A candidate is eligible only when all
repeated runs agree exactly, it reaches the source proof's boundary
fingerprint, and it does not regress the current goal tick. Selection minimizes
goal tick first, then the sum and lexicographic vector of pulse timestamps.
Consequently an earlier same-tick move is retained: it may open space for an
earlier neighboring pulse and expose a faster pair on the next round. Golfing
stops only when no single coordinate/button choice has an eligible earlier
move, then runs a separate exact proof after truncating the winner to
`goal tape_frame + 1`.

The output contains `golfed.candidate.json`, `golfed.tape`, `proof.json`, and
`golf.summary.json`. Every tested round remains below `rounds/`, including the
source proof, manifests, per-attempt evidence, and results. This proves a local
single-coordinate minimum for the fixed ordered A/Start pulse sequence; it does
not claim a global optimum across added/deleted pulses, reordered pulses, or
coordinated moves that require a temporarily later goal tick.

Both boot proof tools accept `--run-request REQUEST.json --repository-root
ROOT` in place of loose executable inputs when the request's exact goal is
`gameplay-ready-f-sp103`. That authority is retained through every reduction
batch and final proof; mixed execution inputs are rejected. They require at
least two repetitions, so `--repetitions 1` is rejected rather than silently
weakening determinism into a single observation.

The beam, CEM/CMA-ES, and Bayesian commands below also accept
`--run-request REQUEST.json --repository-root ROOT` in place of `--game` and
`--dvd`. The request is the sole execution authority and is retained through
every generated batch; executable, game-data, working-directory, game-argument,
or timeout overrides cannot be mixed with it. Each attempt then retains and
authenticates its derived harness request and result.

## Discrete beam search and terminal branch bounds

For a finite catalog of typed `MacroAction` JSON values, run bounded beam
search through the same native evaluator used by ordinary populations:

```text
huntctl search beam --candidate build/seed.candidate.json \
  --options build/discrete-options.json --game ./dusklight --dvd game.iso \
  --output build/search/beam --beam-width 8 --maximum-depth 8 \
  --candidate-budget 1000 --workers 8 --repetitions 3
```

Depth zero evaluates the seed. Each later depth appends one catalog option to
each retained prefix, deduplicates candidate identities before launch, and
ranks only repeated native milestone results. The candidate budget counts
evaluated prefixes and the summary reports the corresponding simulator
episodes, duplicates, beam-pruned prefixes, and depth count.

Branch-and-bound uses one deliberately narrow exact bound: a prefix that has
already proved the terminal goal is never expanded. A suffix appended after
that first hit cannot improve its hit tick and can only increase tape size, so
all such children are dominated. No learned estimate or heuristic is treated
as a bound. Every nonterminal score remains an exact simulator rollout, and
the complete per-depth populations, results, leaderboards, and attempt evidence
remain under the output root.

An optional `--q-priors PRIORS.json` supplies
`dusklight-q-beam-priors/v1`. The table binds the learned model, feature,
action, objective, and exact option-catalog digests. For each parent it orders
supported children by `Q - uncertainty_penalty * ensemble_stddev`; unsupported
options remain available after supported ones. This ordering matters only when
the candidate budget truncates expansion. Q never changes a native leaderboard
score, declares a bound or terminal, selects the champion, or carries route or
promotion authority. `beam.summary.json` records the prior table/model and the
number of prior-ranked children while explicitly retaining native rollout
ranking authority.

## Bounded CEM and CMA-ES

Low-dimensional typed parameters can be optimized with seeded cross-entropy or
full-covariance CMA-ES:

```text
huntctl search continuous --method cma-es \
  --candidate build/seed.candidate.json --axes build/axes.json \
  --game ./dusklight --dvd game.iso --output build/search/cma \
  --generations 20 --population 32 --elites 8 \
  --initial-sigma 0.25 --candidate-budget 640 --rng-seed 7
```

`dusklight-continuous-axes/v1` declares 1–16 unique bounded axes over typed move
and roll heading, magnitude, duration/button fields or motion-path duration,
sample phase, and point coordinates. Values are sampled continuously, rounded
once into the declared native field, compiled into an ordinary candidate, and
validated. Candidates that round to a previously seen input are attributed as
duplicates and never launched.

CEM maintains a smoothed full covariance over its ranked elite set. CMA-ES
maintains the standard weighted mean, global step size, covariance and
conjugate evolution paths, using a jitter-bounded Cholesky transform. Both are
seeded and bounded; neither consumes scalar model fitness. Each generation is
ranked best-to-worst exclusively by repeated native `SearchResults`, then saves
the ranked samples and next optimizer state under `gNNN/optimizer.json`.
`continuous.summary.json` reports candidate/episode budgets, invalid and
duplicate proposals, final optimizer state, exact champion tape, and native
lexicographic score.

## Bounded Bayesian optimization

For tactics where each native episode is expensive but the response is expected
to be locally smooth, the same typed axis file can drive Gaussian-process
expected-improvement search:

```text
huntctl search bayesian \
  --candidate build/seed.candidate.json --axes build/axes.json \
  --game ./dusklight --dvd game.iso --output build/search/bayesian \
  --generations 20 --batch-size 4 --initial-samples 8 \
  --acquisition-pool 2048 --candidate-budget 80 --rng-seed 7
```

The initial design and every bounded acquisition pool come from a seeded,
shifted Halton sequence. After the initial design, an RBF Gaussian process fits
the empirical within-generation native rank utility and expected improvement
selects the next batch. That utility is intentionally ordinal: it does not
replace or approximate the milestone score across generations. Every proposed
vector is rounded into typed candidate fields, compiled, validated, and
deduplicated before launch; repeated native `SearchResults` remain the only
ranking and champion authority.

The optimizer accepts at most 16 dimensions, 512 observations, and 65,536
acquisition points per batch so exact cubic GP fitting stays operationally
bounded. Each `gNNN/optimizer.json` records proposals, native rank observations,
and the next acquisition state. `bayesian.summary.json` records candidate and
episode budgets, invalid and duplicate proposals, the final surrogate state,
and the exact native-ranked champion candidate and tape.

## Equal-budget proposer tournaments

`search tournament` compares already materialized proposer populations through
one deduplicated native evaluation:

```text
huntctl search tournament --definition build/tournament.json \
  --game ./dusklight --dvd game.iso --output build/search/tournament \
  --workers 8 --repetitions 3
```

For an authenticated objective campaign, replace the loose executable inputs
with `--run-request REQUEST.json --repository-root ROOT`. The request becomes
the sole authority for executable, game data, objective, scenario, schemas,
fidelity, timeout, and protocol capabilities; combining it with `--game`,
`--dvd`, `--game-arg`, a working directory, or timeout override is rejected.
Every tournament attempt then retains its derived request/result identities.

Prefix-anchored route suffixes use the same tournament boundary by supplying
`--anchored-prefix`, the compiled `--milestones` program, `--segment`, source
milestone and fingerprint, and goal milestone beside either `--game` and
`--dvd` or the sole `--run-request` authority. Those inputs are content-bound
as one anchored objective before native spend. In request mode, the request
must independently bind the exact goal, compiled milestone-program digest, and
movement action schema. Successful finalists are the clean-boot
prefix-plus-suffix tapes actually replayed, never an unbootable suffix presented
as proof.

To extract one authenticated candidate from a search population or
`q-proposals.json` into a tournament lane without rewriting its generation or
seed:

```text
huntctl search prepare-tournament-lane \
  --candidate build/search/g002/CANDIDATE.candidate.json \
  --proposal-envelopes build/search/g001/q-proposals.json \
  --output build/tournament/random
```

The command writes a one-candidate population and an exact one-envelope set;
it rejects absent, duplicate, or mismatched candidate provenance.

The definition uses schema `dusklight-proposer-tournament-definition/v2`, one
`episodes` or `candidate_ticks` cap per proposer, and 2–16 named entries. Every
entry supplies both its population and a content-authenticated
`dusklight-candidate-envelope-set/v1`:

```json
{
  "schema": "dusklight-proposer-tournament-definition/v2",
  "budget_unit": "episodes",
  "budget_per_proposer": 48,
  "proposers": [
    { "name": "incumbent", "kind": "incumbent_mutation", "population": "incumbent/manifest.json", "proposal_envelopes": "incumbent/proposal-envelopes.json" },
    { "name": "blind", "kind": "blind_exploration", "population": "blind/manifest.json", "proposal_envelopes": "blind/proposal-envelopes.json" },
    { "name": "cma", "kind": "structured", "population": "cma/manifest.json", "proposal_envelopes": "cma/proposal-envelopes.json" },
    { "name": "fqi", "kind": "learned", "population": "fqi/manifest.json", "proposal_envelopes": "fqi/proposal-envelopes.json" }
  ]
}
```

All populations must carry the same segment and boot origin. An envelope set
must exactly cover its population and bind every candidate digest, parent,
generation, seed, objective, action schema, and proposer configuration. The
declared lane kind must match the authenticated proposer kind; all lanes must
share one objective and action schema. Under `--run-request`, ordinary
tournaments match those identities directly to the request. Anchored
tournaments instead retain the derived anchored-objective digest in their
envelopes while separately verifying the request's underlying milestone program
and goal before output creation or simulator spend. Episode caps must be exact
multiples of the repetition count;
candidate-tick caps charge compiled tape frames times repetitions. The runner
refuses definitions without both an incumbent-mutation lane and a
blind-exploration lane, selects every lane under the same declared cap, and
deduplicates candidate IDs globally before launching the combined population.
A shared candidate is evaluated once but credited to every proposer that
supplied it.

`tournament.summary.json` (`dusklight-proposer-tournament/v3`) retains the
shared objective/action identities, each exact proposer identity and envelope
set digest, and attributes shared
duplicates, native predicate hits and misses, improvements over the incumbent
champion, frame wins, distinct authenticated boundaries and their sorted
fingerprint digests, repeated cold-replay passes, charged and physical
episodes/ticks, and total evaluation wall time. Infrastructure failures remain
hard failures; their candidate/proposer ancestry and typed crash, timeout,
desync, or unsupported outcome stay in `evaluations/evaluation.json` rather
than being converted into a score. No proposer supplies results or bypasses
native predicate, determinism, and replay validation.

Every proposer row has an explicit `proved` or `objective_miss` replay verdict.
Only a best candidate whose repeated native trials reached the objective and
passed exact evidence determinism gets a content-addressed tape under
`finalists/`; a miss has no `best_proved_tape` field. This keeps the compact
comparison useful without turning a leaderboard score into promotion proof.

Individual primitives remain available:

    huntctl search seed --segment fsp103_to_fsp104 --output build/search/g0 --size 16 --rng-seed 1
    huntctl search rank --population build/search/g0/manifest.json --results build/search/g0/results.json
    huntctl search evolve --population build/search/g0/manifest.json --results build/search/g0/results.json --output build/search/g1 --size 16 --elites 4 --rng-seed 2

Current result schema v3 carries the exact terminal predicate verdict instead
of inferring feasibility from progress depth. Ranking uses this complete,
declared lexicographic vector, with earlier axes always dominating later ones:

1. exact terminal-predicate feasibility, feasible first;
2. deepest verified goal/milestone depth, deeper first;
3. first-hit median and then best simulation tick, earlier first;
4. compiled tape frame count, shorter first;
5. canonical input complexity, simpler first;
6. authenticated risk-event count, lower first; and
7. declared boundary compatibility, ordered `exact`, `compatible`, `unknown`,
   then `incompatible`.

Input complexity is representation-independent because it is computed after
candidate compilation over absolute DUSKTAPE frames. Each changed ownership
bit and button bit counts independently; each change to wait kind, wait
timeout, stick/substick axis, trigger, analog button, connection, or controller
error counts once. Repeating an unchanged frame costs no additional
complexity.

Risk `null` means unmeasured, never zero, and ranks below every measured risk
count. Boundary compatibility remains `unknown` until it is compared against a
declared authenticated reference; route topology is never used as a substitute.
Candidate ID is only a deterministic tie-breaker after all declared axes.

A current result without `goal_reached`, a goal hit with no milestone evidence,
or repeated runs that disagree on the verdict is rejected before ranking.
Legacy result v1/v2 and population v1/v2 files remain readable under their old
depth-implied or unmeasured-complexity semantics.

Repetitions are a hard determinism check, not a probabilistic
ranking dimension: identical trials must agree on milestone depth, goal
outcome, every hit's simulation tick and tape frame, boundary fingerprints, and
named value projections.
Any disagreement rejects the evaluation. Deterministic all-miss candidates are
valid evidence and remain below candidates that reach a milestone.

Current mutations adjust macro duration, analog heading and magnitude, insert
rolls, split/delete movement segments, and shrink explicit waits. Boot mutation
directly shifts and shrinks the neutral gaps attached to menu button presses;
it does not spend most samples perturbing only the initial boot wait. Candidate
Pad-run populations additionally perturb exact raw stick samples and toggle B
on selected runs, so importing a human tape does not reduce mining to duration
deletion alone. Candidate IDs hash segment plus input program, so identical
tapes deduplicate even if
separate search branches rediscover them; ancestry records the retained parent
and mutation for every generation.

For a successful typed roll, `search golf-option` provides a deterministic
option-relative neighborhood instead of a population mutation. It authenticates
the seed execution and tape, then emits exact proposals for heading, magnitude,
duration, roll-spacing phase, button timing, and cancellation timing. The
proposal manifest does not claim success; each variant must go through the same
goal evaluation, exact execution capture, and cold-tape replay as any other
candidate.

## Hybrid fitted-Q proposals

For anchored movement searches, every proved candidate's first repetition
retains a compact transition corpus. Corpora are content-deduplicated across
generations and fitted together. The current generation's repeat-proved elites
provide aligned candidate tapes on which the learner may intervene; training
can use every compatible episode observed so far without confusing a losing
episode with an eligible parent.

After elites are retained, up to one quarter of the non-elite slots preserve a
bounded quality-diversity archive. Its v3 descriptor covers terminal map/room,
player procedure, midpoint and terminal position, closest scene exit,
deduplicated coarse route sequence, and the sequence of player procedures.
Available named RNG and actor-population projections add authenticated value
axes. The run-deduplicated collision/contact trajectory adds a portable axis
with native session process IDs removed, and terminal milestone boundaries add
a separate authenticated boundary axis. All terminal boundary and value
fingerprints also remain bound into the downstream-state axis.

Trace-backed evaluation constructs `dusklight-semantic-novelty/v2` before cell
placement. It retains the raw run-deduplicated procedure and event sequences,
semantic state changes, portable contact sets, player-relative selected-actor
facts, flag states, quantized position/velocity extrema, and named boundary
fingerprints. Per-axis SHA-256 identities keep archive keys compact, but the raw
descriptor is the canonical explanation surface for later discovery reports.
Missing contact or actor channels remain unobserved rather than becoming an
observed empty set, and process-local actor IDs never affect identity.

`SemanticNoveltyCatalog` makes the corpus-wide decision separately from the
spatial archive distance. It counts each exact semantic transition and each
aligned state/event/contact/actor/flag combination at most once per episode.
An assessment is computed against the pre-insertion catalog and records the raw
first-seen transitions plus every low-support combination and its prior episode
count. The configured rarity ceiling is bounded and included in the assessment;
`spatial_distance_used=false` makes the decision basis explicit. A canonical
sorted snapshot exposes the accumulated support counts for audit and replay.

Autonomous campaigns retain those assessments in
`dusklight-discovery-archive/v1`. Scenario SHA-256 and an exact fidelity SHA-256
are hard partition keys, with headless and headful represented explicitly, so
results from different execution contracts never share cells. Within a
partition, the full semantic descriptor identity selects the cell. A cell keeps
several distinct useful outcome classes (four by default, bounded at eight),
while native-evidence strength, cold-replay count, milestone depth, minimized
length, and artifact identity deterministically select the representative for
one outcome class. Unsupported evidence and weaker replacements are rejected.

`dusklight-semantic-novelty-proposal-signal/v1` converts first-seen transitions
and inverse-support rarity into a bounded numeric proposal-ordering signal. Its
generation artifact retains the complete raw semantic assessment and publishes
the separate transition and combination components. The authority fields are
fixed by the constructor: proposal ordering is true, while native leaderboard,
proof, and promotion authority are false. The normal lexicographic evaluator
therefore remains the only path from a proposed artifact to a proved result.

`dusklight-symptom-cluster-index/v1` suppresses repeated discovery symptoms
within the same scenario/fidelity partition. Stable keys cover crashes, hangs,
OOB routes, corruptions, and event sequences using terminal semantic state, a
bounded event tail, portable contact identity, and terminal boundary identity.
Crash keys use bounded module/symbol frames and a category, never volatile
addresses or process IDs. A cluster retains one representative, occurrence and
generation counts, and at most eight distinct example artifacts, so repeated
hits remain measurable without growing an unbounded duplicate directory.

Novel artifacts use `dusklight-novelty-minimization/v1` for bounded frame
deletion. The starting assessment is frozen into an authenticated preservation
predicate containing every required first-seen transition, rare combination,
catalog epoch, rarity ceiling, and one exact named replay boundary. Every
deletion receives at least two cold replays; it is accepted only when all
repetitions agree on the complete semantic evidence and preserve the boundary's
simulation tick, tape frame, and canonical fingerprint. The report records the
repetition contract and every attempted range, before/after frame counts,
acceptance, and precise rejection reason.

Promising headless artifacts cross an explicit
`dusklight-headful-replay-request/v1` queue into a separately identified
headful fidelity partition. Evidence rank and the bounded proposal signal gate
automatic enqueueing; artifact identity deduplicates pending work. Every replay
requires a content-addressed terminal PNG, while hangs, OOB routes, corruptions,
and event-timing symptoms also require a short video. Validated attachments and
the replay boundary are bound into a pending
`dusklight-human-classification-request/v1` with a fixed classification choice
set. Incomplete evidence leaves the replay pending so capture can be retried.

Completed reviews append `dusklight-human-discovery-label/v1` records. Each
record binds the immutable request, headful replay, replay boundary, source
artifact, reviewer, and the original objective ID/SHA-256. A correction must
name the latest label it supersedes; the prior record remains in sequence.
`dusklight-corpus-human-label-metadata/v1` exports that history beside matching
corpus artifacts with `replay_authority=false` and
`objective_rewrite_authority=false`. Objective disagreement is rejected rather
than being reconciled by changing the earlier definition.

`dusklight-open-question-campaign/v1` scopes bounded semantic questions to one
scenario/fidelity partition and an authenticated campaign definition. Current
questions report unseen procedure/contact pairs, collision destinations without
a scene transition, and contact-state changes while the semantic state remains
unchanged. Assessment always precedes catalog insertion, retains the raw pairs
or changes, and is proposal-only with no promotion authority. Episode and fact
caps prevent an unattended campaign from growing without bound.

This is a bounded MAP-Elites policy: each exact descriptor cell retains its
best native lexicographic result, with frame count and candidate ID as stable
tie-breakers. Farthest-first novelty selection then reserves population slots
for cells farthest from the current elites and already selected cells, even
when they are not currently fastest. The archive keeps at most 256 cells;
`behavior-archive.json` schema v3 records the policy, chosen candidate IDs, and
complete cell descriptors. Neither cell placement nor novelty can promote a
candidate without the normal repeated native evaluation and proof gates.

Fitted-Q proposals may receive half of the slots left after archive retention. They
alternate between a state-guided mean-Q action change and a fully unmasked
uncertainty-weighted action change. The learned lanes are local-improvement
operators over successful parent tapes; failed and near-miss episodes still
train the critic and remain proposal parents for the separately attributed
structured, archive, and blind-coverage lanes. Each learned change replaces a
one-, two-, or four-frame window with an exact canonical controller sample,
compiles back to an ordinary candidate, and goes through the same cold-process
milestone evaluator as every other route. Unsupported schemas, misaligned tape/action
pairs, unsupported required facts, nondeterministic repetitions, insufficient
action/state coverage, or inadequate held-out native performance disable Q
proposals for that generation rather than weakening evaluation. Remaining
slots use the unmasked structured, archive, and blind-coverage operators. The
only bootstrap exception is one recorded trial capped at two candidates, one
per learned lane; it cannot bypass fact, determinism, or coverage gates and is
consumed only after a learned candidate is emitted. Larger initial proposal
batches rotate all remaining slots across the safe structured, archive, and
blind lanes, so the learned cap does not discard budget.

The versioned `dusklight-action-guidance/movement-v2` mask is an advisory prior
over the 68 movement-v2 action classes. Normal gameplay recommends all actions;
an event prefers unbuttoned movement and neutral-stick button states; an absent
player prefers neutral-stick button states; and a pad error prefers neutral.
These recommendations exist only in Q proposal ordering. Guided exploitation
scores only the intersection of recommended and learned actions instead of
evaluating every output and filtering afterward. The periodic unmasked
uncertainty lane can still select any observed action class, including one the
mask does not recommend. If a state has no recommended learned alternative,
the guided lane emits no proposal rather than falling back through the mask;
the systematic, uncertainty, random, and Latin-hypercube lanes remain
explicitly unmasked. Tape compilation, candidate validation, corpus
ingestion, native evaluation, milestone scoring, minimization, replay, and
proof acceptance do not import or consult the mask. Consequently a
glitch-producing input that looks invalid to the prior remains an ordinary
executable and promotable proof candidate.

Movement-state v2 transition corpora use the authenticated
`dusklight.offline-rl.route-goal-progress-reward/v2` step reward: every frame
costs `-1`, while each newly satisfied predicate in the configured route goal
adds `+64`. The goal-progress trace channel, objective identity, predicate
count, and monotonic depth are validated before extraction; missing,
unauthored, changing, or regressing progress fails closed. This lets a critic
distinguish an early approach/load-zone near miss from a tape that made no
useful progress without inferring progress from coordinates or modifying game
state. The three goal-progress observation features and reward schema are
authenticated into corpus and model identity, so older v2 corpora cannot be
silently mixed with them.

Before fitting the route critic, the proposal layer also projects every bounded
tape ending to a terminal decision and applies the authenticated
`dusklight-route-q-terminal-reward/v1` adjustment: `+512` for reaching the
objective and `-512` for ending without it. This prevents a short failed tape
from outranking a longer successful route without rewriting collected evidence.
The step and terminal schemas and values are part of both model lineage and
proposer configuration identity.

Each generation writes `q-proposals.json` v11 with its training size, complete
readiness and coverage gates, step/terminal-reward and successful-parent policies,
guidance schema, masked-state count, guided and
unmasked action evaluation counts, unmasked probe-state count, intervention
counts, exact collection schedule, and proposal count (or an explicit
unavailable reason). Candidate
ancestry marks `q_GuidedExploit` and `q_UnmaskedExplore` proposals, making
equal-budget attribution auditable. In the first
closed-loop route smoke, both Q proposals replayed and reached the 138-frame
goal; the accompanying 137-frame improvement came from a conventional deletion
mutation, not from Q. The distinction matters: executing learned proposals is
proved, while global-search superiority is not yet claimed.

A later five-generation, 12-candidate farm produced the first attributable
learned improvement. The structured generation-zero elite reached Ordon Spring
in 134 simulation ticks. An uncertainty-selected Q intervention changed frames
101..103 to action 18 and reached it in 129 ticks. The exact learned candidate
then passed three independent cold replays at 129/129/129. This is evidence that
the learned layer can discover a useful non-obvious local action, not evidence
that it dominates deterministic roll-spacing or waypoint search in general.
Generation-local `behavior-archive.json` records which alternate routes were
retained and why.

After correcting the route critic's terminal projection, a fixed seed-2
12-candidate run produced a guided FQI edit at frames 106..108. A fresh
four-lane tournament then charged exactly one candidate and three native
episodes to every proposer. The learned edit reached the current authored goal
at 135/135/135 ticks, the strongest fixed-seed structured proposal reached it
at 137/137/137, the scripted incumbent at 138/138/138, and blind exploration
missed. The learned, structured, and scripted lanes all reported a cold-replay
pass rate of 1.0. The retained ignored report is
`build/harness/ordon-spring-learned-win-v1/run/tournament.summary.json`; it
binds the current anchored-objective digest, movement-action schema, proposal
envelopes, seeds, budgets, and exact candidate identities.

The winning learned candidate was then used as the exact seed of a bounded
native narrowing run. The repeatedly proved descendant kept the 135-tick goal
while reducing the suffix from 144 to 143 frames and canonical input complexity
from 115 to 114. Three clean processes agreed on goal boundary fingerprint
`10e8535fa7f688a7b0be646fd7dd7aac`. That fingerprint differs from the original
learned finalist, so both successful boundary states remain in evidence rather
than treating minimization as identity-preserving. The retained ignored summary
is `build/search/ordon-spring-learned-finalist-min-v1/run.summary.json`.

The dedicated exact anchored reducer subsequently proved and trimmed the
original learned finalist without accepting that terminal-state drift. Across
235 bounded proposals it reduced the suffix from 144 to 136 frames, actions
from 81 to 77, and canonical input complexity from 115 to 111. Three source
processes and three independent final-proof processes all reached the goal at
relative tick 135, absolute simulation tick and tape frame 575, with the
original boundary fingerprint `54ebb7fb2397087d9abc598202785197`. No action
deletion or duration reduction preserved that complete contract; the accepted
change was the exact post-goal trim. The retained ignored report is
`build/search/ordon-spring-exact-min-v4/minimize.summary.json`, with its source
proof, complete reduction history, final proof, compact suffix, and realized
clean-boot tape beside it.
