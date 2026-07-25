# Tactic-route hot-path artifact inventory

Date: 2026-07-25

This report inventories every artifact family written by the native
`tactic-route` runner. The classification follows the storage categories in
`TASKS.md`; “scratch” identifies files that are needed only while one native
evaluation is being consumed and therefore do not belong in the durable
campaign store.

The authoritative write paths are:

- `run_native_tactic_route` and `run_seed` in
  `tools/huntctl/crates/orchestration/src/native_tactic_route_runner.rs`;
- `execute_selected_tactic` and its static/reactive implementations in
  `tools/huntctl/crates/orchestration/src/native_tactic_worker.rs`;
- `NativeSuffixWorkerSession::run_batch` in
  `tools/huntctl/crates/orchestration/src/native_suffix_worker.rs`;
- `TacticQCampaign::write_checkpoint` and `final_result` in
  `tools/huntctl/crates/orchestration/src/tactic_q_campaign.rs`; and
- the lifecycle marker and read-side projection functions in
  `tools/huntctl/crates/workbench/src/tactic_route_runtime.rs`.

## Measured shape

The current v6 Ordon exit-approach campaign
`build/campaigns/ordon-exit-approach-second-pair-20260725/tactic-route-v5`
made four retained decisions over 89 native ticks. It wrote 274 files totaling
13,772,714 bytes:

| Artifact family | Files | Bytes |
| --- | ---: | ---: |
| Successful final-result JSON | 1 | 6,293,601 |
| Monolithic tactic-Q checkpoint JSON | 1 | 2,841,814 |
| Native episode shards | 82 | 2,325,649 |
| Native request JSON | 82 | 952,216 |
| Native result JSON | 82 | 572,188 |
| Engine state, milestones, and logs | 5 | 204,161 |
| Final report JSON | 1 | 197,000 |
| Seed-result JSON | 1 | 166,514 |
| Per-decision trace JSON | 4 | 140,180 |
| Initial probe request, result, and shard | 3 | 64,812 |
| Final and periodic graph JSON | 2 | 9,878 |
| Accumulated edge tapes | 4 | 2,238 |
| Decision-summary JSON | 4 | 1,275 |
| Performance JSON | 1 | 612 |
| Successful tape | 1 | 576 |

The 82 native request/result/shard triplets came from only four logical
decisions. A reactive tactic currently executes a separate prefix replay for
each local tactic tick and leaves all three files behind.

The older 70-decision Ordon campaign
`build/campaigns/ordon-tactic-q-seed181081-refit32-fixed-20260724` wrote 227
files totaling 340,644,443 bytes. Its final graph alone was 182,784,563 bytes,
its final result was 97,873,647 bytes, and its final checkpoint was 42,411,657
bytes. This demonstrates the superlinear duplication in the current store; it
is not a projection based on the four-decision sample.

## Classification

