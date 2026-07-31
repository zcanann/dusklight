# Learning framework tasks

Track only unfinished work required to build a generic learner that actually
learns routes and learns them fast enough to be useful. Delete completed items;
git history and retained benchmark bundles are the record.

`TASKS_ROUTE_PLANNER.md` is a separate product backlog.

## Success bar

- Learn from a restorable native state, generic observations, legal actions,
  and a real terminal predicate—without authored waypoints, route scripts, or
  route-shaped rewards.
- Beat frozen/non-learning and random-valid controls on development and
  held-out seeds. Human replay may help but may not be required or cap quality.
- Find the Ordon load zone in a five-minute median and within fifteen minutes
  on every retained seed.
- Continue improving after first success, beat the 131-tick human replay, and
  reach 123 native ticks or fewer with exact cold replay.
- Repeat scratch discovery and optimization on a second route without changing
  the learner or its objective.

Ordon is an acceptance test, not a reward specification. Coordinates,
waypoints, straightness, rolling, wall contact, and camera alignment are not
reward terms; they are observable facts or available actions when native state
supports them.

## P0 — Establish real learning

- [ ] Complete the checkpoint proof with a fault-injected native interruption
      and show that the resumed campaign exactly matches an uninterrupted
      control.
- [ ] Run identical-budget learned, frozen/non-learning, and random-valid
      campaigns on development and held-out seeds. Pin all inputs and report
      terminal rate plus useful expansions and wall time to first terminal.
- [ ] Fix the learning loop until learned treatment beats both controls. Audit
      observation/action availability, horizon and censoring, credit assignment,
      exploration, experience publication, policy lag, duplicate worker effort,
      and any scheduler behavior exposed by the comparison.
- [ ] Ablate the ordinary suboptimal human replay. Show whether it improves
      sample efficiency while remaining optional and surpassable.

## P1 — Reach useful throughput

- [ ] Emit one compact campaign summary: outcome, route ticks, expansions and
      time to terminal, expansions/second, control deltas, phase timing, worker
      utilization, learner lag, retries, and dominant failure.
- [ ] Profile fixed work with non-overlapping time attributed to native state
      handling, simulation, IPC, scheduling, learning, persistence,
      finalization, and idle time.
- [ ] Fix the measured bottlenecks and prove useful fixed-work scaling across
      the practical worker counts for the machine. Eliminate waste from
      duplicate search, serialization, contention, whole-history work, and
      unbounded memory or persistence growth.
- [ ] Meet the five-minute median and fifteen-minute worst-seed scratch
      discovery targets on retained development and held-out seeds.

## P2 — Improve routes after discovery

- [ ] Keep learning from successful trajectories after first terminal and show
      repeatable post-success improvement without collapsing exploration.
- [ ] Beat 131 ticks and reach 123 ticks or fewer. Cold-play the selected route
      twice with identical controller bytes, terminal evidence, native-tick
      count, identities, and replay fidelity.

## P3 — Keep the framework auditable and maintainable

- [ ] Make orchestration ownership explicit: exact child handles, bounded CPU
      and memory, cancellation propagation, and no process-name or
      broad-ancestry discovery or termination.
- [ ] Split oversized modules along execution, learning,
      persistence/recovery, transport, and reporting boundaries. Enforce a
      source-size gate and independently test and profile each boundary.
- [ ] Add clean-checkout validation for schemas, matched controls,
      deterministic replay, interrupted recovery, retained evidence, bounded
      history growth, and campaign summaries.
- [ ] Let the learner test parameterized and composed legal actions while
      retaining primitives and rejected evidence. Promote compositions only
      when held-out results improve; do not author a blessed tactic list.
- [ ] Repeat scratch discovery and post-success optimization on a second native
      route with the same observations, objective, and learner implementation.

## Evidence rules

- Run the smallest matched experiment that answers one named question.
- Report sample efficiency and execution efficiency separately.
- A fast route is not evidence of learning unless learned treatment beats both
  controls.
- Architecture work must remove a measured bottleneck, close a correctness
  hole, or make the learning claim easier to audit.
- Keep durable hot-path state versioned and binary. Limit JSON to small requests
  and human-facing exported reports.
