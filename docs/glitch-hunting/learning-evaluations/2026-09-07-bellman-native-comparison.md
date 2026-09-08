# Bounded native check of transition-level hindsight

## Preselected question and budget

Does the transition-level hindsight Bellman learner improve real goal discovery
or route cost enough to justify its fitting time, compared with structured
nonlearning? This compares complete policies, not an isolated hindsight ablation:
V3 includes its documented time-cost objective and pre-success coordinate task.

Use source `e6f3df708d`, the optimized `campaign` Cargo profile, the same CLI for
both arms, and the existing pinned game binding. No demonstration or prior
campaign replay is imported. Use seeds 104729 and 130363 sequentially within each
arm, two native workers, 64 decisions per seed, two proposals per decision,
1,024-tick horizon, 180-second wall budget per seed, and 2 GiB memory budget.
These seeds appeared in the previous comparison; they are not held-out evidence.
Within-arm experience sharing means the second seed is not an independent
learning replication. Do not share experience between arms.

Learned arm: `hindsight_bellman_forest`, refit cadence 32, replay staleness bound
8, existing epsilon 350,000 per million. Control: `structured-non-learning`,
the same treatment identifier but no deployed learned updates, generation-barrier
replay. Both use the existing 12-update model configuration. Timing includes
the full campaign, not just simulator ticks; build time is reported separately.

Request:
`routes/Glitch Exhibition/intro/benchmarks/ordon-p0-zero-shot-route-learning-v1.request.json`.
Binding: `build/diagnostics/heading-v7-execution-20260906/execution.json`.
Outputs: `build/campaigns/ordon-bellman-comparison-20260907/`.

The optimized build took 7m06s; CLI SHA-256:
`a90b03a6aa6d6a8704a7141d15bc25694dddb259ac9036aab6727b6ae0d04064`.
An initial launch under `hindsight/` mistyped the wall budget as
180,000,000,000 microseconds. It was explicitly aborted, terminating only the
identified campaign PID and its two native children. Its partial artifacts are
preserved and excluded, not resumed or imported. The valid learned run uses
`hindsight-bounded/` with 180,000,000 microseconds; the control uses `structured/`.

Success remains `ordon_spring_load_committed`, a native loading transition,
not a coordinate match. Any discovered route must pass cold replay before being
reported as verified. The request's historical 131-tick promotion threshold
does not replace the user's better-than-human objective.

Inspect stopping reasons, fresh versus inherited terminal evidence, changed
policy choices, model-update time, and prefix/restore work alongside route ticks.
Do not extend this into seed mining or adjust budgets after seeing a result.
If learning does not help, use the retained transitions and decision evidence
to choose the next correction rather than promoting the treatment.

## Pre-run orchestration inspection

The refit cadence is not the only update trigger. `learner_update_required` in
`native_tactic_route_runner/learner_authority.rs` also fits when pending replay
revisions exceed the staleness limit. With two admitted proposals per decision,
the eight-revision setting can require updates about every five decisions,
well before the nominal cadence of 32. This is intentional bounded-staleness
behavior, but the configured cadence alone understates expected fitting work.

`worker_pool/dispatch.rs` gives the direct process-local source to the primary
proposal. A sibling on another worker has no such source and goes through
`materialize_job_frontier`, which rebuilds from the authenticated root and
checks the observed state against the restoration receipt. This explains a
recurrent prefix-replay path even when direct continuation works. Sharing the
primary worker's live handle across processes would be invalid; useful reuse
requires worker-local cached ancestors or a genuinely transferable snapshot.

The prior hindsight arm's 86,623 replayed prefix ticks included 31,808 ticks
from macro validation. Its two exploration seeds accounted for the remaining
54,815 ticks and 127 prefix materializations. Macro validation and sibling
materialization are separate costs; do not attribute the entire total to one.
These observations identify engineering opportunities, not proof of why a
learned policy fails to discover a route. No dispatch or update setting is
changed during this comparison.

## Results

Do not promote V3. Under these budgets it found no route, while the control
reproduced its 353-tick route. Neither result establishes better-than-human
performance. No additional seeds or budget extensions were run.

