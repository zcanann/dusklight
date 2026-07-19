# Semantic oracle library

`huntctl` evaluates versioned reached/avoided classifications over decoded,
immutable gameplay traces:

```sh
huntctl oracle evaluate \
  --program tests/fixtures/automation/semantic_oracles.json \
  --trace build/run.gameplay.trace \
  --supplemental build/run.oracle-observations.json \
  --run-outcome build/run.outcome.json \
  --output build/run.oracle-results.json
```

The `dusklight-semantic-oracles/v1` program supports stage, room, inclusive
world-space region, player procedure/mode, animation resource/frame, indexed
flag, exact placed-actor state, and event identity/mode/status targets. Each
oracle declares `reached` or `avoided` polarity. Results retain the first exact
supporting observation, including simulation tick, tape frame, and typed source
facts.

```json
{
  "schema": "dusklight-semantic-oracles/v1",
  "oracles": [
    {
      "name": "entered-tunnel",
      "polarity": "reached",
      "target": {
        "kind": "region",
        "stage": "F_SP104",
        "room": 1,
        "min": [600.0, -50.0, -200.0],
        "max": [750.0, 200.0, 100.0]
      }
    },
    {
      "name": "avoid-wrong-room",
      "polarity": "avoided",
      "target": { "kind": "room", "stage": "F_SP104", "room": 2 }
    },
    {
      "name": "crawl-animation",
      "polarity": "reached",
      "target": {
        "kind": "animation", "bank": "under", "resource_id": 42,
        "frame_min": 0.0, "frame_max": 20.0
      }
    }
  ]
}
```

Reached classification needs one valid matching observation. Avoidance is a
stronger claim: the trace must be nonempty and untruncated, and the required
channel must be known (`present` or semantically `absent`) at every record.
Unavailable, truncated, or unsampled data produces `indeterminate`, never a
false avoided result. Matches are accepted only from `present` payloads, so
zero-filled absent channels cannot match a target.

Flags and broad actor populations are supplemental because they are not yet
continuous Trace v2 channels. Their bounded sidecar is keyed by simulation
tick. A reached oracle may use one sampled matching snapshot. An avoided flag
requires every queried flag at every trace tick; avoided actor state requires
a declared complete actor population at every trace tick. Missing coverage is
indeterminate. This lets native read-only snapshots feed the Rust corpus-wide
classifier without treating milestone-only samples as continuous proof.

This library is classification-only. It does not mutate gameplay, change a
terminal predicate, or promote a candidate.

## Collision and invalid-state targets

The same program also provides trace-wide safety/anomaly targets:

- `collision_crossing` detects an adjacent-position crossing of a normalized
  world plane while none of the declared collision-contact mask is present;
- `out_of_bounds` detects leaving an inclusive authored world AABB;
- `void_survival` requires a bounded consecutive logical-tick run below a
  declared height without ground contact;
- `unexpected_load` compares each pending destination with a bounded allowlist;
- `wrong_warp` detects an observed location change whose destination does not
  match the declared expected location;
- `excessive_motion` independently bounds adjacent displacement and observed
  velocity/forward speed;
- `non_finite_state` identifies the exact non-finite player field (normally the
  strict trace decoder rejects this even earlier); and
- `impossible_coordinates` applies an explicit absolute coordinate bound.

Pair/window targets retain both endpoint positions, signed plane distances,
collision flags, measured displacement/speed, consecutive void ticks, or exact
actual/expected destinations as appropriate. They require known player,
collision, or stage channels across the complete trace before an `avoided`
result is possible.

## Process, corruption, and liveness targets

Failures which can prevent a valid trace from closing use a separate
`dusklight-run-outcome/v1` sidecar. It records an optional terminal condition,
typed anomaly observations, and the domains that the producer monitored
continuously. `actor_corruption`, `slot_exhaustion`,
`watched_field_corruption`, `heap_failure`, `crash`, `hang`, `softlock`, and
`control_loss` targets retain their exact source facts rather than inferring a
failure from a missing or truncated trace.

```json
{
  "schema": "dusklight-run-outcome/v1",
  "monitored": ["actor_integrity", "actor_slots", "watched_fields", "heap",
                "progress", "control"],
  "termination": {
    "kind": "timed_out", "wall_time_millis": 30000,
    "stalled_millis": 5000, "last_simulation_tick": 812
  },
  "anomalies": [
    {
      "kind": "control_loss", "start_tick": 700, "end_tick": 812,
      "tape_frame": 699, "procedure_id": 7,
      "reason": "input ownership stayed disabled"
    }
  ]
}
```

