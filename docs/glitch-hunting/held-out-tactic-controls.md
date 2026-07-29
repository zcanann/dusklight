# Held-out native tactic controls

Use the generated binary training corpus or a campaign checkpoint:

```text
huntctl learn compare-tactic-value-controls \
  --input GENERATED-TRAINING.dtqc \
  --output HELD-OUT-CONTROLS.json
```

The v2 report evaluates four treatments on identical whole-group splits:

- the scratch-discovery pre-terminal achieved-goal reachability model;
- the mean authenticated return of each exact executable action;
- the production `structured-non-learning` scheduler-only policy; and
- the production `random-valid` policy.

The scheduler and random controls call the same policy implementation used by
native campaigns. They are not reconstructed from action duration or catalog
order. The sealed control seed and a state-derived decision index make their
ordering reproducible.

Each test state's candidate surface contains its independently realized test
actions. Every candidate is known applicable because native execution produced
the retained transition. The report does not pretend that this subset is the
state's complete action catalog. Full-surface availability is audited
separately by `audit-tactic-observations`.

The state-region split is the decision-quality test: all actions at a spatial
state remain together, so pairwise ordering, top-action win rate, and observed
regret are meaningful. The action-realization split tests extrapolation and
unsupported-action accounting. It may have only one withheld action at a
state, in which case pairwise metrics are explicitly absent.

The pre-terminal learner is trained with achieved-goal relabeling, which
removes native terminal authority and objective reward from its acquisition
score. A disjoint validation fold fits an affine mapping from that reachability
score to authenticated terminal-conditional ticks. Test-fold error and
calibration are reported only when that mapping is identifiable. The raw score
still supplies pairwise ordering, and nearest-neighbor distance remains an
epistemic diagnostic rather than a confidence claim.

This command supplies the measurement contract for the P2 held-out gate. It
does not complete that gate until a retained native corpus shows the learned
treatment beating all three controls.