| Policy | Seeds reaching goal | Best route ticks | Campaign wall seconds | Model-update seconds |
| --- | --- | --- | --- | --- |
| Hindsight Bellman V3 | 0/2 | None | 187.3 | 83.8 |
| Structured nonlearning | 1/2 | 353 | 112.7 | 8.1 |

Both arms completed 128 decisions and 256 proposals, with all four seed runs
stopping at the decision limit rather than the wall limit. Reported leases
were complete: no failed, cancelled, retryable, or unresolved leases. The
control's two shadow/bookkeeping model updates were not deployed; the learned
arm performed 29 campaign-level updates. Per-seed legacy `learner_updates`
counters are not the shared learner's update count.

The control first reached the goal in seed 130363; it did not inherit an
already-successful route from seed 104729. Its 353-tick route passed two cold
replays against the native predicate. Proof SHA-256:
`ddfea5ab023a25dce45dfd6973077cd282d18adda426084e0dcaf95b60f71ca3`.
Controller tape SHA-256:
`0610b04d8a5cfd348f5d7e7678622f48538fb85b6bfb5fd342eb7714c1d66a51`.
The 131-tick promotion threshold was not satisfied.

Both campaign summaries validate against their reports and sealed plans.
This checks report consistency, not policy quality. Generated checkpoints,
traces, reports and proof remain local under `build/`; this note is not a
substitute for those artifacts when auditing the run.

## What the failed learned run teaches

- Learning influenced choices: 17 of 27 same-state update probes changed the
  selected action. There were 83 generalized-value selections and 45 epsilon
  selections. This is not a dead or entirely bypassed model.
- Value-selected actions averaged 4.55 realized ticks; epsilon-selected actions
  averaged 17.22. Of the 83 value selections, 34 lasted one tick and 63 lasted
  at most four. These are descriptions, not reasons to ban short actions.
- The longest selected suffix reached 675 ticks. A four-second exploration
  cutoff is not the explanation here.
- Direct restoration/continuation worked: 126 requests of each kind, no reported
  fallback replay. There were still 126 prefix materializations and 44,729
  replayed prefix ticks, including 6,562 from macro validation. The control
  replayed 80,544 prefix ticks, including 35,437 from macro validation.
- V3 used about 45% of reported campaign wall time for model updates. It was
  slower overall despite less prefix replay than the control. Reducing replay
  overhead alone would not establish useful action values.
- Neither arm promoted a discovered macro in this comparison. Candidate mining
  existing in the pipeline is not proof that tactics improve subsequent search.

## A concrete value-horizon failure

The parameterized kernel initializes values to zero on every fit. The live
bridge requests 12 backups; it does not continue the previous fitted values.
Consequently, repeated fits on unchanged experience repeat a finite-depth
estimate, not continued value iteration toward the long-horizon objective.

The focused test `bounded_backups_do_not_establish_cost_to_goal_convergence`
demonstrates this without a game, feature ambiguity, or missing success data:
one fully observed action waits one tick and returns to the same state; another
reaches the goal in twenty ticks. With discount 0.999, twelve cold backups rank
waiting above the known goal-reaching action. Twenty-four backups reverse the
ranking. All four parameterized-kernel tests pass. The test characterizes the
limitation; it does not fix it or demonstrate convergence in the game.

This proves a mechanism that can favor short actions under this time-cost
objective. It does not prove it is the sole cause of the native failures:
goal generalization, rare auxiliary successes, and value extrapolation to
unobserved candidate actions remain possible contributors.

## Next correction

Address the learner's effective value horizon and cold-refit behavior before
another native campaign. Use accumulated experience to improve long-horizon
estimates across updates, with explicit convergence/update diagnostics and
recoverable training state if updates become history-dependent. Do not merely
increase every full-forest refit until this one route happens to succeed, and
do not hide the failure with a waiting penalty or a blessed movement reward.

Worker-local ancestor reuse remains a separately grounded engineering
opportunity. Keep it separate from claims about learning quality. The broad
TASKS.md outcomes remain open; this comparison narrows the next correction,
not the framework's goal.
