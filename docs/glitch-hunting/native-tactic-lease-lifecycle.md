# Native tactic lease lifecycle

Native tactic route report v35 introduced the separation of execution attempts
from unique graph work and interior observations. Current v36 retains it.

Each seed owns an append-only binary `leases.dtql` journal. Before a proposal
batch is sent to native workers, the coordinator fsyncs an `issued` record
containing the execution-plan identity, decision index, graph expansion
identities, and lease identities. The batch subsequently receives exactly one
terminal classification:

- `completed`: native evidence was admitted and its decision record was
  durably appended;
- `retryable`: worker execution failed transiently, or an unresolved issue
  found on resume has no matching durable decision;
- `cancelled`: execution ended through the typed cancellation path; or
- `failed`: returned evidence failed plan, fact, transition, or graph-admission
  validation.

Records use bounded CBOR payloads with length and SHA-256 framing. A truncated
tail is discarded at reopen, while digest corruption, reordered events,
duplicate resolutions, and resolutions without an issue fail closed. Batch
identity includes the journal event index, so retrying the same graph action
is a new dispatch attempt rather than an overwrite.

On resume, an unresolved issue whose decision index and expansion list exactly
match a durable decision record is repaired as completed. Any other unresolved
issue becomes retryable. This closes the crash window between appending the
decision and appending its lifecycle resolution without inventing completion.

The exported `lease_accounting` reports:

- proposal dispatches;
- completed, retryable, cancelled, failed, and unresolved leases; and
- the exact journal digest.

These are deliberately different from
`graph.completed_expansions` (unique admitted node/action work) and
`graph.observed_segments` (interior state transitions). Scratch acceptance
requires every issued lease to be resolved and recomputes the accounting sum.
The self-contained evidence bundle carries the exact binary lease journal and
checks it against the reported digest.

This journal makes lost dispatch attempts visible. Its ordering with campaign
checkpoints and the decision journal is specified in
`native-tactic-crash-recovery.md`; native fault-injection evidence remains a P3
requirement.