| Artifact or path | Write frequency | Intrinsic class | Current dependency | Required disposition |
| --- | --- | --- | --- | --- |
| `attempts/attempt-N/initial/request.json` | Once per invocation | Scratch | Launches and authenticates the persistent worker | Consume, then discard; preserve only request and worker identities in the durable store. |
| `attempts/attempt-N/initial/result.json` | Once per invocation | Scratch | Builds the authenticated initial fact boundary | Consume, then discard after the initial snapshot and identities are stored. |
| `attempts/attempt-N/initial/result.json.episodes.dseps` | Once per invocation | Scratch | Supplies the initial native observation | Do not copy into every campaign. Retain by digest only if admitted to the learning corpus or candidate proof. |
| `attempts/attempt-N/native-state/**` | Runtime lifetime | Scratch | Engine logs, milestone output, and process-local automation state | Keep outside the durable campaign store and clean after shutdown; promote only explicit failure diagnostics. |
| `attempts/attempt-N/renderer-cache/**` | Runtime lifetime | Scratch | Renderer/runtime cache | Reuse as a cache; never journal or checkpoint it. |
| `seed-*/native/decision-N/request*` | One per static/controller tactic, up to one per local tick for iterative tactics | Scratch | Required only to invoke and validate that native batch | Delete after the transition record is committed. The journal keeps the request identity, tactic digest, source identity, and tape digest. |
| `seed-*/native/decision-N/result*` | Same multiplicity as requests | Scratch | Required only while validating the worker response | Delete after the transition record is committed. Retained-candidate proof may project a sealed result later. |
| `seed-*/native/decision-N/result*.episodes.dseps` | Same multiplicity as requests | Learning or retained-candidate evidence | Supplies typed observations and exact consumed PAD | Admit useful shards once by content digest to the transition corpus. Delete ordinary failed/equivalent scratch shards; retain proof shards only for promoted candidates. |
| `seed-*/decision-trace/decision-N.json` | Every retained decision | Optional reporting | Resume currently rereads every trace to reconstruct timing, counts, episode number, and the final report | Remove from resume. Put the small semantic fields needed for exact continuation in the binary journal and project detail on demand. |
| `seed-*/decision-summary/decision-N.json` | Every retained decision | Optional reporting | Workbench “latest decision” projection | Remove from the hot path; derive from the journal. |
| `seed-*/edge-tapes/edge-N.tape` | Every retained decision | Retained-candidate evidence | Workbench edge replay | It is an accumulated route, not an edge fragment. Store each tactic tape once by digest and materialize an accumulated route only when replay is requested. |
| `seed-*/knowledge-graph/graph-N.json` | Branch interval and terminal | Optional reporting | Workbench graph projection | Remove from the hot path; derive from checkpoint, journal, and content references. |
| `seed-*/checkpoints/tactic-q-DIGEST.json` | Resume interval | Required for resume and learning | Rolling resume checkpoint; the preceding rolling file is deleted | Replace with the compact versioned binary checkpoint. |
| `seed-*/pause-checkpoints/decision-N/tactic-q-DIGEST.json` | Pause/cancel | Required for resume and learning | Immutable pause boundary used by CLI and workbench resume | Replace with the same binary checkpoint envelope and update discovery by envelope kind/version rather than `.json` naming. |
| `seed-*/performance/decision-N-attempt-M.json` | Pause and finalization | Optional reporting | Preserves cumulative timing across resumed invocations | Put cumulative counters in the compact checkpoint or a small journal control record; export JSON only in the report. |
| `seed-*/final-checkpoint/tactic-q-DIGEST.json` | Seed finalization | Required for learning; retained-candidate evidence when promoted | Freezing, inspection, and final identity | Store one compact binary checkpoint that references shared content. Do not embed replay routes and snapshots again. |
| `seed-*/graph.json` | Seed finalization | Optional reporting | Completed workbench graph | Project on demand. |
| `seed-*/successful.tape` | Successful seed only | Retained-candidate evidence | Replayable promoted route | Keep only for retained candidates. It is already a binary tape and must be content-addressed. |
| `seed-*/final-result.json` | Successful seed only | Retained-candidate evidence | Bundles terminal snapshot, complete replay, every replay route, and route tape | Replace the bundle with a small proof manifest referencing the terminal snapshot, route tape, transition range, and native proof artifacts by digest. Export readable JSON on demand. |
| `seed-*/seed-result.json` | Seed finalization | Optional reporting | Completed-seed resume and workbench summary; embeds the complete decision trace | Store a compact seed-complete control record. Project the readable result and trace from the journal. |
| `report.json` | Campaign finalization | Optional reporting | CLI/workbench completion report; embeds every seed result and trace again | Keep as an explicit on-demand export, not an operational resume object. |
| `lifecycle/cancelled.json` | Explicit cancellation | Required orchestration control | Prevents a cancelled workbench campaign from resuming | Replace with a compact journal lifecycle record or keep as a tiny authored control marker outside the per-decision path. It contains no learning state. |

## What the operational store actually needs

A retained transition needs only:

- deterministic decision, episode, seed, and selection metadata;
- source and next fact-snapshot digests;
- tactic-definition digest and exact realized tactic-tape digest/range;
- source and next logical checkpoint identities;
- reward/value sample, terminal bit, and episode group;
- references to any admitted native episode shard; and
- enough frontier and learner state to reproduce the next seeded choice.

Fact snapshots, tactic definitions, exact tactic tapes, and admitted episode
shards are immutable content objects. They must be written once by digest.
Accumulated routes are chains of tactic-tape references, not copied tapes.
Graphs, decision detail, summaries, and proof bundles are projections over
those records.

The current checkpoint violates that boundary by embedding `current`, the
complete accumulated `route_tape`, every replay transition (including full
before/after snapshots), every accumulated `replay_route`, and every episode
group in one JSON object. `final-result.json`, `seed-result.json`, `report.json`,
edge tapes, and graph outputs then copy substantial portions again.

## First implementation boundary

The smallest safe cut is the compact binary campaign checkpoint plus a decoder
that resumes the exact current in-memory state. This removes the largest
operational JSON object without changing search semantics. The transition
journal and content-addressed snapshot/tape split should follow immediately;
until then, converting the same duplicated payload to binary is only a format
fix, not the required storage design.
