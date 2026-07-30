# Active tasks: build a credible save-state learning planner

This file contains unfinished framework work only. Completed implementation
belongs in Git, and experimental history belongs in immutable benchmark
artifacts.

## Product objective

Build a generic system that can learn efficient controller routes through a
deterministic game from an authenticated native checkpoint.

The environment gives the system:

- exact restorable game states;
- typed observations and currently applicable actions;
- controller inputs and their realized trajectories;
- a terminal predicate; and
- native ticks consumed.

The system must discover a terminal route, improve it, and learn reusable
action compositions without authored route coordinates, benchmark-specific
rewards, or UI-authored TAS graphs.

The Ordon acceptance problem is intentionally elementary: walk around one
obstacle and hit the real load zone. The ordinary human recording reaches it
at tick `131`; tick `123` or lower is the first credible optimization result.
If scratch learning cannot solve and improve this in minutes rather than
hours, the framework is not credible.

## Evidence snapshot and present diagnosis

The incoming 2026-07-29 work made substantial, useful progress:

- The durable content-addressed state graph owns exact states, realized
  expansions, restoration plans, terminal paths, learner rows, scheduling
  decisions, persistence, and reports.
- Exact restored typed state is validated before graph-owned native expansion.
  Long options expose branchable interior states, and deterministic tests cover
  transpositions, restart, lease lifecycle, and exact terminal returns.
- A deterministic around-the-corner fixture has a known nine-tick optimum. A
  learner trained on graph-disjoint translated fixtures reaches it in `40`
  unique expansions versus `101` for the non-learning control. This is good
  plumbing evidence, not native-policy acceptance.
- Persistent native workers, compact repeated-request transport, subsystem
  suppression parity, phase timing, and checkpoint-owner locality are
  implemented. A short macOS fixed-work treatment reports a 15.54x steady-state
  speedup over relaunching the same topology.
- The Rust source-quality gate has zero debt exemptions and all 552 production
  files are below 1,500 physical lines. The four final oversized orchestration
  files are split by result tests, launch preparation, proposal-pool execution,
  frontier policy, and checkpoint validation responsibilities. The complete
  orchestration suite is hermetic and passes all 325 tests.
- Scratch validation now publishes a movable content-addressed bundle carrying
  request, execution, plan, route, per-seed, graph checkpoint, terminal
  tape/result, source-authority evidence, and a content-bound campaign audit.
  New reports retain every exhausted budget and every proposal's exact
  root-route length, allowing the audit to distinguish first terminal from
  first evaluation of the eventual best route. The clean-checkout audit command
  validates source quality, formatting, the workspace, all orchestration tests,
  and every committed bundle.
- New decision journals retain the exact applicable action surface, fitted
  value and uncertainty support, and selected action across restart. Campaign
  audits and matched comparisons explicitly mark legacy or incomplete
  action-surface timelines, so absent roll/camera opportunities cannot be
  misreported as bad learned ranking.
- Live native decisions now retain their complete state-local pre-lease
  scheduler queue: exact return, generalized terminal support and conditional
  ticks, uncertainty, prediction error, visits, policy rank, global/local
  exploration ranks, consumed learner-model identity, evaluated subset, and
  final committed expansion. The queue is content-bound, campaign audits mark
  legacy gaps explicitly, and a binary-checkpoint restart must reproduce it.
- A content-bound observation audit now recomputes every learner feature
  vector bit from the retained native facts and joins decision sources to
  their typed applicable-action surfaces. It reports actual velocity,
  trajectory, camera, prompt, kinematic-consequence, roll, A, and L coverage
  and fails on missing legacy evidence, authored route sequences, or
  benchmark-specific features. A retained native campaign still has to pass
  this audit before the P2 observation task is complete.
- The held-out tactic comparator now evaluates the actual scratch-discovery
  achieved-goal learner against exact-action mean and the production
  scheduler-only and random-valid policies on identical whole-group splits.
  It reports unsupported actions, validation-calibrated test error, epistemic
  distance, pairwise ordering, top-action wins, and regret. The P2 gate remains
  open until a retained native corpus demonstrates an advantage.
- A content-bound post-terminal control audit now joins immutable pre-lease
  learner queues to later exact graph outcomes and compares predicted total
  ticks with least-visited and random-valid ordering. It withholds the
  exhaustive-local oracle claim unless every queued action has an authenticated
  terminal continuation. Retained native evidence is still required.
- Native route report v36 retains the checksummed binary lease-lifecycle
  journal. Every issued native proposal is classified as completed, retryable,
  cancelled, failed, or unresolved; scratch audits reject unresolved or
  arithmetically detached accounting. Proposal dispatches, unique graph
  expansions, and observed interior segments are distinct metrics, and the
  portable evidence bundle carries the exact journal.
