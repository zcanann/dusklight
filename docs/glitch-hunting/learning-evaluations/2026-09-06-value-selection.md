# Learned value was being overridden by its sign

## Finding and correction

The epsilon-greedy tactic selector treated a best learned value at or below zero
as a reason to select an unsupported action, even on non-exploratory decisions.
This was not merely a hypothetical reward convention mismatch: the active
campaign's `replay_model` fits authenticated negative ticks-to-terminal returns.
An existing test explicitly required the incorrect sign-based fallback.

The selector now bootstraps only when no applicable action has a learned value.
Otherwise its configured epsilon policy controls exploration, and exploitation
uses the highest-ranked applicable action regardless of value sign. Untried
actions remain eligible for exploratory decisions and sibling proposals.

This removes a policy override; it adds no reward shaping, route-specific rule,
or calibration threshold. Other campaign layers, including exact-graph incumbent
selection and generalized acquisition, still exist. This change does not prove
that those layers form a sufficiently effective learner.

## Evidence

- Before the fix, the regression failed with `UnsupportedBootstrap` instead of
  `Greedy`, including with epsilon set to zero.
- The selector regression covers equivalent rankings below, at, and above zero,
  learned and frozen policies, one- and two-proposal batches, and zero, partial,
  and full exploration. It checks the exploration rate and retained alternatives.
- The online around-corner fixture also checks selection from its fitted critic
  after adding an unseen executable action. This checks actual negative learned
  returns independently of the exact-graph incumbent, which could otherwise mask
  the selector bug.
- Validation: all 457 learning and 499 orchestration library tests passed after
  the production fix. The extended online around-corner test then passed with
  the additional trained-critic assertion. Builds and tests used two jobs/threads.

No native campaign result is attributed to this change. The earlier heading
evaluation used a binary built before it. Native usefulness remains unproven;
the next evaluation should distinguish value-guided choices from overrides as
well as measuring completed route quality and useful work per wall time.
