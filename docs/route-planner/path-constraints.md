# Route path constraints

Route-book v7 distinguishes terminal requirements from invariants over the
entire explored path.

- `require_predicate` is an end-state condition checked with the goal.
- `forbid_predicate` rejects every state where the predicate is true.
- `maintain_predicate` requires the predicate to remain true at the start and
  after every action. False states are pruned; unknown states remain unknown and
  cannot pass as preserved.
- `require_transition` requires one exact transition to occur before the route
  can finish.
- `forbid_transition` prevents one exact transition from executing.
- Required/forbidden techniques, minimum evidence, and per-axis cost ceilings
  retain their existing typed forms.

This makes constraints such as “preserve Faron twilight,” “remain on file 0,”
and “never use this save transition” path properties rather than terminal-state
approximations. Transition constraints participate in backward relevance and
continuation progress just like pinned/banned actions. Maintained predicates
also become relevance roots, but a false initial invariant cannot be repaired
later: the initial search state is checked before expansion.

All constraint references are validated against the exact composed catalog and
their route-book scope must be contained by the referenced fact or transition
scope. Route evidence and suite-coverage reports include maintained-predicate
facts and required/forbidden transition contracts.
