# Engine-session reuse boundary

The first persistent-session target is deliberately narrower than a general
checkpoint worker: keep one native process, Aurora, the opened game image, and
process services alive while sequential stage-boot requests continue to use
the authenticated harness request/result path.

Query the current executable before attempting reuse:

```sh
cargo run --manifest-path tools/huntctl/Cargo.toml -- \
  session audit \
  --worker build/macos-default-debug/Dusklight.app/Contents/MacOS/Dusklight \
  --worker-arg --automation-worker
```

`dusklight-engine-session-reuse-audit/v1` is a typed refusal, not a claim that
the engine has been initialized. Its evaluated boundary is `pre_engine_boot`;
its target is the unimplemented `post_authenticated_run` reuse boundary. The
blocker list is unique and code-sorted so orchestration can name the first
unproved subsystem instead of treating `engine_session=false` as an opaque
feature flag.

The current blockers and required evidence are:

| Code | Required guarantee |
| --- | --- |
| `automation_state_reset` | All paths, flags, players, recorders, observers, and timers return to declared per-run defaults. |
| `dolphin_thread_join` | DVD, memory-card, audio, and OS worker threads join at quiescence and can be recreated. |
| `game_global_reconstruction` | Game context, process lists, reset data, and static managers reconstruct from a clean origin. |
| `heap_recreation` | JFW/JKR heaps and ARAM contain no live references at destruction and can be recreated from a valid arena. |
| `mod_lifecycle` | Native hooks and registrations survive the retained process or repeat initialization without duplication. |
| `process_run_lifecycle_partition` | Aurora, DVD hosting, logging, and process services are separated from game-run teardown. |

Reuse remains refused until every blocker is discharged in code and an A/B/A
sequence produces cold-identical terminal, tick, boundary, tape, trace, and
objective evidence. Removing a blocker because a second call happens not to
crash is insufficient.

## Completed-run lifecycle seam

The completed native path now has a runtime-owned, ordered lifecycle instead
of letting `main01` destroy its own dependencies:

1. `pre_game_run` admits exactly one transition to `game_run_active`;
2. the loop flushes authenticated proof, then `finish_game_run` closes mods,
   UI, and movie state at `post_authenticated_run`;
3. diagnostics stop before machine heaps are destroyed;
4. machine destruction broadcasts the irreversible emulated-OS shutdown; and
5. Discord, texture/config state, and Aurora close last as host services.

Any second admission after `post_authenticated_run` is explicitly
`RefusedResetUnproved`. This is the first precise reuse refusal boundary: proof
for the completed request exists and Aurora/DVD hosting are still alive, but
no world reconstruction guarantee exists. The existing shutdown order is
preserved because moving UI, mods, logging, or threads across heap destruction
without ownership evidence would turn the reset experiment into undefined
lifetime behavior.

Authenticated executions bind the same blocker inventory at that exact
boundary inside `dusklight-native-lifecycle-timing/v2`. The decoded audit is
therefore covered by the ordinary run-result identity and native timing
artifact digest. Timing v1 remains readable for historical cold-process
reports, but it contains no session audit and cannot prove a post-run reuse
decision.

The completed-run path actually performs that second admission check before
diagnostics or host services stop. A wrong admission changes the ordered
lifecycle, prevents timing v2 from authenticating `post_authenticated_run`, and
then fails the teardown boundary checks. The checked result is therefore an
executed refusal inside the first process, not a capability guess made by
huntctl.
