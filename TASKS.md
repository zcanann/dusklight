# Active task: build a route optimizer that beats the incumbent

This file contains only unfinished learning-framework work. Completed work and
implementation history belong in Git and benchmark reports.

## Objective

Build a generic checkpointable search-and-learning system that:

1. starts from an authenticated native checkpoint;
2. observes typed game state and derived progress measurements;
3. proposes controller tactics and bounded tactic compositions;
4. evaluates many alternatives from retained states;
5. learns which state/action transitions are valuable;
6. discovers and promotes reusable tactics; and
7. optimizes a terminal-reaching route for the fewest native input ticks.

Ordon is the current acceptance benchmark. Its authenticated incumbent first
reaches the terminal at tick 125. A terminal reach, a reliable terminal reach,
or a 125-tick tie is diagnostic evidence only. The framework has not succeeded
on this benchmark until a machine-generated route first reaches the same
terminal in fewer than 125 ticks and reproduces from cold boot.

The browser workbench records, inspects, and replays execution graphs. It is not
a TAS authoring surface. Human recordings may supply optional experience but
must not define privileged actions, observations, state, or terminals.

## Non-negotiable architecture

- JSON is not an operational checkpoint, replay, transition, model, frontier,
  or journal format.
- Reuse the existing versioned binary envelopes, tape encoding, native episode
  shards, transition corpus, content digests, and zstd support.
- Keep serialization behind storage interfaces. In-memory learner types must
  not depend on JSON or a particular file layout.
- Small authored requests and exported human-readable reports may use JSON.
- Hot exploration records only the data required to resume, learn, and verify
  identities. Full evidence graphs and readable reports are projections
  produced after a candidate is retained.
- Every performance claim must include useful native simulation, restore,
  orchestration, and persistence time. Do not hide overhead outside the
  measured boundary.

## P6 - Beat the authenticated Ordon incumbent

- [x] Replace exact-descriptor option memorization with a state-action value or
      outcome model that shares evidence across executable controller factors.
      At minimum, the model input must represent commanded stick magnitude and
      direction changes, duration, button identity/duty/cadence, and typed
      targeting relative to the current state. Controller hashes and option IDs
      remain evidence identities, not model features.
- [x] Train that shared model from authenticated native outcomes, including
      terminal progress, realized ticks, path efficiency, velocity retention,
      commanded stalls, wall contact, momentum loss, and collision correction.
      Do not replace the terminal predicate with coordinates or let shaping
      outrank a faster authenticated terminal hit.
- [x] Prove held-out action generalization before resuming the acceptance
      search: exclude exact controller instances from training and show that
      the learner ranks productive rolling above otherwise comparable
      non-rolling motion, straighter velocity-preserving motion above needless
      turning, and measured wall-impact/momentum-loss outcomes below clean
      motion. A catalog acquisition script choosing those instances is not
      acceptable evidence.
- [x] Run the integrated system from the exact `to_ordon_spring_q125` source
      boundary with no route-specific preferred-action script. For this
      benchmark that means optimization request
      `ab863b9649094ac1f2d77f7b0c2fc1baf59b2ef23a5d64c4f9d5f9c05704861c`,
      segment `to_ordon_spring_q125`, source boundary index `506`, and native
      source fingerprint `4786d70e77b31416d02864783434c7dc`.
- [ ] Search until it produces a candidate whose first authenticated terminal
      hit is strictly earlier than tick 125.
- [ ] Minimize the complete successful controller tape without changing the
      source checkpoint, terminal predicate, game build, or execution fidelity.
- [ ] Cold-replay the complete winning tape at least twice from process boot
      with the learner and controller out of the loop.
- [ ] Require byte-identical input and identical authenticated terminal
      evidence across the cold replays.
- [ ] Publish total native simulation, wall time, worker count, peak memory,
      winning lineage, first-hit tick, and proof identities.

Acceptance:

- A machine-generated route reaches the authenticated Ordon terminal in fewer
  than 125 ticks.
- The route reproduces exactly from cold boot.
- A report from any other optimization request or source boundary is
  ineligible even when it uses the same terminal predicate. First-hit cost is
  measured relative to boundary `506`; a later residual checkpoint cannot be
  compared with the incumbent.

Anything short of both conditions is progress evidence, not success.
