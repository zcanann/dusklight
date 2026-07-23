# Ordon q131 goal-learning checkpoints

This report projects the complete authenticated q131 v2 learning loop onto one
sealed row per committed checkpoint. It is diagnostic evidence, never promotion
evidence.

## Sealed lineage

- Loop state: `f20fdaf22213bb2e62edcef37fafe6d164aa393ba9b9b8d27ac26140952a6634`
- Checkpoint report: `1ea9250e745031e48dd04c1210def19288e7860448b83903d30022098c463494`
- Content blob: `020779f76136704d6022a242abcd627e72fc3a560be9679b4e384b1eb3e9138b`
- Charged native ticks: 1,920
- Artifact: `build/campaigns/ordon-q131-goal-learning-v2/checkpoint-performance-v1.json`

The report replays the learning journal and authenticates each referenced
critic, policy manifest, frozen model, and collapse report before joining their
metrics. It cannot be assembled from a stale summary file.

## Held-out performance and realized coverage

| Generation | Native success | Validation Brier | Success-time MAE | Reach / return disagreement | Parent states | PAD actions | Action trajectories | State identities | Contact signatures |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 0 / 4 | 0.08708 | 1.894 | 0.02979 / 0.02546 | 1 | 140 | 1 | 161 | 1 |
| 2 | 0 / 4 | 0.08763 | 1.992 | 0.03864 / 0.02933 | 1 | 159 | 1 | 161 | 2 |
| 3 | 0 / 4 | 0.08843 | 1.484 | 0.03249 / 0.02758 | 1 | 113 | 1 | 161 | 1 |

All three critics materially beat their training-mean baselines on held-out
reachability and time-to-go. All twelve online policy executions nevertheless
miss the terminal. Every generation begins from one checkpoint, repeats one
action trajectory across its four deterministic rollouts, and is marked
collapsed even though each trajectory contains 113--159 distinct consumed PAD
values and visits 161 distinct state identities.

## Failure attribution

The controlled evidence points to exploration/population collapse plus sparse
online terminal coverage, not an inference-boundary or throughput failure:

- every native action was independently reinferred byte-exactly in Rust before
  admission;
- the frozen policies use the complete factorized PAD surface and emit many
  distinct actions, so this is not an unsupported-action fallback;
- 1,920 ticks completed through persistent workers, so host execution was not
  the limiting event;
- offline critic calibration remains good while online success stays zero,
  separating prediction quality on the existing corpus from useful proposal
  diversity;
- the single parent state, single action trajectory, and near-single contact
  signature identify the measured population bottleneck.

The next comparison should vary rollout seed/checkpoint distribution or
explicitly diversify policy proposals while holding the critic, action surface,
native budget, and terminal authority fixed. A larger or different value model
is not justified by this result alone.