- Every seed now owns an exact binary recovery transaction at each durable
  decision boundary. Resume derives its transaction index from the decision
  journal, loads only that exact content-bound campaign checkpoint, reconciles
  the lease journal, prunes partial native attempt directories and stale or
  ahead recovery points, and repairs committed shared replay idempotently.
  A one-shot hard-exit injector covers lease-issued pre-dispatch, worker
  execution, native completion, recovery commit, and decision commit. Its
  content-bound comparator requires identical semantic traces, useful graph
  work, graph/replay shape, terminal results, and exact retry accounting. All
  five one-decision Windows native cells pass locally. Retained long-campaign
  and cross-platform fault evidence remains open.
- Bounded execution plans now divide their aggregate checkpoint-memory limit
  across active workers and put that capacity into every native cache request.
  The v2 campaign audit recomputes configured and observed pool bounds,
  distinguishes lane refresh pressure from actual fitted-model replay lag,
  fails selection if that lag exceeds the sealed limit, accounts for replay
  fallback, and reports checkpoint-owner locality and assignment skew.
- The hermetic clean-checkout gate does not launch the game executable. A
  current-build one-decision Windows smoke exposed detached root-probe cache
  and batch-schema contracts that the complete Rust suite had missed. Those
  defects are fixed, but supported-platform native launch compatibility is not
  yet a retained gate.
- A dedicated native launch-smoke command now fixes a one-worker,
  one-decision, one-proposal topology and publishes a movable content-addressed
  bundle. It retains the native build/capability hello, initial cached-root
  request/result, compact proposal envelope/result, binary plan and lease
  journal, graph checkpoint, resource audit, and source authorities. The
  clean-checkout gate discovers every committed smoke manifest. The retained
  `win32-x86_64` cell passes from clean native revision `dc374771b1` with
  bundle identity `651b486d54d108e3a214bd6da91dcd22bf3094d8aa536dee030f60f49b1c5a9c`.
  The matching macOS cell remains open.
- Native option summaries now retain same-tick wall-contact/commanded-motion
  overlap and the momentum loss measured on those ticks. The v5 tactic feature
  schema exposes the measured consequence with explicit legacy missingness;
  contact itself still has no encoded desirability.
- A content-bound terminal-route diagnosis command profiles every
  graph-selected scratch result against an authenticated ordinary
  demonstration. It reports typed roll/camera use, duration, trajectory,
  velocity, detour, contact-correlated slowdown, neutral/repeated work, and
  exact action-availability/support coverage without scoring human similarity.
- Matched scratch comparison now keeps graph-node acquisition schedules fixed
  across learned, scheduler-only, and random-valid action ranking. Its
  fail-closed comparator rejects source, horizon, topology, schema, budget, or
  cadence drift and reports both graph sample efficiency and end-to-end phase
  occupancy. No matched native result has been retained yet.
- A bounded macOS scratch campaign reached the actual Ordon load-zone
  predicate in all four configured seeds without human replay. Its best route
  was `231` ticks, median time to first terminal was about `691.6` seconds, and
  worst time was about `728.3` seconds.
- The historical fixed-work curve was only a one-decision warm-fleet
  microbenchmark. Throughput report v4 now refuses that shortcut: every cell
  must execute at least 16 decisions and prove learner updates, durable replay,
  graph admission, repeated non-root restores, persistence, and bounded-cache
  eviction. It retains the phase and occupancy counters needed to identify
  saturation. A portable evidence bundle now content-addresses and compresses
  the aggregate, all sample route reports and their recomputed resource audits,
  request, execution, plan, and source authorities; the clean gate discovers
  committed bundles. No v4 native curve has been retained yet.
