# Post-terminal tactic controls

Evaluate decision-time optimization rankings against later authenticated graph
outcomes:

```text
huntctl learn audit-post-terminal-tactic-controls \
  --report ROUTE-REPORT.json \
  --output POST-TERMINAL-CONTROLS.json \
  --repository-root REPOSITORY
```

The audit joins two authorities without allowing either to rewrite the other:

- each optimization decision's retained pre-lease scheduler queue supplies the
  learner model identity, predicted terminal support, conditional ticks,
  uncertainty, visits, and exact candidate set;
- the seed's final binary graph checkpoint supplies completed executable
  expansions and exact authenticated continuations to terminal.

Report v2 also proves scheduler coverage rather than inferring it from decision
counts. For each seed it derives every executable, non-root, nonterminal
interior on every authenticated terminal tape from the final graph, resolves
every evaluated lease in the retained trace back to its exact source node, and
retains the supported, leased, and unleased node sets. A seed reports
`complete_supported_interior_coverage` only when the supported set is nonempty
and the exact unleased set is empty.

For each successful-path source queue, the report compares:

- predicted total root-to-terminal ticks;
- least completed visits; and
- a content-seeded random-valid order.

It reports the selected action's observed total ticks and regret, plus how many
ordered evaluations each treatment would require before reaching the best
observed local outcome. Unknown or censored continuations are not failures.

Exhaustive-local is an oracle only when every queued action has a completed
executable expansion with an exact terminal continuation. The report exposes
`exhaustive_surface_complete` and withholds
`exhaustive_local_evaluations` otherwise. This prevents a partially explored
queue from masquerading as proof that the learner recovered the local optimum.

The command provides the measurement contract for the post-terminal P2 gate.
The gate remains open until retained native campaigns contain comparable
successful-path decisions and demonstrate both better ranking than visit and
random controls and cheaper recovery of the complete exhaustive-local best.
