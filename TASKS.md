# Tasks: build a learning framework that learns fast enough to use

This file contains only unfinished work. Delete completed items; commits and
retained benchmark bundles are the history.

## Target

From a restorable game state, observable state, legal actions, and a real
terminal predicate, the framework must discover and improve routes without an
authored route, route-specific reward, or hidden scheduler policy.

The first acceptance problem is the Ordon load zone:

- discover it reliably from scratch;
- beat the 131-native-tick human replay;
- reach 123 ticks or lower as the first credible quality result; and
- do all of this within useful wall-clock time.

## P0: prove the learning loop

- [ ] Pin terminal, censored, unsupported-action, and stale-outcome semantics
      with end-to-end tests. Closing and reopening a campaign must reproduce
      the exact learner state and action rankings.
- [ ] Audit the learner's state/action interface. Every legal primitive and its
      parameters must be visible when available, and observations must expose
      generic evidence sufficient to infer progress and control effects:
      motion/trajectory, camera, action history, contact kinematics, prompted
      actions, and terminal evidence. Missing values must be explicit.
- [ ] Run matched learned, frozen-ranking, and random-valid native campaigns.
      Prove on development and held-out seeds that learning lowers useful
      expansions to terminal. Repair the learner before tuning route quality
      if it does not.
- [ ] Ablate ordinary human replay as optional experience. It may improve sample
      efficiency, but may not provide waypoints, route structure, an incumbent
      policy, or be required for success.

## P0: make scratch discovery reliable

- [ ] Give exploratory policies enough horizon to wander around the corner and
      find the load zone. Budget exhaustion is censored/unknown, not failure.
- [ ] Ensure concurrent workers lease distinct useful state/action expansions
      and do not collapse onto the same greedy frontier or silently exclude
      uncertain actions.
- [ ] Retain compact per-seed evidence and discover the real load-zone predicate
      from scratch on at least four development seeds and held-out seeds. No
      authored coordinates, route-shaped reward, seed mining, or authored
      action sequence is allowed.

## P0: make discovery fast enough to matter

- [ ] Profile one fixed-work native campaign by checkpoint materialization,
      restore/capture, controller preparation, simulation, IPC, scheduling,
      persistence, model fitting/publication, finalization, and worker idle
      time. The categories must account for wall time without overlap.
- [ ] Remove hot-path whole-history work and prove checkpoint reuse, replay
      admission, frontier leasing, persistence, and learner publication remain
      bounded as history grows. Add scaling tests for repaired paths.
- [ ] Run fixed-work 1/2/4/8/16-worker scaling cells after the serial path is
      sound, then meet a throughput target derived from the observed
      expansions-to-terminal distribution: five-minute median and fifteen-
      minute worst-seed scratch discovery, with headroom.

## P1: improve routes after discovery

- [ ] Continue learning after the first terminal. Branch successful trajectories
      from interior states at intervals no coarser than four native ticks and
      rank counterfactuals by complete root-to-terminal outcome while retaining
      exploration and unsuccessful evidence.
- [ ] Demonstrate strict post-terminal improvement in at least three seeds,
      then beat 131 ticks and reach 123 ticks or lower.
- [ ] Cold-play the selected tape twice without the learner and require exact
      controller bytes, terminal tick/evidence, identities, and fidelity.

## P1: keep results trustworthy and the system operable

- [ ] Replace raw-report archaeology with a compact campaign summary containing
      outcome, route ticks, useful work, learned/control comparison, phase
      timing, utilization, retries, learner lag, and dominant failure.
- [ ] Split oversized production modules by responsibility and enforce the size
      gate. In particular, separate campaign decision execution, durable
      commit/recovery, and finalization/reporting.
- [ ] Make long campaigns bounded and exactly resumable: versioned binary
      content-addressed state, fail-closed identity/schema/fidelity checks, and
      fault injection around dispatch, execution, commits, and learner
      publication. JSON is only for small requests and exported reports.
- [ ] Keep process ownership exact: a coordinator may stop only child handles
      it directly created. Never kill by process name or broad ancestry.
- [ ] Add clean-checkout validation for tests, schemas, retained evidence, and
      deterministic replay.

## P2: prove this is a general framework

- [ ] Learn parameterized action compositions from native experience and
      promote them only when they improve held-out search without reducing
      reliability or throughput. Keep primitive sequences and alternatives;
      do not author blessed tactics.
- [ ] Repeat scratch discovery and optimization on a second native route without
      changing the reward model or authoring route structure for it.

## Experiment rules

- Each experiment answers one named question with the smallest useful matched
  comparison.
- Report both sample efficiency (useful expansions to outcome) and execution
  efficiency (useful expansions per second).
- Do not treat a faster benchmark as learning unless learned ranking beats
  frozen and random controls.
- Do not encode Ordon-specific behavior such as straightness, rolling, wall
  avoidance, camera alignment, or waypoints as reward. Expose observations and
  legal actions and let the learner value them.
- Keep detailed results in immutable benchmark bundles, not in this file.