- The first current-build Windows v4 cell exposed a real architectural
  bottleneck before the curve completed: one worker required about `62.5`
  minutes for 16 decisions and 256 native expansions. Native simulation used
  about `39.5` seconds, while checkpoint persistence used about `2,253.8`
  seconds, or 60% of wall time. Each decision serialized and reverified the
  whole accumulated replay twice. The remaining curve cells were stopped in
  accordance with experiment discipline. A process-local content-reference
  cache and single-pass recovery commit preserve the same authenticated
  durable format without rereading every immutable object. The matched
  one-worker treatment completed the same 16 decisions, 256 expansions, and
  4,512 native ticks in about `2,601.2` seconds: wall time fell 30.7%,
  persistence fell 45.3%, and useful expansion throughput rose 44.2%.
  Removing duplicate whole-checkpoint authentication reduced the same cell to
  about `2,152.5` seconds with `777.6` seconds in persistence. Relative to the
  untreated cell, wall time fell 42.6%, persistence fell 65.5%, and useful
  expansion throughput rose 74.3% to about `0.119` expansions per second.
  Checkpoint v6 now replaces the remaining whole-history write path with a
  bounded root manifest, one immutable graph base followed by authenticated
  dirty-object and edge-addition records, and a parent-linked append journal
  for retained replay. The graph-derived training corpus is reconstructed
  instead of persisted as a second authority. V5 remains readable and migrates
  on the next write. A 129-mutation growth test proves that individual graph
  records remain within 32 bytes of the first delta after the complete graph
  has grown beyond ten times one record; two-decision replay restart tests
  exercise the append chain. The clean exact-plan v6 rerun completed the same
  16 decisions, 256 unique useful expansions, and 4,512 native ticks with five
  learner updates and four replay refreshes. Relative to the single-auth v5
  predecessor, campaign wall fell 81.3%, persistence fell 96.2%, and output
  bytes fell 86.4%. The retained comparison is
  `docs/glitch-hunting/benchmarks/tactic-route-checkpoint-v6-windows-20260729.json`.
  The first full-curve launch then exposed an invalid memory configuration
  before completing its first sample: a `294,721,440`-byte native checkpoint
  could not fit in the `20,971,520` bytes available to each member of a
  16-worker fleet under a `335,544,320`-byte aggregate bound. Fleet launch now
  requires every worker to report the same nonzero root-checkpoint size and
  rejects any per-worker cache that cannot retain one checkpoint, with the
  required and available byte counts. The balanced curve must use a bounded
  aggregate budget large enough for all 16 workers; it must not reinterpret a
  late missing-retention result as request detachment.
- The corrected Windows curve completed its first one-worker v4 sample before
  the run was stopped: 16 decisions, 256 useful expansions, 5,096 native
  ticks, and about 28.6 wall minutes. Native simulation used only 36.3 seconds;
  campaign admission, graph admission, persistence, and restore/capture work
  consumed most of the measured time. Per-decision wall time rose from about
  33.6 seconds over the first four decisions to 148.5 seconds over the last
  four. Code inspection found that each native batch cloned the complete
  accumulated graph and rebuilt its learner projection, after which retaining
  the selected result cloned and projected that same graph again. Checkpoint
  `e7803326dc` removes the redundant second admission and requires its
  incremental persistence head to remain unchanged. This is a source-level
  bottleneck fix, not retained curve evidence; rerun a matched cell before
  claiming an end-to-end improvement.
- The exact matched one-worker treatment at checkpoint `d6f386d62f` preserved
  the request, execution plan, 16 decisions, 256 useful expansions, 5,096
  native ticks, five learner updates, 256 replay rows, and useful-expansion-set
  identity. Structurally shared ordered graph indexes and incremental learner
  projection reduced duplicate graph admission by 99.2%, campaign admission by
  39.7%, orchestration by 38.6%, and wall time by 9.3%, raising useful
  expansion throughput from about `0.149` to `0.164` per second. The remaining
  measured costs are about 337 seconds of persistence, 326 seconds of tactic
  execution including restore/capture, 196 seconds in the first authoritative
  batch admission, and 178 seconds of model update. The local committed sample
  is matched diagnostic evidence, not the required sealed ten-cell curve.
- A second exact matched one-worker diagnostic at checkpoint `354f11a46d`
  preserved all 16 selected option identities, every emitted proposal tape,
  the 256-member useful-expansion set, 5,096 native ticks, five learner
  updates, and 256 replay rows. Combining transition validation with replay
  identity, reusing prepared route identities, and sharing immutable node
  facts reduced campaign admission only from about `196.3` to `184.5` seconds
  (6.0%). Complete wall time regressed from about `1,556.8` to `1,632.8`
  seconds as model update and persistence varied upward, so this is diagnostic
  evidence, not a throughput win and not permission to launch the full curve.
  Inspection then found that every proposal transition had already been fully
  authenticated while constructing its rewarded outcome, but batch graph
  admission repeated that complete large-snapshot pass. Checkpoint
  `68e37a31ef` carries an immutable authenticated-transition receipt into graph
  admission and preserves the existing JSON/CBOR transition wire shape. It is
  source-level treatment awaiting the same one-worker measurement.

That campaign proves bounded terminal discovery under its exact conditions. It
does not prove practical discovery, useful native learning, route
optimization, cross-platform parity, or a generic framework.

The current evidence cannot yet tell whether the limiting factor is:

- poor expansion ranking or insufficient exploration;
- too few useful expansions per second;
- restore, IPC, learner, persistence, or scheduler contention during a real
  campaign;
