# Bounded native comparison of delayed-return exploration

## Preselected comparison

Compare `hindsight_return_knn`, the existing
`goal_relabeled_universal_frontier_double_q` treatment, and a
`structured-non-learning` policy. All use source `b8b96ff5b3`, action family V8,
the same game executable and request, no demonstration, seeds 104729 and 130363,
two native workers, one sequential lane, 64 decisions per seed, two proposals
per decision, and a 1,024-tick rollout horizon.

Each seed has a 180-second wall budget and a 2 GiB memory budget. The wall limit
is per seed, not aggregate; startup/finalization can add overhead. Learned runs
use refit cadence 32 and maximum replay staleness 8. The nonlearning control
uses generation-barrier replay because it does not deploy updates. Experience
is shared between seeds within a treatment, never between treatments; these
are not independent learning replications.

The question is whether delayed-value learning improves real completed routes
or useful work per wall time, not whether it improves an auxiliary metric.
Report fresh versus inherited success, policy choices, checkpoint use, and
stopping reasons alongside route ticks. Do not adjust the experiment after
seeing one treatment's result.

## Execution identity

The optimized CLI rebuild took 10m22s before any native run. Its SHA-256 is
`f79df7ef7fb1b638c92655f5279c9e4bd41323b8cb8a9c822aa6662becb1e61b`.
Compilation time is separate from campaign runtime.

Request: `routes/Glitch Exhibition/intro/benchmarks/ordon-p0-zero-shot-route-learning-v1.request.json`.
Binding: `build/diagnostics/heading-v7-execution-20260906/execution.json`.
The binding name is historical; it pins the native game, not the CLI's action
family. Its game SHA-256 is
`4cca697ab5fea67b42193a99859c95259dc2f29db5c97cc893598861e1a09167`.
The success predicate remains `ordon_spring_load_committed` (stage/room/spawn
transition), never coordinate distance.

Outputs: `build/campaigns/ordon-hindsight-comparison-20260906/`, with separate
`hindsight`, `existing`, and `structured` directories.

## Results

Do not promote the hindsight treatment. The nonlearning control found the
shortest route in this comparison; neither learned treatment demonstrated an
advantage. All three remain well behind the reported 125-tick human route.

| Treatment | Seeds reaching the goal | Best route ticks | Campaign wall seconds | Model-update seconds |
| --- | --- | --- | --- | --- |
| Hindsight return KNN | 0/2 | None | 168.9 | 5.5 |
| Existing learned policy | 1/2 | 564 | 234.1 | 67.1 |
| Structured nonlearning | 1/2 | 353 | 125.9 | 0.5 |

Each arm completed 128 decisions and 256 proposals. All six seed runs stopped
at the decision limit, not the wall limit. The control's model-update time is
bookkeeping/shadow fitting; it deployed no learned-policy updates. Times exclude
the rebuild and subsequent cold verification. This small, sequential comparison
is evidence against promoting the new treatment, not a statistical estimate of
either learner's eventual success rate.

Both successful arms first produced a terminal route in seed 130363; neither
inherited an already-successful route from seed 104729. The existing policy
produced one terminal proposal. The control produced 24 terminal proposals and
13 selected terminal decisions; these are repeated observations within one
successful seed, not independent discoveries.

## What the same runs establish

- The new model influenced execution: 15 of 27 same-state update probes changed
  the selected action. Its first seed made 41 generalized-value selections and
  23 epsilon selections. This establishes influence, not useful learning.
- That seed explored suffixes up to 872 ticks, roughly 29 seconds of game time.
  Its failure cannot be attributed to a four-second episode limit.
- Hindsight fitting was much cheaper than the existing treatment's fitting in
  these runs, but did not produce a completed route. Cheaper updates alone do
  not establish a better learner.
- Save-state branching was active. Each learned arm recorded 127 direct
  process-local restores and 125 direct continuations, with no direct-restore
  fallback replays. Nevertheless, they also replayed 86,623 and 90,224 prefix
  ticks respectively; the control replayed 80,544. Working restoration does not
  yet mean prefixes are efficiently reused throughout exploration.
- All proposal leases completed without reported failed or unresolved leases.
  That rules out those reported execution failures here, not every possible
  orchestration or learning bug.

The prefix counters and reported native-tick totals measure different work;
do not derive an overall simulator-throughput claim from the latter alone.
Likewise, nested timing counters are not independent slices to add together.

## Cold verification

Both discovered routes passed two cold replays from the original source
boundary, with neither the learner nor controller service in the loop. The
353- and 564-tick results reached the actual loading-transition predicate,
not a coordinate proxy. Repetitions agreed on their terminal fingerprints.
These checks authenticate the routes; they do not qualify them as beating the
human reference or change the request's promotion threshold.

Proofs under the output directory:

- `structured/cold-replay/proof.json`: content SHA-256
  `c8a455149e6e7f9b0a09a328bee2fdd9106a6e87c6957e620a7163145173ffb3`.
- `existing/cold-replay/proof.json`: content SHA-256
  `573e2ad7796e9defd742a26035e7c85602d5f536a9e66f53f4a61639feabfede`.

Per-arm `campaign-summary.json`, `report.json`, and per-seed decision journals
contain the measurements above. These generated artifacts remain local under
`build/`; this note preserves the outcome but is not a substitute for the raw
artifacts when reproducing or auditing the run.

## Decision

Keep the new treatment opt-in and leave the general learning capability tasks
open. Do not extend this comparison into seed mining or tune it against the
observed routes.

The next learning investigation should use the recorded experience to explain
the action rankings: whether observations distinguish the relevant choices,
whether delayed targets support those choices, and whether the model's goal
generalization preserves that evidence. In particular, inspect generalization
from achieved intermediate endpoints to an unreached requested goal; this is
a hypothesis to check, not an established cause. A controlled ranking check
can separate representation, target, and selection problems before another
native campaign or algorithm is justified. Prefix materialization is also a
measured engineering cost, but reducing it would not by itself prove that the
learner chooses better actions.