A timeout only matches `hang` when its measured stalled wall time meets the
oracle threshold. Softlock is distinct: logical simulation ticks continue but
the native progress monitor reports no semantic progress for a bounded window.
Control loss likewise uses a bounded native monitor window. Abnormal exit code,
signal, and reason are retained for crashes; actor identity and expected/actual
values, slot counts, watched-field values, and heap allocation facts are
retained for corruption/resource failures.

For reached oracles, one typed observation is sufficient even if the run ended
abruptly. Avoidance requires the corresponding domain in `monitored`; crash
avoidance requires an explicit terminal outcome. Thus an empty sidecar or a
missing monitor never silently classifies a failed run as clean. The sidecar may
also be embedded as `run_outcome` in the supplemental observation file, but the
CLI rejects supplying it both ways.

## Progression and save-state targets

The run sidecar also carries semantic anomalies which do not belong in generic
player traces:

- `duplicate_item_reward` identifies item versus reward, numeric identity, both
  grant sources, and the observed grant count;
- `preserved_storage_state` retains the storage field, its required reset value,
  and the value which incorrectly survived a boundary;
- `event_queueing` retains the running event and the exact queued event IDs, and
  supports event-identity and minimum-depth selectors;
- `sequence_break` records the named progression sequence and exact
  expected/actual steps; and
- `save_state_anomaly` records the save slot, field, expected value, and actual
  value, including anomalies detected before the first simulation tick.

Their continuous-coverage domains are `inventory_rewards`, `storage`,
`event_queue`, `sequence`, and `save_state`. As with failure monitors, a clean
avoided classification is impossible unless the producer explicitly lists the
corresponding domain.

## Cross-run and corpus comparison

Cross-run classification has its own command and versioned evidence artifact:

```sh
huntctl oracle compare \
  --program tests/fixtures/automation/comparison_oracles.json \
  --evidence tests/fixtures/automation/comparison_evidence.json \
  --output build/comparison-oracle-results.json
```

`dusklight-comparison-evidence/v1` assigns at most one run to each of the
`headful`, `headless`, `control`, and `treatment` roles. Every run carries an
ordered stream of semantic events. An event records its logical tick, optional
tape frame, typed event kind, and lowercase SHA-256 of canonical typed facts.
The optional final boundary identity detects downstream divergence even when
the event streams remain equal.

`headful_headless_divergence` and `control_treatment_difference` compare exact
event order, logical timing, tape provenance, event kind/signature, stream
length, and final boundary identity. Results retain both sides of the first
difference. An observed prefix difference is valid evidence even if a later
failure truncated a stream; equivalence or avoided-divergence classification
requires both streams to declare `complete`.

`novel_semantic_event_signature` selects zero or more roles (zero means all)
and returns the first event signature absent from the reference catalog. The
evidence binds that catalog with `catalog_identity`, the lowercase SHA-256 of
the canonical catalog artifact, and the result repeats it. Avoided novelty
requires every selected stream to be complete, so dropped tail events cannot
be mistaken for membership in the known corpus.

## Native-to-corpus composition

Cheap native monitors should emit bounded typed observations into
`dusklight-run-outcome/v1`; they should not perform corpus scans or novelty
searches in the game loop. The Rust-side composition command converts those
observations into comparison evidence:

```sh
huntctl oracle compose \
  --manifest tests/fixtures/automation/oracle_composition.json \
  --output build/comparison-evidence.json
```

The `dusklight-oracle-composition/v1` manifest embeds one to four role-labeled
run outcomes and a `dusklight-semantic-event-catalog/v1`. Every run carries its
complete `ArtifactIdentity`. Headful/headless roles must be compatible under
the cross-fidelity policy; control/treatment roles must be compatible under the
cross-build policy. Incompatible inputs are rejected before oracle semantics
are evaluated. Composition then validates every native observation, hashes its
canonical typed facts, attaches its
logical tick and tape frame as separate provenance, adds process termination as
a semantic event, canonicalizes/deduplicates the catalog, and derives the
catalog identity. Absolute tick provenance is excluded from event signatures;
bounded durations remain semantic facts. Thus the same corruption at a
different tick has the same corpus signature while exact run comparison can
still detect its timing change.

This is the intended cost boundary: native code performs constant/bounded work
per tick and reports exact facts; offline Rust performs serialization, hashing,
cross-run joins, catalog membership, novelty classification, and report
retention. No classification path mutates the game or feeds a result back into
the executing tape.