- action availability or observation features that make good behavior hard to
  infer;
- a discovery-to-optimization handoff that stops exploiting useful terminal
  evidence; or
- correctness bugs hidden by incomplete retained evidence and failing quality
  gates.

Do not answer this by seed mining. Run the matched diagnostics below before
another long campaign.

## Non-negotiable design

### One authoritative state graph

- A node is an exact restorable native boundary, its complete typed state, and
  the evidence identity that binds them.
- An action expansion records one selected primitive or learned option and its
  complete native realization. Observed segments connect interior boundaries
  without pretending the continuing option was selected again.
- Merge nodes only when future-affecting native state is proven equivalent.
  Semantic proximity is a learner feature, not transposition authority.
- Live process handles and portable machine images are caches for graph nodes,
  never alternative sources of search truth.
- Replay, learner batches, indexes, reports, visualizations, and tactic mining
  are derived graph views. There must not be a second authoritative frontier
  or behavior archive.

### Learning ranks search; native evidence decides truth

- The objective is lexicographic: reach the authenticated terminal, then
  minimize native ticks. Do not encode it as a tunable terminal-reward
  constant.
- Predict terminal reach/support and conditional ticks-to-go separately.
  Budget-censored continuation is unknown, not a failed terminal proof.
- Learned values rank unexecuted node/action expansions. They never fabricate
  terminal support, replace the executed policy result, or promote a route.
- Exact successful graph paths provide exact ticks-to-go targets. Generalized
  estimates and uncertainty remain separately inspectable.
- Before a terminal exists, exploration may use coverage, visits, reachability,
  uncertainty, and prediction error. These are search priorities, not
  benchmark-specific reward.
- State may expose velocity, trajectory history, momentum retention, camera
  state, contacts and measured kinematic effects, prompted-action availability,
  and action history. Do not hand-code “straight,” “roll,” wall avoidance, an
  Ordon waypoint, or any other desired behavior as utility.
- Every currently applicable primitive and promoted option remains selectable.
  Unsupported estimates remain visibly unsupported.

### Discovery and optimization are distinct regimes

Before the first terminal, workers lease diverse node/action expansions and
share graph evidence without collapsing their local coverage frontiers.
Exploration horizons must be long enough for an unskilled route to reach the
goal.

After the first terminal, immediately decompose the successful path into exact
branchable states at no coarser than four native ticks. Rank counterfactuals by
predicted total root-to-terminal ticks, uncertainty, and visits while retaining
a fixed broad-exploration share. A slow first route is evidence, not policy
authority.

### Optional human replay is ordinary evidence

An authenticated human replay may add one path and exact terminal return to the
same graph. It adds no authored waypoint, privileged action, behavior-cloning
authority, separate reward, or mandatory curriculum. Removing it may reduce
sample efficiency; it must not remove the framework's ability to succeed.

### Operational data is binary and content-addressed

JSON is allowed for small authored requests and exported reports. It is not the
operational format for checkpoints, graph nodes, edges, replay, models,
journals, or learned tactics.

## P0 - Restore a truthful, auditable baseline

P0 is a cross-platform evidence-collection gate, not an unimplemented campaign
subsystem. Platform-exclusive collection belongs to that platform's work lane
and does not stop executable engineering or experiments on another platform.

- [x] Implement the native campaign smoke gate. It materializes a current
      execution, launches the compact persistent worker, authenticates the root
      probe, executes one cached tactic decision, passes the v2 resource audit,
      and fails clearly on stale capabilities, detached root-cache
      capacity/schema, or missing native result telemetry.
- [x] Retain and clean-checkout validate the Windows smoke result for the sealed
      request and seed `155921`.
- [ ] **macOS lane:** run and retain the matching smoke for the same sealed
      request and seed. This is ready for platform execution; no remaining
      smoke-gate implementation is known.
- [x] Exhaust repository recovery of the original `231`-tick macOS campaign.
      The aggregate summary is present, but its child report and per-seed graph,
      checkpoint, tape, and terminal-result artifacts are absent from reachable
      Git history, remote branches, and the fork's GitHub Actions artifacts.
- [ ] **macOS lane:** rerun the sealed four-seed campaign and commit its
      self-contained scratch evidence bundle. Do not accept another aggregate
      summary without all independently validated per-seed evidence. Source
      checkpoint `ea88b71af2` fixes two blockers in that bundle path: terminal
      tape ticks are now measured relative to the authenticated source
      boundary, and terminal results are checked against their execution-plan
      authority rather than the unrelated execution-binding digest. This
      remains open pending the platform rerun and retained bundle.

Exit gate:

