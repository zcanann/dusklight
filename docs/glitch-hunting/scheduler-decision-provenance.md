# Native tactic scheduler decision provenance

Every new native tactic decision retains the state-local expansion queue that
existed immediately before the graph leases were issued. This closes an
important audit gap: the generic scheduler had deterministic restart tests,
but live campaigns previously retained only the selected option and a lossy
frontier summary.

The `scheduler_decision` record is bound to:

- the pre-lease binary state-graph digest;
- the consumed learner-model digest and graph generation;
- the discovery or optimization regime;
- every eligible state-local expansion in deterministic queue order;
- exact authenticated terminal ticks-to-go, when available;
- generalized terminal support and conditional ticks-to-go;
- uncertainty, prediction error, visits, and policy rank;
- global and state-local exploration ranks;
- the subset leased for native evaluation; and
- the first leased expansion, which is the policy's final committed selection.

All ranking quantities are integers. The complete queue has its own digest, so
platform float formatting cannot change its identity. Journal projection
rejects a malformed queue or one bound to a different learner-model digest.
Scratch campaign audits expose the queue per decision and publish
`scheduler_timeline_complete`; legacy campaigns decode with missing provenance
rather than appearing complete.

The orchestration regression test restores the same pre-lease campaign from
its binary checkpoint, repeats scheduling with the same model identity, and
requires byte-independent equality of the resulting decision provenance.

