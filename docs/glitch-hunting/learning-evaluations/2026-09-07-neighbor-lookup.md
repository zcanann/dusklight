# Action evidence lost during nearest-neighbor lookup

## Finding

The generalized critic selected at most 16 nearest **state** rows before
considering action distance. This was not equivalent to its stated joint
state-action distance metric. Relabeling one physical transition against many
goals could fill the shortlist and exclude relevant examples of another action.

This affects the shared generalized critic, not only the experimental hindsight
treatment. It is a lookup defect; fixing it does not establish that conditional
return regression is an adequate learning algorithm.

## Recorded-experience inspection

Used the first hindsight seed's final checkpoint from the September 6 bounded
comparison, with 128 training transitions and 74 distinct recorded descriptors.
Queried training row 0 against the unchanged goal coordinate. Its distance to
the requested goal was 1,646 units. This is a retrospective fit to the complete
checkpoint, not a reconstruction of an online decision or an evaluation on
withheld experience. Recorded descriptors are diagnostic candidates, not an
assertion that all are currently applicable.

Before the change, the top five predictions all used the same eight training
examples. Their labels concerned achieved goals at distances 715–1,273, with
returns ranging from -33 to -302 ticks. The leading prediction was -233.64 for
`family/camera-lock-roll-forward/3f2b7536f6e98608c4b4`. Its nearest joint distance
was 0.684, versus 0.400 for a lower-ranked candidate using the same examples.
Different weighted averages of the same examples can change rankings without
establishing evidence that the preferred action is better.

After the change, the same query selects
`family/camera-lock-roll-forward/82bc5a4a0d27095d341f`, with predicted return
-80.07 and nearest joint distance 0.402. Previously excluded examples with
zero action distance now enter its neighborhood. The former leading candidate
now predicts -110.29 from a different neighborhood.

This is not a route improvement. About 61% of the new leading prediction's
weight comes from 16-tick returns to nearby achieved goals, while the requested
goal remains 1,646 units away. The lookup now follows its metric, but a more
optimistic estimate need not be more accurate. Goal conditioning and the use
of success-conditional targets remain important limitations to investigate
using these explanations, before interpreting scores as requested-goal values.

## Change

Remove the state-only count cutoff. Search the sorted state cohort for the
exact eight nearest joint state-action neighbors, maintaining a bounded sorted
buffer. State distance is a lower bound on joint distance, so scanning stops
only when remaining rows cannot enter that buffer. The existing exact-state
locality rule remains unchanged; this does not replace the model's distance
definition, rewards, goal labels, or exploration policy.

Expose an on-demand explanation of the actual prediction neighbors: training
sample, state/action distances, normalized weight, features, and outcome. The
explanation and deployed prediction share the same lookup implementation. It
adds no campaign-time logging or bulk serialization path.

## Reproduction

The read-only example loads and validates the binary checkpoint without
launching the game or rewriting evidence:

```powershell
cargo run --manifest-path tools/huntctl/Cargo.toml -p dusklight-orchestration --example explain_hindsight -j 2 -- build/campaigns/ordon-hindsight-comparison-20260906/hindsight/seed-000-104729/final-checkpoint/tactic-q-47075f7cb89dff90e2da2995098b29c902d03d364504bc3b0b97a9aee8208ee1.dtqz -1655.7073 728.89667 -4232.1816
```

The optional last argument selects another training row. Output is a small
human-facing JSON diagnostic; checkpoint and replay storage remain binary.

## Verification and interpretation

The regression case places 32 rows of one action ahead of relevant examples
of another in state distance. The latter now contribute to the prediction and
the higher-return action ranks first. A separate matrix compares the bounded
lookup with exhaustive joint-distance sorting, including exact states, unseen
states, ties, and distant queries. Explanation weights reconstruct the deployed
reward prediction.

All 462 learning and 500 orchestration unit tests passed after the change.

Native route-quality improvement remains untested. The September 6 results
still stand. The broader question remains whether the critic's targets and
generalization teach useful choices, not merely whether an update changes the
choice. No additional native campaign or benchmark tuning is part of this
milestone.