- A clean checkout passes the documented audit command.
- Current Windows and macOS builds pass the retained native launch smoke.
- Another engineer can independently validate the `231`-tick claim and every
  per-seed terminal without access to the originating macOS build directory.

## P1 - Diagnose the `231`-tick campaign before changing the algorithm

- [ ] Produce a per-seed campaign audit with time and unique useful expansions
      to first terminal and best terminal, selected-node and selected-action
      reasons, exploration versus learned selections, restores and fallbacks,
      learner revisions consumed, terminal-path lengths, and stop reasons.
      Source checkpoints `8abfcf2489` and `97cd210747` advance and verify the
      scratch campaign audit v3. Terminal-improvement ticks are relative to
      the authenticated graph root; first/best wall and work counters are
      recomputed from the decision timeline; and every claimed terminal
      proposal must match the graph's complete typed action, realized tape
      range, target, route length, reward, and executable evidence. Six focused
      audit tests and the complete 315-test orchestration suite pass. This
      remains open only for a retained v3 audit over the diagnostic campaign.
- [ ] Compare each discovered route with the `131`-tick ordinary replay at the
      level of trajectory, velocity retention, action availability, roll/camera
      use, option duration, detour, contact-correlated slowdown, and idle or
      repeated work. This is feature/action-surface diagnosis, not permission
      to encode the human route as reward.
- [ ] Run matched scratch cells for learned ranking, scheduler-only coverage,
      and random-valid ranking. Hold source, terminal, action schema, seeds,
      horizons, proposal width, workers, and expansion/wall budgets fixed.
- [ ] For those cells report both sample efficiency and execution efficiency:
      terminal rate, useful expansions to terminal, useful expansions per
      second, simulated ticks per expansion, restore time, native simulation,
      IPC, graph admission, learner update, persistence, and idle/saturation.
- [ ] Reproduce one sealed diagnostic cell on macOS and the reference
      workstation with identical game/controller bytes and execution fidelity.
      Explain any difference in terminal, route tick, action applicability, or
      throughput before comparing learning treatments across platforms.

Exit gate:

- Evidence identifies whether the primary failure is expansion quality,
  expansion throughput, restoration/orchestration, action/feature coverage, or
  platform drift. More native work is then assigned to that measured cause.

## P2 - Prove native learning improves search

- [ ] Demonstrate on held-out native state groups that the pre-terminal learner
      ranks independently realized actions better than action-mean,
      scheduler-only, and random-valid controls. Report coverage, calibration,
      pairwise ordering, uncertainty, and unsupported actions.
- [ ] Demonstrate that learned ranking reduces median unique useful expansions
      to the real terminal under matched budgets. The deterministic
      around-the-corner fixture remains a regression test, not this gate.
- [ ] After terminal support appears, demonstrate on held-out successful-path
      states that learned total-tick ranking selects better counterfactuals
      than visit-count and random-valid controls, and recovers the
      exhaustive-local oracle's best outcome with materially fewer native
      evaluations. Exhaustive-local is an outcome ceiling and evaluation-cost
      control, not a policy the learner can honestly beat.
- [ ] Audit the observation vector against native evidence. Applicable actions,
      velocity and trajectory history, camera state, prompted actions, and
      kinematic consequences such as lost speed must be available when
      supported by state; no derived signal may directly encode desired Ordon
      behavior.
Exit gate:

- Learning reduces native expansions to terminal and improves held-out
  counterfactual selection. A faster unchanged coverage heuristic does not
  satisfy this gate.

## P3 - Make real-campaign throughput scale

- [x] Replace checkpoint v5's whole-payload identity and eager campaign clone
      with a versioned manifest over immutable graph/replay objects and
      append-only authenticated summaries. Preserve v5 read/migration support,
      but make each durable decision authenticate newly admitted content once
      and install a bounded root manifest. Add a long synthetic growth test
      proving that per-decision persistence volume does not rise linearly with
      accumulated history.
- [x] Rerun the identical one-worker, 16-decision, 256-expansion Windows cell.
      Require persistence to stop dominating wall time and compare phase
      occupancy, useful expansions per second, native ticks, graph work, and
      artifact volume with all three retained predecessor cells before
      spending a full curve.
- [ ] Run fixed-work curves at 1, 2, 4, 8, and 16 workers over enough decisions
      to exercise graph growth, repeated restores, learner updates,
      persistence, and bounded checkpoint eviction. Retain the complete
      content-bound sample reports and v4 aggregate, including phase occupancy
      and saturation counters. A one-decision warm-fleet microbenchmark is
      necessary but not sufficient.
