# Native tactic crash recovery

Native tactic campaigns commit each completed decision through three durable
records:

1. a content-addressed campaign checkpoint and binary `recovery.dtrc`
   manifest;
2. the append-only decision journal; and
3. the terminal classification in the binary lease journal.

The recovery manifest binds the exact decision index, checkpoint file and
checkpoint digest to the seed's accumulated performance and restore
accounting. It uses bounded CBOR with version, length and SHA-256 framing.
Checkpoint files and manifests are fsynced before their partial directory is
atomically renamed to `recovery-checkpoints/decision-N`. On Unix, the recovery
directory is then fsynced. A completed decision is ordered as:

1. fsync the proposal lease before dispatch;
2. execute and admit the realized native evidence to the in-memory graph;
3. persist recovery point `N + 1`;
4. fsync decision-journal record `N`;
5. publish the committed rows to shared learner replay;
6. fsync the lease's `completed` resolution; and
7. prune recovery points other than `N + 1`.

Recovery uses the count of valid decision-journal records as its transaction
index. It loads only the recovery point with that exact index; it never selects
the numerically latest checkpoint. The campaign checkpoint, execution plan,
feature and objective identities, graph history, performance counters, and
decision journal must agree before native work resumes.

This produces explicit behavior for each crash window:

| Failure point | Durable decision count | Recovery behavior |
| --- | ---: | --- |
| After lease issue, before native dispatch | `N` | Resume checkpoint `N`; the unresolved lease becomes retryable. |
| During native execution | `N` | Resume checkpoint `N`; unresolved lease becomes retryable. |
| After native completion, before recovery `N+1` | `N` | Resume checkpoint `N`; work is retried rather than invented. |
| After recovery `N+1`, before decision append | `N` | Ignore and prune the ahead checkpoint; retry the lease. |
| After decision append, before replay publication or lease resolution | `N+1` | Resume checkpoint `N+1`; repair shared replay idempotently and reconcile the exact lease as completed. |
| After lease resolution, before pruning | `N+1` | Resume checkpoint `N+1`; stale recovery point `N` is pruned. |

Torn decision- or lease-journal tails are ignored only where their framing
proves them incomplete. Digest corruption, detached counters, unsafe paths, and
structural disagreement fail closed. Partial recovery directories never count
as resumable state and are pruned after a valid exact recovery point is loaded.

The deterministic tests prove binary manifest integrity, checkpoint round
trip, path containment, partial-work pruning, native-attempt pruning, shared
replay repair, and exact counter agreement.

## Native process-loss proof

`huntctl learn tactic-route` accepts a one-shot hard-loss injector:

```text
--fault-inject before-dispatch
--fault-inject during-execution
--fault-inject after-native-completion
--fault-inject after-recovery-point-commit
--fault-inject after-decision-commit
```

`--fault-decision N` selects the decision boundary. The injector first writes
`seed-*/fault-injection.json`, then exits the entire orchestrator with status
`86`, without running Rust destructors. The durable marker makes the fault
one-shot: rerun the identical command with `--resume` to recover instead of
injecting it again.

Compare the recovered report to an unfaulted report from the identical sealed
execution plan:

```text
huntctl learn audit-tactic-fault-recovery \
  --control-report CONTROL/report.json \
  --recovered-report RECOVERED/report.json \
  --output RECOVERED/fault-recovery-audit.json
```

The current v2 content-bound audit rejects campaign-identity drift, semantic
decision-trace drift, exact state-graph, replay-snapshot, or learner-authority
drift, useful-expansion or graph/replay-shape drift, terminal-result drift,
unresolved leases, and any retry count other than the exact expected batch.
The semantic identity covers the complete typed decision and proposal trace,
including rewards, measurements, value estimates, applicable actions,
parameterized feedback, state facts, and emitted tapes. It excludes only wall
time and physical worker/checkpoint placement, which may legitimately change
after process loss. Legacy v1 audits remain readable but do not satisfy the
long-campaign evidence gate.

The four pre-decision-commit faults must add exactly one dispatched batch of
retryable leases. The post-decision-commit fault must add none.

The five one-decision Windows native cells pass this audit. That local smoke
proves the harness and repaired transaction boundaries, but it is not the
retained long-campaign, cross-platform evidence required by `TASKS.md`.
