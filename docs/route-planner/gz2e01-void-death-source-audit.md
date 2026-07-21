# GZ2E01 void, death, and title-return source audit

This audit records the exact-executable and source-visible branch structure that
must precede an executable GZ2E01 void/death model. It deliberately does not turn every branch
into a generic “reload” transition. Several branches consume different backing
stores, and some destinations are encoded indirectly through collision exits.

## Exact executable evidence

The planner-owned `binary-function-evidence/v1` extractor resolved these exact
symbols from the registered GZ2E01 `main.dol` and symbol table. The executable
SHA-256 is `e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8`;
the symbol-table SHA-256 is
`8b8c98b86b6270543709adbbd489ca4a5cd4fa5c30fd4a410420702fd37a085a`.

| Function | VA | Size | Code SHA-256 | Artifact SHA-256 |
| --- | ---: | ---: | --- | --- |
| `memory_to_card__10dSv_info_cFPci` | `0x80035798` | `0x26c` | `7cf6fc958ed1e4cdcf4b3e168364cbd7a42a545a1812d139a4442e41ae5fd8e9` | `5b65a8833c8fb246e5c0292e0f22ecf6b05f5e3a123f2f18ee33c343a9805f1e` |
| `startRestartRoom__9daAlink_cFUliii` | `0x800bdf60` | `0x30c` | `738eea0aabe272b6a9ac7ecd84e62c137769813721770955f82cf9695f3dae9e` | `7a05e8bb294e50e0a6d8cf87152bf7ac88aca71e57bd3738d8007a171ae4e56f` |
| `checkRestartRoom__9daAlink_cFv` | `0x800be3e4` | `0x5f0` | `dbb2fe71121cbdbfe2514010bb066ac0b3c063434e99b4a769459ccbfacbdbfb` | `0a557c9bed46530e109182f8d1e648182a6b10c1ded0eec56649b67451d0e92f` |
| `checkRestartDead__9daAlink_cFii` | `0x80118b34` | `0xc0` | `940c69251522802fc97558c38a661867ce5c47077358e8450d5de5aa43983dc8` | `5823f9b8cd273dbf128cfece69b1d89690fd9c3f383789108d4ee10ce063fb0f` |
| `procCoDead__9daAlink_cFv` | `0x8011c1b4` | `0x478` | `22f6a70a37b32139d98899711fb1f52daa6f5c71db301bcb9fa041986265d9e6` | `84641294b6728c4acad403f2dc37b897df75b8c1e8976808a91f551e2bcd7a4e` |
| `saveClose_proc__11dGameover_cFv` | `0x8019b5f4` | `0x1c8` | `1c670da55f7f8c8f49c6e16682407b615ab8064a099aec72c6fa3a9fc95ce7b4` | `f94057684bf4cf10a1cdbf8ea1c9c451683d430faf5faaa4932b531ee913b3d9` |
| `restartInit__12dMenu_save_cFv` | `0x801f30b8` | `0xf8` | `22acf942d9526e73e3e01d320f3da1658e7c4de5e16cfb3e8640d6fda7e4018e` | `fe4ae5527102a981d9981d27e156c91771dc48aee7e7a4a33934c183cf4058e0` |

## Audited sources

| Source | SHA-256 | Relevant code |
| --- | --- | --- |
| `src/d/actor/d_a_alink.cpp` | `e03a99558b9badea3f3976cc7d8c7a11b716a7402de6ad8b8b7832750ae8525c` | `startRestartRoom`, `checkRestartRoom` |
| `src/d/actor/d_a_alink_demo.inc` | `45112b2d6fcc98613fbf896282c1e496fa488ffeef1a6f440d2ada22ca204dc6` | lethal checks, dead process, continue dispatch |
| `src/d/actor/d_a_alink_damage.inc` | `cbc11915027ab7da4a838fa35de95c640321b7d3187cd17bcab0bdd68b2d50e0` | lava/quicksand restart dispatch |
| `src/d/d_gameover.cpp` | `f4b46cdb449d214dafec4dd727e400bf7cabf0834712b3350b9c1cb3cc1a5f0f` | continue/title choice and pre-continue cleanup |
| `src/d/d_menu_save.cpp` | `78acd5de6255c5031eeeb0d041509b9080b7121e68a1546d14ba75a6454f0f4e` | retry, continue, reset, and `restartInit` |
| `src/d/d_com_inf_game.cpp` | `b9b37aed0b76eef2d27b35a2ece6ee077086a970f98d18936a83649303f15761` | reset-to-opening guard and play-state initialization |
| `src/d/d_save.cpp` | `7e6f09aa36af30932e8ce64423284f885ed0b4e632b22f18d6f0a6b4d104b453` | transient save projection in `memory_to_card` |