- [x] Make the multi-sample curve resumable before another full launch.
      Commit each completed sample by ordinal, repetition, worker count,
      execution-plan identity, report identity, and useful-expansion set.
      Resume must independently validate and skip complete samples, continue a
      durable partial route sample, reject a torn or detached sample, and never
      relaunch native work merely because final aggregate sealing was
      interrupted.
      Source checkpoint `fa2770a794c5` implements the protocol but does not
      close this gate: compile it, run the focused pure tests, and retain
      interruption tests covering a completed prefix, a durable partial
      sample, detached/torn progress, and aggregate-only resealing with zero
      native launches.
      Source checkpoint `52101efb3d` fixes a fatal entry bug in the earlier
      implementation: an existing output was rejected before `--resume` could
      be honored. Resume now revalidates the physical sample roots, complete
      contiguous sample evidence, fleet-launch journal, and exact aggregate
      execution order; it rejects non-files, detached/future samples, and
      torn records. Focused pure regression sources cover fresh/resume mode,
      completed-prefix skipping, a durable partial sample, detached and
      non-prefix progress, torn records, and aggregate-only zero-launch
      resealing. Source checkpoint `97cd210747` compiles and passes all of
      those focused cases; the complete suite passes all 318 tests at
      checkpoint `3d6d9b4fe7`.
- [ ] Retain a sealed long-campaign v2 resource audit proving the declared
      memory and fitted-model staleness bounds while reporting replay fallback
      and checkpoint-owner skew. Use the hard-loss injector before dispatch,
      during execution, after native completion, after recovery-point commit,
      and after decision commit, and retain all five content-bound recovery
      audits proving exact semantic work and retry accounting. Run at least one
      fault after graph/replay growth, not only at decision zero.
- [ ] Remove the measured end-to-end saturation bottleneck until the reference
      workstation sustains enough useful evidence to meet the P4 wall-time
      gate. Prefer reducing restore/replay and idle time before adding capacity.
      The first current resumable Windows curve launch exposed a separate
      startup-pressure defect: all 16 native workers booted concurrently,
      consuming 16 cores before any sample began. Checkpoint `9d5d929294`
      launches the identical persistent fleet in contiguous batches of at most
      two, retains total fleet-launch time outside sample wall time, and passes
      the complete 319-test orchestration suite. A five-second live startup
      interval used `10.05` native CPU-seconds with exactly two active boots.
      This bounds launch pressure but does not close the end-to-end throughput
      gate. The first corrected one-worker sample then showed strong
      history-dependent coordinator cost despite only `36.3` seconds of native
      simulation in roughly `1,717.4` wall seconds. Checkpoint `e7803326dc`
      removes a duplicate whole-graph clone, admission, projection, and dirty
      persistence mark from each retained selected outcome. Checkpoint
      `d6f386d62f` makes the remaining atomic graph transaction structurally
      shared and merges only newly admitted learner rows while preserving
      byte-identical graph serialization. The exact matched cell reduced graph
      admission 99.2% and wall time 9.3%, but still spent about 196 seconds in
      authoritative batch admission. Checkpoint `354f11a46d` reduced that path
      only 6.0%, to about 184.5 seconds, while total wall regressed 4.9%;
      checkpoint `68e37a31ef` now reuses the immutable validation/identity
      receipt created during rewarded-outcome evaluation instead of
      re-authenticating every large transition during batch graph admission.
      Rerun the exact one-worker cell and require the measured admission path,
      not merely a unit microbenchmark, to fall materially before spending the
      remaining v4 cells; the curve must still identify the steady-state
      saturation point.
- [x] Preserve native state, applicable actions, controller output, terminal
      evidence, and first-hit tick for every disabled presentation subsystem.
      Source checkpoint `703dcdbbba` advances the parity report to v2 and
      makes those authorities explicit: it separately binds the native state
      trajectory, recomputed complete primitive action surface at every
      boundary, chosen and consumed controller pads, first-hit vector, and
      terminal projection for every legal retained/suppressed pair. The
      historical ignored v1 report only compared opaque episode payloads and
      cannot close this gate. Focused tests compile and pass at source
      checkpoint `97cd210747`; keep this open until a retained v2 native report
      passes.
      Source checkpoint `0f4d6e6831` adds the portable evidence path: it bundles
      every condition's exact batch request, result, and binary episode shard
      with the sealed optimization, execution binding, and report. Offline
      validation reconstructs all five authorities from bundled native bytes,
      and the clean-checkout audit automatically discovers committed manifests.
      Source checkpoint `97cd210747` compiles this path in the passing complete
      orchestration suite. Checkpoint `5a1e3e8f25` retains the passing
      `win32-x86_64` v2 report and portable bundle. All nine legal
      retained/suppressed conditions reproduce the exact native state
      trajectory, complete primitive action surfaces, controller bytes,
      first-hit vector, terminal projection/evidence, and binary episode.
      Report identity
      `89d1c04bc39d6aeafb2b577f6eed65e651ce3b82b7488c44fa2011524c2a1141`;
      bundle identity
      `f21d10b2fd0677b174e741d8ddbe59b450d2c0525b430f5691f55c9bc5dad454`.
      The clean-checkout audit validates the bundle at checkpoint
      `3d6d9b4fe7`.

