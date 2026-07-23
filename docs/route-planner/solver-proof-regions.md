# Solver proof regions and collapse safety

Planner graph v10 can attach one solver result as a nested, navigable proof
subgraph. Planner service v46 returns that graph beside every ordinary solve
report, and **Solve selected goal** opens its `Solver proof` region in the Route
Workbench.

Solve report v15 retains the exact terminal `ContinuationIdentity` for the
primary plan and every alternative; portable solve report v14 carries those
updated per-context reports.

Each primary or alternative plan receives its own region. The projection retains:

- A plan node with terminal state identity, preferences, costs, and weakest
  evidence.
- One state node for every boundary, including the start and terminal states.
- One ordered proof-step node for every selected transition, technique, or
  writer, linked both to its causal catalog record and its source/result states.
- Separate per-plan state nodes even when two plans happen to share a raw state
  digest, so distinct frontiers never disappear through graph-node deduplication.

## Safe collapse rule

The primary plan stays expanded. An alternative is collapsed by default only
when both conditions hold:

1. Its complete terminal `ContinuationIdentity` equals the primary plan: semantic
   state digest, satisfied required actions, required/banned/preferred sequence
   progress, satisfied preferences, and route-condition unknownness all match.
2. The primary resource label is no worse in depth or on any route-cost axis.

The region records this as typed `continuation_equivalent` collapse evidence,
including the full continuation and both resource labels. Graph validation
independently checks every action/progress/preference field, nonzero digests, and
the dominance relation.

If either condition differs, the alternative remains expanded. Its
`residual_differences` record explicitly lists terminal-state, required-action,
sequence-progress, preference, route-condition, resource-label, and
weakest-evidence differences as applicable. A requested UI collapse can
therefore never be mistaken for a solver proof.

## Explored-frontier merges

The solver's `ContinuationMergeProof` records are projected into a separate
collapsed child region. Each record retains the exact continuation state and
preference identity plus the dominating and dominated resource labels. This
region may collapse only with a nonzero `proven_continuation_merges` count, and
every individual proof is revalidated for strict Pareto dominance before the
graph is emitted.

The base mechanics, authored plan regions, AND/OR predicate regions, and cycles
remain unchanged. Proof projection adds a view over solver output; it does not
author mechanics, route steps, or inferred state loss.