These hashes seal the audited source family. The table above separately seals
the exact retail functions; witnessed runtime traces remain independent evidence.

## Void and hazard restart selection

`daAlink_c::checkRestartRoom` has at least three materially different outcomes:

1. A ground/hazard polygon can provide an exit ID. The game requests
   `dStage_changeScene(exitID, ..., roomNo, ...)`. This consumes collision and
   stage-exit data, not the held restart-room destination.
2. When no usable polygon exit exists (`0x3f`), `startRestartRoom` requests
   `dStage_restartRoom` using the held restart room number plus a derived start
   mode and packed restart parameter.
3. Lava, quicksand, frozen-swim, coach, board, boar, spinner, magnetic-boots,
   and other special cases alter the mode, damage, room, or branch before either
   request is made.

Both request forms are gated by live player/process state. The common restart
path requires the one-shot restart flag to be clear and either an already
running event or successful compulsory-event acquisition. It first checks
whether the damage would cause death; a lethal result enters game over instead
of requesting the restart. Only the nonlethal branch sets the one-shot flag and
requests the stage change.

Therefore a planner needs separate candidates for at least:

- collision-exit hazard change;
- held restart-room reload;
- special hazard variants that modify restart mode/damage; and
- lethal diversion to the death flow.

An observed void plane alone proves none of those destinations. It is a physical
precondition for entering the selection logic.

## Death and continue selection

`checkDeadHP` and `checkRestartDead` admit death from zero life, forced-gameover
state, oxygen exhaustion, or lethal restart damage, subject to fairy and magic
armor behavior. `procCoDead` creates the game-over UI and waits for a completed
choice.

For ordinary game over, choosing continue produces game-over status 2. The dead
process then restores life to 12 and selects one of four destination families:

1. special `D_MN09A` room-50 exits selected by layer;
2. boss-room exit 0 under the source guards;
3. an actor-captured exit ID and optional room override; or
4. `startRestartRoom`, consuming the held restart-room backing.

The restart mode is normally 5, but is 0 in `F_SP102` and `D_MN08D` room 55.
These are distinct predicates, not friendly labels for one universal death
reload.

Choosing not to continue eventually calls `mDoRst::onReset`. The later play-scene
loop may execute `dComIfG_resetToOpening` only when its platform-specific reset,
menu, fader, and card-communication guards pass. The already modeled
reset-to-opening transition begins there; death does not directly load the title
map or file 0.

## Mutations before restart

The continue path is not destination-only. Source-visible mutations include:

- life restoration to 12 before the death scene request;
- clearing monkey-lantern stolen/dropped event bits when the recovery bit is
  absent;
- restoring the lantern and backed-up oil when its acquisition bit is set but
  its inventory slot is blank;
- minigame-item restoration; and
- the Stallord-arena game-over-type-1 Ooccoo removal and last-warp reset.

The same lantern adjustments appear in `dMenu_save_c::restartInit`. They mutate
live state on the continue path. They must not be conflated with
`dSv_info_c::memory_to_card`, which temporarily applies related changes to form
the serialized projection and then restores the live values.

## Required executable representation

The model should add independent, state-driven programs for:

- polygon-exit void/hazard requests;
- held restart-room requests using the current stage plus restart-room backing;
- lethal diversion into game-over state;
- each death-continue destination family;
- continue-time life, lantern, oil, minigame, Ooccoo, and warp mutations; and
- the reset request that precedes the existing guarded title transition.

Each scene request must remain pending until scheduler/world-load progress is
observed. A restart-room record is not itself a map transition, a collision exit
does not rewrite that record, and a title reset does not imply a successful save
projection.

## Remaining evidence gaps

- Seal any remaining callee ranges needed to decode collision-exit and packed
  restart-parameter semantics; the seven top-level branch functions above are
  already exact-GZ2E01 evidence.
- Capture voids using a polygon exit and the `0x3f` restart fallback.
- Capture ordinary death continue, boss-room death, and return-to-title.
- Identify the exact actor fields that hold the captured death exit/room pair.
- Decode the packed restart parameter and start-mode effects sufficiently to
  reconstruct spawn, form, equipment, and damage behavior.
- Model `memory_to_card` as a transformed persistent projection that leaves the
  post-call live runtime unchanged.