Exit gate:

- Additional workers improve long-running useful expansions per second with
  bounded memory and staleness.
- The P4 time gate is feasible at the measured expansions-to-terminal count,
  rather than extrapolated from a short microbenchmark.

## P4 - Make scratch discovery practical and general

- [ ] Reach the actual load-zone predicate in all four sealed evaluation seeds
      with median time to first terminal at or below five wall minutes and
      worst-seed time at or below fifteen minutes on the reference workstation.
- [ ] Repeat the acceptance run on held-out seeds that were not used to repair
      or tune the discovery policy. Report terminal rate and confidence rather
      than treating the four development seeds as generic evidence.
- [ ] Demonstrate that adding the ordinary `131`-tick human replay improves
      median time or expansions to first terminal while remaining unnecessary
      for scratch success.
- [ ] Retain the complete P0 evidence bundle for every acceptance run.

Exit gate:

- Walking off the map is a reliable minutes-scale operation, not an overnight
  search.
- Failure sends work back to the measured P1/P2/P3 cause; it does not authorize
  more seed mining.

## P5 - Reliably improve successful routes

- [ ] Continue useful work after first terminal instead of treating discovery
      as campaign completion. Hand the exact successful graph path and terminal
      returns directly to the optimization scheduler.
      Source checkpoint `3dbf335b7d` keeps rank-zero terminal-support lanes on
      exact-return optimization while sealed nonzero acquisition ranks retain
      discovery-ordered broad exploration. Source checkpoint `62aac70a51`
      fixes the remaining terminal handoff: a terminal-triggered branch now
      restores the scheduled frontier immediately instead of unconditionally
      restarting from root, and a campaign-level test requires rank-zero
      acquisition to select the supported successful-path interior. Source
      checkpoint `e72f47009c` makes authenticated rank-zero support acquire a
      scheduled frontier every decision, while periodic root and demonstration
      refreshes remain confined to ordinary exploration branches. The complete
      315-test orchestration suite passes at source checkpoint `97cd210747`;
      this remains open until retained native traces prove post-terminal work
      and improvement rather than merely continued execution.
- [ ] Schedule counterfactuals across the complete interior-state sequence of
      every newly successful path, not option endpoints alone.
      Source checkpoint `c9bfb90865` aligns native and optional-replay
      boundaries to at most four ticks and requires terminal-path interior
      nodes to retain exact optimization-scheduler support. Source checkpoint
      `62aac70a51` removes best-route-only node support, schedules interiors
      from every authenticated terminal route, and prevents validated
      transposition canonicalization from erasing a route-specific supported
      interior. Source checkpoint `e72f47009c` removes the eight-decision
      counterfactual cadence bottleneck and advances the post-terminal control
      report to v2: each seed now retains the exact supported, leased, and
      unleased interior-node sets and can claim complete coverage only for a
      nonempty supported set with no missing lease source. Focused tests compile
      and pass at source checkpoint `97cd210747`; keep this open until a
      retained native v2 report proves complete sequence coverage.
- [ ] Demonstrate repeated monotonic best-route improvement from the first
      terminal in at least three sealed seeds. One favorable counterfactual is
      not proof. Source checkpoint `978f7ee3b1` makes the proof contract
      executable: scratch audit v3 binds each claimed improvement to matching
      action, duration, target state, route length, emitted tape, reward, and
      terminal truth in the final graph, while journal projection requires the
      policy-selected first proposal to remain the retained result. The source
      contract and focused tests pass at checkpoint `97cd210747`; this stays
      open until three retained sealed seed timelines show strict improvement.
- [ ] Beat the `131`-tick ordinary demonstration without making it incumbent or
      policy authority.
- [ ] Reach tick `123` or lower from scratch or optional-replay assistance,
      then cold-replay the complete tape twice with the learner out of loop.
      Source checkpoint `b43c547045` adds a tactic-route-specific cold replay
      command and offline validator. It requires the selected seed to be the
      report's graph-selected campaign best and at or below the sealed tick
      ceiling, then launches at least two fresh learner-free processes from
      separately retained copies of the complete controller tape. The source
      compiles in the complete passing suite at checkpoint `97cd210747`; this
      stays open pending a retained tick-`123`-or-lower proof.
      Source checkpoint `6221b966a0` adds a portable outer bundle that nests
      the independently validating scratch campaign plus only proof-referenced
      replay artifacts, requires an actual tick of `123` or lower, and is
      automatically discovered by the clean-checkout evidence gate.
