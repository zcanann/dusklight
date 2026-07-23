# GZ2E01 scene-change consumer audit

This audit answers a narrower and more useful question than “which SCLS records
exist?”: which exact source consumers turn an indexed SCLS destination—or a
direct destination—into a scene-change request. It applies only to the exact
`gcn-us-1.0-gz2e01` content identity.

The reproducible command is:

```text
route-planner audit-scene-change-consumers \
  --source-root SOURCE_PROJECT/src \
  --content-identity GZ2E01-CONTENT.json \
  --output build/gz2e01-scene-change-consumer-audit.json
```

The extractor walks only ordinary `.cpp` files, rejects symlinks, strips line
and block comments while preserving line numbers, hashes every source file that
contains a recognized occurrence, and seals the sorted file/line/symbol census.
It records no host path or source text. The reproduced artifact has whole-file
SHA-256
`d7302f24f08f8df17cea7bf6507469dcca86d4565f662c2bd997dc9f323e0ee9`
and semantic content seal
`d6525e0caa83554e6949a1e02a3a9c04815db9c3cba759848168f3620616e342`.
The exact canonical artifact is checked in at
`tools/route-planner/crates/engine/data/gz2e01-scene-change-consumer-audit.json`;
an engine test decodes it, recomputes every aggregate, and checks byte identity.

## Exact census

| Consumer family | Occurrences | Source files | Meaning |
| --- | ---: | ---: | --- |
| `dStage_changeScene` | 80 | 49 | An actor/player/core path supplies an exit index; the runtime resolves that index through the source room's loaded SCLS table. |
| `dStage_changeSceneExitId` | 2 | 2 | Link's collision path derives an exit index from the contacted ground polygon before entering the common indexed resolver. |
| `dStage_changeScene4Event` | 2 | 2 | Event completion resolves a selected event exit through SCLS with event-specific wipe/room behavior. |
| `onSceneChangeArea` | 11 | 9 | A door, warp, or scene-area actor latches an exit on the player; the player state machine performs the later indexed transition. |
| `onSceneChangeAreaJump` | 3 | 2 | A scene-area/fall actor latches the jump variant rather than immediately changing scenes. |
| `dComIfGp_setNextStage` | 40 | 14 | The consumer supplies a concrete destination directly and does not consume an SCLS record. |

There are 138 recognized occurrences across 68 exact source files. Occurrences
include the core resolver definitions as well as callers, so the artifact covers
both the dispatch boundary and its consumers.

## Planner consequences

An extracted SCLS row is only an indexed destination. The activation contract
comes from one of several distinct producer families:

- Collision exits need the contacted polygon's exit code, Link's current room,
  and the player collision/state-machine conditions.
- `SCENE_EXIT`-style areas and doors need the actor's decoded exit parameter,
  volume or interaction feasibility, any guard/event/collision phases, the
  player latch, and the later player transition.
- Direct actor calls need that actor's state-machine branch, parameter-derived
  index, and hard guards. An actor placement alone does not establish execution.
- Event exits need the selected REVT/LBNK/event branch and its completion or skip
  semantics.
- Direct `setNextStage` calls are separate transition providers. Joining them to
  a nearby SCLS row would invent a dependency and can select the wrong target.

The existing importer already promotes the source-audited boss/keyed-door
families, collision/SCLS joins as explicitly inferred candidates, and selected
cutscene exits. Every other call site now has an exact source file, digest, line,
symbol, and consumer family for follow-up, but remains an encoded or unknown
activation until its actor/event/player guards are audited. This is deliberate:
the census closes the consumer inventory without claiming that all 68 source
files have executable planner semantics.

## High-value families exposed by the census

- Door/player-latch consumers: ordinary boss, L1/L5 boss, mini-boss, knob,
  double-door, small-gate, and boss-warp families.
- Generic area consumers: `d_a_scene_exit`, `d_a_scene_exit2`, Kargaroc-fall,
  river-back, event-tag, twilight-gate, and level-8-gate actors.
- Stateful actor-index consumers: NPCs, bosses, dungeon elevators, cannons,
  portals/fairies, and scripted enemies.
- Non-SCLS direct destinations: title/menu/game-info flows, twilight gates,
  scripted enemies, Kargaroc paths, and event data.

These buckets are routing for future import work, not semantic equivalence
classes. Each concrete actor still needs its own parameter and activation audit.
