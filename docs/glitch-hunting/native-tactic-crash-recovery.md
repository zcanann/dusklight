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

1. admit the realized native evidence to the in-memory graph;
2. persist recovery point `N + 1`;
3. fsync decision-journal record `N`;
4. fsync the lease's `completed` resolution; and
5. prune recovery points other than `N + 1`.

Recovery uses the count of valid decision-journal records as its transaction
index. It loads only the recovery point with that exact index; it never selects
the numerically latest checkpoint. The campaign checkpoint, execution plan,
feature and objective identities, graph history, performance counters, and
decision journal must agree before native work resumes.

This produces explicit behavior for each crash window:

| Failure point | Durable decision count | Recovery behavior |
| --- | ---: | --- |
| Before native dispatch | `N` | Resume checkpoint `N`; no lease exists. |
| During native execution | `N` | Resume checkpoint `N`; unresolved lease becomes retryable. |
| After native completion, before recovery `N+1` | `N` | Resume checkpoint `N`; work is retried rather than invented. |
| After recovery `N+1`, before decision append | `N` | Ignore and prune the ahead checkpoint; retry the lease. |
| After decision append, before lease resolution | `N+1` | Resume checkpoint `N+1`; exact lease/decision reconciliation marks completion. |
| After lease resolution, before pruning | `N+1` | Resume checkpoint `N+1`; stale recovery point `N` is pruned. |

Torn decision- or lease-journal tails are ignored only where their framing
proves them incomplete. Digest corruption, detached counters, unsafe paths, and
structural disagreement fail closed. Partial recovery directories never count
as resumable state and are pruned after a valid exact recovery point is loaded.

The deterministic tests prove binary manifest integrity, checkpoint round
trip, path containment, partial-work pruning, and exact counter agreement.
Native fault-injection evidence is still required to prove the same ordering
against real worker-process loss and is tracked separately in `TASKS.md`.