- [ ] Require identical controller bytes, first-hit tick, terminal evidence,
      game identity, fixture, source boundary, and execution fidelity.
      The checkpoint above seals every per-repetition controller artifact and
      milestone result, requires identical exact boundary fingerprints and
      first-hit frames, and binds the request, execution plan, route report,
      graph/result identities, executable plus runtime dependencies, game
      data, fixture manifest, world context, source fingerprints, fixed
      automation CVars, and headless fixed-step unpaced launch contract. Its
      offline validator re-authenticates the source campaign and every retained
      byte. The portable bundle additionally cross-binds the proof to the
      bundled graph-selected terminal tape/result instead of requiring paths
      into the originating build tree. The compiled test portion passes at
      source checkpoint `97cd210747`; this remains open until native evidence
      passes.

Exit gate:

- At least three sealed seeds improve their first discovered terminal route.
- The best authenticated route is tick `123` or lower and reproduces exactly.

## P6 - Discover and reuse tactics

- [ ] Mine parameterized action compositions from successful graph paths and
      high-value counterfactuals. Direction plus camera modifier, roll cadence,
      curved steering, and prompted actions use the same generic mechanism.
      Checkpoint `934e92a8e1` fixes false composition support: connected tapes
      are now bucketed across repeated occurrences and retain only each
      occurrence's true entry state, not internal steps. Source checkpoint
      `9987099995` advances the registry to v3: candidate identity and binary
      persistence now retain the complete ordered typed primitive/parameter
      sequence plus every occurrence's full transition chain. Source
      checkpoint `79ff1f996a` advances that contract to v4, retains each
      component's bounded canonical executable source, captures those sources
      in new journals, and reconstructs legacy family actions deterministically.
      The source compiles in the passing complete suite at checkpoint
      `97cd210747`; this remains open pending native discovery evidence.
- [ ] Learn typed entry conditions from independent source states.
      Candidate construction requires at least two distinct entry-state
      identities. Source checkpoint `be0427afcd` fixes a false-generalization
      bug: stage/room, procedure, contacts, and distance evidence now remains
      in joint typed cells with per-cell distance ranges instead of fabricating
      unsupported Cartesian combinations from independent sets. Compiled tests
      pass at checkpoint `97cd210747`; this stays open until held-out native
      applicability evidence validates the learned condition.
- [ ] Promote only when a composition improves terminal/tick return on held-out
      state groups relative to executing its primitive components.
      Source checkpoint `79ff1f996a` deletes the best-single-primitive
      shortcut and executes the complete retained component sequence natively
      from the same held-out frontier under the same horizon; route report v37
      counts those executions separately and leaves the legacy shortcut count
      at zero. Source checkpoint `b23d9453a7` additionally requires paired
      dominance in every supporting held-out state, preventing cheap failures
      from offsetting slower terminal routes in aggregate tick sums. Compiled
      tests pass at checkpoint `97cd210747`; this stays open until a sealed
      native report proves every promotion comparison used that path.
- [x] Keep every valid primitive selectable after promotion. Source checkpoint
      `f350c40a73` compares the complete descriptor map before and after
      promotion, proves every primitive remains byte-for-byte identical, and
      proves an inapplicable promoted macro leaves the primitive catalog
      exactly unchanged. Source checkpoint `97cd210747` fixes the state-local
      comparison test and passes the complete 315-test orchestration suite.
- [ ] Compare promotion-enabled and primitives-only search on held-out seeds
      using terminal rate, time to first terminal, time to best route, and
      unique useful expansions per second.
- [ ] Repeat the complete discovery and improvement evaluation on a second
      native route problem before claiming a generic framework.

Exit gate:

- At least one learned composition provides reproducible held-out search value.
- The second route problem succeeds without benchmark-specific reward or
  authored route structure.

## Experiment discipline

- Every native run names the architectural invariant or exit gate it tests.
- Do not run more than one diagnostic cell after a failed architectural
  treatment. Analyze or redesign before spending another full campaign.
- Controls share source checkpoint, terminal predicate, game bytes, fixture,
  route horizon, expansion budget, worker topology, and fidelity.
- Reports distinguish terminal discovery, route promotion, best authenticated
  tick, exact graph return, generalized prediction, and exploration priority.
- Report wall time and unique useful expansions together. Faster failure is
  throughput evidence; fewer expansions at unchanged throughput is policy
  evidence.
- A terminal hit, a `125`-tick tie, or one isolated route improvement is
  progress evidence only.
