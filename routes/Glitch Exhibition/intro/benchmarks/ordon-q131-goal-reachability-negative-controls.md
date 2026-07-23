# Ordon q131 goal-reachability negative controls

This report records the first complete, sealed negative-control sweep over the
authenticated Ordon q131 goal-trajectory corpus. It is diagnostic evidence,
not promotion evidence.

## Sealed inputs and budget

- Dataset: `b8aeea2eaba9a290b36ad87e89f8f0d7ec56d842f3efb919fde2e4ef0e827faf`
- Replay corpus: `3de1412e3ab4d1dd75a55c7dbe83b0c61c7a684e4bdc9c7f7aca5d190099efd0`
- Negative-control report: `ee757e62985c89d12dde4a0d67b623584ed24f1e34fe4866981febf3abed8d5a`
- Content blob: `d272e76750c9f85aaaa12c05a5248584c1eee525c4dcb00ee582be0f701d5451`
- Rows: 114,268 training; 13,869 validation; 15,661 test
- Episodes: 821 training; 98 validation; 110 test
- Equal learner budget per treatment: two ensemble members, one epoch, hidden
  width two, identical optimizer settings and seed
- Artifact: `build/campaigns/ordon-q131-negative-controls-v1/report.json`

The small equal learner is an information-ablation pilot. The admitted full
q131 critic remains the performance reference; this sweep does not substitute
for its 7-member, 64-epoch, width-64 training budget.

## Held-out validation results

| Treatment | Representation | Changed cells / targets | Brier | Success time MAE | Return RMSE | Tick-cost MAE |
|---|---|---:|---:|---:|---:|---:|
| State-conditioned baseline | full current v1 input | 0 / 0 | 0.13063 | 7.955 | 0.29383 | 39.070 |
| Shuffled outcomes | full target control | 0 / 105,913 | 0.23035 | 34.782 | 0.33573 | 38.920 |
| Action-only input | previous-PAD proxy only | 30,075,946 / 0 | 0.21444 | 31.557 | 0.33674 | 38.962 |
| Removed collision/geometry | contact/height/correction proxies only | 847,704 / 0 | 0.13384 | 7.418 | 0.29309 | 39.066 |
| Removed actors | not represented | 0 / 0 | 0.13063 | 7.955 | 0.29383 | 39.070 |
| Removed history | previous PAD only | 420,020 / 0 | 0.13505 | 6.756 | 0.29370 | 39.017 |
| Removed checkpoint/tape identity | not represented | 0 / 0 | 0.13063 | 7.955 | 0.29383 | 39.070 |

Relative to the equal-budget baseline, shuffled outcomes worsen held-out Brier
by 76.3%, successful time MAE by 337.3%, and return RMSE by 14.3%.
Action-only input worsens those metrics by 64.2%, 296.7%, and 14.6%.
State and authentic outcome association therefore carry signal beyond an action
frequency shortcut.

## Attribution before architecture changes

The sweep diagnoses an observation-contract boundary, not a reason to swap
learning algorithms:

- The current model has no collision mesh or surface-geometry input. Its
  collision treatment can remove only contact bits, ground/roof heights, and
  the most recent XZ collision correction. Removing those proxies does not
  materially hurt this small model.
- The current model has no non-player actor set or actor-state input. The actor
  control changes zero cells and produces the exact baseline model identity.
- The current model has no temporal observation stack or recurrent state. Its
  only history-like input is the previous raw PAD. Removing it does not hurt
  this pilot.
- Checkpoint and tape identifiers never enter the model input. Removing them
  changes zero cells and reproduces the exact baseline model identity, which is
  affirmative evidence against this identity-leakage path.
- The current consumed action is not a reachability input; “action-only” can
  retain only the previous PAD. That proxy performs much worse than state input.
- One training epoch at width two does not learn discounted tick cost, so the
  near-identical tick-cost errors are inconclusive and must not drive an
  architectural change.

The next controlled comparison should first add the missing actor, geometry,
and temporal information to the observation representation while holding the
learner and split fixed. Recurrent state, larger capacity, or a different value
algorithm is justified only after that representation comparison establishes
which missing channel improves held-out terminal metrics.

The machine report seals all six controls, exact changed-cell/target counts,
all train/validation/test metrics, source identities, and `promotion_authority:
false`.
