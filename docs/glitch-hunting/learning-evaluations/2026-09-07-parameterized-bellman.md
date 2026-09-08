# Bellman learning from incomplete parameterized attempts

## Why this change

The experimental hindsight critic regresses costs of recorded continuations
that reached a relabeled goal. Unconnected attempts are omitted for that goal.
The continuous V1 forest and Double-Q controls similarly regress completed-path
costs. Their model names do not mean their live fitting path performs Bellman
updates on unfinished experience.

That is materially different from transition-level hindsight replay: change the
goal, recompute its reward, and train an off-policy learner on those transitions.
See the [HER paper, sections 2–3](https://arxiv.org/pdf/1707.01495).

## Implemented

`--value-treatment continuous_bellman_forest` selects an explicit V2 treatment.
It uses the existing regression forest and state/action feature encoder, with
duration-aware Bellman targets:

`observed_reward + discount^duration * max Q(next_state, available_candidate)`

Every physical transition is included, whether or not replay contains a
completed route. Only a real terminal suppresses the continuation term.
Nonterminal rollout cutoffs still bootstrap; missing successor actions are an
error, not an implicit terminal or fabricated failure. Initialization is zero
before the first fit, not a claim about the eventual return of unfinished paths.

The native bridge builds successor candidates through the existing parameterized
action generator and `LearnerState` applicability checks. It uses the successor
camera and prompted-action availability, not the previous state's mask. The
goal coordinate is reconstructed from the authenticated target-relative
observation. Rewards and native terminal criteria are unchanged.

The treatment is wired through immutable snapshots, live action selection,
the unmanaged fitting cache, and learned frontier ranking. Preterminal frontier
value support has a general internal name; its historical serialization key
is retained for existing evidence authentication. The default treatment has
not changed. Older treatment IDs keep their recorded meanings for comparisons.

## Evidence

- Final library run: 465 learning and 501 orchestration tests passed, with
  test concurrency capped at two threads.
- Kernel tests verify duration discounting, terminal handling, exclusion of an
  unavailable high-value successor, and bootstrapping at nonterminal cutoffs.
- A native-fact fixture with no completed route fits successfully; additional
  backups change its values using continued costs.
- An orchestration test fits an immutable snapshot with no terminal experience,
  consumes it, and selects an executable unseen action through the learned
  path without a motion-calibration gate. An initial test-fixture configuration
  mismatch was rejected by the existing identity check and corrected in the
  fixture.
- An offline fit used all 128 transitions from the failed first hindsight seed
  in the September 6 comparison, with zero terminal rows. Four backups took
  6.063 seconds in the unoptimized development build, excluding checkpoint
  loading and ranking. It produced finite estimates for 74 recorded descriptors.
  This is not a native-throughput measurement or a route-quality result.

Reproduce that read-only inspection by appending `--bellman` to the checkpoint
command in [the neighbor-lookup note](2026-09-07-neighbor-lookup.md). The inspector
uses four backups for this bounded diagnostic. Its recorded-action query set
is not an assertion that every queried action is currently applicable. Live
selection uses the real applicability mask.

## Limits and next decision

This treatment currently learns the authored objective; transition-level
hindsight augmentation is not implemented here. It therefore does not yet
resolve sparse-reward discovery before any authored-goal success. Nor does it
repair or promote the separate conditional hindsight treatment.

Successor maximization uses a bounded sample of the built-in parameterized
control families, not every possible composition or promoted macro. The
successor-feature cache fails explicitly above 64 million scalar values
(256 MB); broader replay needs bounded sampling or a more compact query cache.
The fitting rounds follow the configured iteration budget, not an assumption
that recorded paths end at that many steps.

The next learning change should feed correctly relabeled transitions into this
same Bellman path, preserving task semantics and nonterminal continuation.
After that, compare actual discovery against the nonlearning control under a
bounded budget. No native campaign was launched for this milestone, and no
general learning capability task is marked complete.
