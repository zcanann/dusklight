# Learning framework clean-checkout audit

Run the complete local quality and retained-evidence gate from the repository
root:

```text
python ci/source_quality/audit_learning_framework.py
```

The command checks:

- the production Rust physical-line limit and debt-exception ledger;
- repository Rust formatting;
- the complete `huntctl` workspace build;
- every `dusklight-orchestration` unit and documentation test; and
- every tracked native scratch evidence bundle whose manifest uses
  `dusklight-native-tactic-scratch-evidence-bundle/v2`.

Scratch discovery validation publishes a movable bundle with:

```text
huntctl learn validate-tactic-scratch-discovery \
  --request OPTIMIZATION.json \
  --execution EXECUTION.json \
  --report ROUTE-REPORT.json \
  --output ACCEPTANCE.json \
  --bundle BUNDLE-DIR \
  --repository-root REPOSITORY
```

The bundle retains content-addressed request, execution, plan, route, campaign
audit, seed, binary lease journal, checkpoint, terminal tape/result, and small
source-authority artifacts. The campaign audit includes every decision,
proposal dispatch and terminal lease outcome, proposal count, selection
reason, learner snapshot, restore source, exhausted budget, terminal path, and
exact time/expansion point at which each shorter terminal route was first
evaluated. New reports also retain every state-local applicable action, its
fitted value and uncertainty when supported, and the selected action.
Audits mark the action-surface timeline incomplete for legacy reports instead
of reconstructing availability after the fact. They also retain a
content-bound pre-lease scheduler decision with the complete state-local
expansion queue, exact and generalized return evidence, consumed model
identity, evaluated subset, and final committed expansion. The
`scheduler_timeline_complete` flag makes missing legacy provenance explicit. A
bundle can be copied away from its originating build tree and validated
independently:

```text
huntctl learn validate-tactic-scratch-bundle --bundle BUNDLE-DIR
```

An existing route report can also be audited before acceptance:

```text
huntctl learn audit-tactic-scratch-campaign \
  --report ROUTE-REPORT.json \
  --output CAMPAIGN-AUDIT.json \
  --repository-root REPOSITORY
```

Reports written before proposal root-route lengths and exact stop-reason sets
were added remain readable, but their campaign audit marks terminal-improvement
timing or the stopping budget as legacy-unreported instead of guessing.

Audit the actual learner input and action surface from a route report plus its
generated binary corpora:

```text
huntctl learn audit-tactic-observations \
  --request OPTIMIZATION.json \
  --report ROUTE-REPORT.json \
  --input GENERATED-TRAINING.dtqc \
  --output OBSERVATION-AUDIT.json
```

This recomputes every retained feature vector bit from native facts and reports
velocity, trajectory, camera, prompt, kinematic-consequence, roll, A, and L
coverage. It also rejects authored route coordinates and benchmark-specific
policy signals.

Compare the actual pre-terminal reachability learner with matched action-mean,
production scheduler-only, and production random-valid controls:

```text
huntctl learn compare-tactic-value-controls \
  --input GENERATED-TRAINING.dtqc \
  --output HELD-OUT-CONTROLS.json
```

The state-region split measures pairwise ordering and regret on independently
realized actions at wholly held-out native states. See
[held-out-tactic-controls.md](held-out-tactic-controls.md) for the candidate
surface, calibration, and unsupported-action contracts.

Audit post-terminal total-tick ranking against later graph truth:

```text
huntctl learn audit-post-terminal-tactic-controls \
  --report ROUTE-REPORT.json \
  --output POST-TERMINAL-CONTROLS.json \
  --repository-root REPOSITORY
```

This compares decision-time learned, least-visited, and random-valid ordering.
It recognizes exhaustive-local as an oracle only when every queued candidate
has an exact terminal continuation. See
[post-terminal-tactic-controls.md](post-terminal-tactic-controls.md).

Executable and game-image bytes are deliberately not duplicated into the
bundle. Their exact SHA-256 identities, runtime dependency identities, native
source boundary, objective, process tape, milestone program, world context,
and card-fixture manifest remain sealed in the retained execution and request
authorities.
