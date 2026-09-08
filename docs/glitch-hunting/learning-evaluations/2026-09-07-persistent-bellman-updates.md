# Preserve Bellman learning across updates and recovery

## Problem addressed

The [native comparison](2026-09-07-bellman-native-comparison.md) did not show a
useful learned policy. Its follow-up counterexample established a specific
limitation: twelve cold backups can prefer a one-tick self-loop to a known
twenty-tick goal-reaching action, even with complete, unambiguous evidence.
Repeated cold fits never extend those twelve backups.

## Change

The campaign's single learner authority now continues Bellman fitting from
its previous values for the V2 and V3 Bellman treatments. The supplied iteration
budget is an update budget, not a fresh zero-initialized horizon on every
publication. The feature width, action head and discount must be compatible;
the authority also binds the prior to the execution, objective, root and exact
ordered replay prefix. An update cannot silently reuse values from another
task or rewrite its prior experience.

No reward, goal predicate, candidate family, exploration horizon, or default
treatment changed. V3 retains its documented time-cost and auxiliary-goal
semantics. Its achieved-goal set can change as experience grows; cumulative
backup counts are not a claim of that many steps of exact planning on one
unchanging dataset.

## Durable state and introspection

The actual forest is now recoverable binary state, stored separately in the
existing content-addressed store. The small V8 learner snapshot manifest binds
its digest, previous snapshot, cumulative backup count, and last maximum target
residual. State is written before the manifest and durable learner head are
published. New state therefore cannot become the live head before its model
bytes have been stored.

The residual measures the largest absolute Bellman target minus prior
prediction over the last update's training rows. It is not held-out accuracy,
a calibrated uncertainty, or proof of convergence. No automatic convergence
threshold or benchmark-specific update count is introduced.

Restart loads the saved Bellman regressor. Historical snapshot lookup loads
that historical model rather than fitting its replay with the current learner
head. Loading still reconstructs the separate legacy/shadow option model;
this change does not claim that all restart fitting has been eliminated.
Missing or corrupt Bellman state fails explicitly, without falling back to a
cold fit. Blob size is bounded at 16 MiB; restored forests validate action/tree
shape, split indices, depth, finite values and training metadata before use.

The managed campaign's public snapshot view returns the deployed manifest,
including its Bellman state reference. Local implicit Bellman refits during
action selection are disabled: these campaigns must consume a managed learner
snapshot. Explicit pure `fit_*` APIs remain cold fits for standalone callers
and comparisons; continuing callers use the update API and retain its state.
Old V7 snapshots retain their cold reconstruction path and can seed a new
stateful update. They do not retroactively acquire the new behavior.

## Evidence

Verification passed: 473 learning, 503 orchestration and 117 evidence library
tests, with test concurrency capped at two. The CLI's all-targets check also
passed; its existing unused-import/dead-code warnings in `tests/harness_cli.rs`
remain. The orchestration suite was rerun after the final snapshot-view and
unmanaged-selection changes.

- The kernel's two twelve-backup updates, with a binary serialization/reload
  between them, match one uninterrupted twenty-four-backup fit byte-for-byte.
  They stop preferring the self-loop in the fully observed counterexample.
  Incompatible discounts and malformed restored forests are rejected.
- A campaign-authority test publishes real-format native-fact transitions,
  saves two successive models, reopens the durable replay/head, and recovers
  the exact model without performing another Bellman update. Historical lookup
  does not move the head. The next update after restart matches an uninterrupted
  update, including model bytes and snapshot identity.
- The same test removes and corrupts the current model blob in its private
  temporary store. Both restart attempts fail; restoring the blob recovers the
  original head. No user campaign artifacts are modified by these checks.
- V2 and V3 snapshot integration tests continue fitting, consume the resulting
  snapshot, and select executable actions without native-success evidence.
  They also check that unmanaged implicit cold selection is rejected and the
  public manifest matches the consumed model.

## What remains unproven

This fixes a concrete update/recovery defect, not the whole learning problem.
Continued approximate Bellman updates may still generalize badly or fail to
converge, and maximizing over unobserved actions can still produce misleading
values. The new counters make persistence and update behavior inspectable;
they do not establish better routes.

The September 7 comparison remains the native route-quality evidence. No new
game campaign or optimized rebuild was launched for this milestone. Next,
compare the stateful learner under a bounded native budget before claiming an
improvement. Keep the general TASKS.md capability outcomes open.
