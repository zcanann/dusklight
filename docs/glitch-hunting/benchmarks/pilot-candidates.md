# Skybook pilot candidates

This is a decision aid, not the approved selection file. The five candidates
below were screened from the checked Skybook manifest at revision
`e9104852ff6b87862b67100f58aaa729096b42dc`. Approval remains a human scope
decision.

## Recommended ladder

| Order | Skybook page | Why it belongs in the pilot | Main missing work |
| --- | --- | --- | --- |
| 1 | `file-name-cursor-breakout` | The existing Eye Shredder tape, observer, semantic oracle, original-layout shadow model, renderer diagnostic, and cold-run evidence already exercise the complete proof path. Prior work makes this a cheap selected benchmark despite its memory-corruption mechanism. | Bind the existing benchmark definition and proof artifacts to the imported page identity. |
| 2 | `dash-cancel` | Short controller-only Wolf movement technique. A stage fixture can isolate dash start and a `B` cancel; the semantic oracle can require cooldown reset and a successful immediate re-dash. | Choose one stable ledge-free practice location and expose the relevant dash/cooldown state if current procedure/action facts are insufficient. |
| 3 | `epona-slide` | Exact one-frame `R` release to `A` dismount timing, two source videos, and an obvious movement-state outcome. It tests mounted stage fixtures and frame-perfect input without requiring a route chain. | Choose a flat practice location, author Epona/player placement, and define the dismount-slide state/velocity oracle. |
| 4 | `normal-bombs-underwater` | Three-step source method, one-frame timing, one video, and a semantic result that is stronger than a screenshot: a normal bomb remains usable underwater. It exercises inventory/loadout stage boot. | Choose a water-entry fixture with an eligible height and expose normal-bomb ownership/held/deployed plus swim-state facts. |
| 5 | `city-in-the-sky-gate-skip-via-pot-displacement` | The map-specific collision representative: a documented setup, visual cue, two videos, and an unambiguous success condition of passing the gate and entering the next room. | Author the exact City in the Sky room/layer/loadout/actor fixture, identify the pot and rupee, and expose wall-contact/room-transition evidence. |

The intended execution order is deliberate. Eye Shredder proves that selection
metadata can bind an existing complete benchmark. Dash Cancel is the smallest
new stage-boot case. Epona Slide and Normal Bombs Underwater then add mounted
and inventory/water fixture requirements. The City in the Sky gate is last
because actor placement and collision make it the most expensive, but it is the
first case that validates the requested boot-into-a-map practice workflow
against a concrete map-local goal.

## Exact imported identities

| Slug | Source path | Source SHA-256 |
| --- | --- | --- |
| `file-name-cursor-breakout` | `_posts/file-name-cursor-breakout.md` | `f2e4ce02f15d01b2028e328cbf9f12bfd0e96ddfa3ae033bf736ee7d4d3d8fd3` |
| `dash-cancel` | `_posts/dash-cancel.md` | `0bd17d353d0fd0112f8ac95384b3d0f031eb9743861f750fe8ad551dd103f593` |
| `epona-slide` | `_posts/epona-slide.md` | `69d64717831a51699e3980b4ab695b45d3a985cd75e1de7a94ee9d12cf19a5bb` |
| `normal-bombs-underwater` | `_posts/normal-bombs-underwater.md` | `900c2009861beeabc59a2d345881a88588e9900ff51f94ea6aed77dfffbce899` |
| `city-in-the-sky-gate-skip-via-pot-displacement` | `_posts/city-in-the-sky-gate-skip-via-pot-displacement.md` | `189db8166dcd425c640beb8ca0af494cbe84cdbfe160d4450309629c802e0093` |

Each identity must be revalidated against `benchmarks/skybook/manifest.json`
when materializing the approved selection. The selection should fail closed if
the repository revision, source path, or source digest changes.

## Alternatives considered

- `quick-climb` and `clawshot-l-slide` have short descriptions but no exact
  practice location or video in the imported page, making their first fixture
  and oracle more interpretive than Dash Cancel.
- `long-jump-attack-lja` is high-value and well sourced, but target placement,
  platform-specific boomerang behavior, and multiple method families make it a
  better second-wave benchmark after the simple movement cases.
- `ordon-springs-sword-shield-skip` is map-specific and richly documented, but
  its Hugo manipulation, damage/reload constraint, trigger crossing, and
  chained LJAs are substantially more expensive than the proposed gate case.
- `item-pickup-slide` has an uncertain direction/speed mechanism and documented
  multi-hour clips, which is unsuitable for the first bounded pilot.
