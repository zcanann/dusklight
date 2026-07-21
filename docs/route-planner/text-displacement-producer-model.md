# Text Displacement producer model

Status: acceptance model complete for four distinct producer shapes. The raw
temporary-bit backing and generic BMG operations are source-audited for GZ2E01;
the producer techniques below currently retain their community/version evidence
instead of being silently generalized to every executable.

## Shared result, independent causes

All four routes can produce flow-control B (`tempBitLabels[12]`, packed
coordinate `0x0002`), but the planner does not represent that result as a
`has_text_displacement` capability. Each route mutates the same raw temporary
component through a different transition program:

| Producer | Causal program | Independent feasibility/state |
|---|---|---|
| Coro | reach the post-bottle/pre-next-line node; set B; interrupt and cancel the pending general-flow cleanup | bottle dialogue reached, Ooccoo available, producer-specific one-frame pull witness |
| Auru | first edge interaction sets A; second edge interaction advances to B | two ordered talks, Auru loaded, player controlled and talking inside his talk volume while outside the cutscene trigger |
| Yeta | first Snowpeak talk sets B; map/talk interruption prevents cleanup | first-talk one-shot still available and a producer-specific same-frame map witness |
| Zombie Ooccoo | lethal first-introduction pull sets A; the one-time intro becomes consumed; a second pull advances A to B | first-warp introduction unused, death-frame pull witness, then the retained A bit and first-step marker |

The first and second Ooccoo actions are separate nodes. The first action cannot
be repeated after its one-time state changes, and the second cannot execute
without the first action's raw A bit and route marker. Likewise Auru does not
jump directly to B: the second transition reads both his talk count and raw A.

## Physical and temporal proof identity

Coro, Yeta, and the first Ooccoo action each reference a distinct
`TemporalRequirement` and distinct `WitnessedMicrotrace`. A microtrace can only
discharge a requirement when its action ID, frame interval, input, scope, and
precondition match. Removing Coro's witness leaves the Coro transition in the
unknown timing frontier; neither Yeta's nor Ooccoo's one-frame witness can stand
in for it.

Auru is not modeled as a timing alias. His route uses an interaction obligation
with a required talk volume and an excluded cutscene-trigger volume. The
acceptance case succeeds at a point in the annulus and fails at a point inside
both volumes. An exact retail fact pack must supply the evidenced shapes and a
reachable pose; the acceptance geometry only proves that the generic schema and
solver preserve the distinction.

## Evidence boundary

The community route descriptions are recorded at
<https://www.zeldaspeedruns.com/tphd/tech/text-displacement>. They describe:

- Coro's one-frame Ooccoo pull after the bottle text,
- two Auru talks just outside his cutscene trigger,
- Yeta's first-talk/map same-frame interruption, and
- the first-use Ooccoo/death setup followed by a second advancement.

That page is TPHD-scoped. It is evidence for the route shapes, not proof that
the same coordinates, timing windows, and interaction geometry apply to
GZ2E01. Conversely, `src/d/d_msg_flow.cpp`, `src/d/d_event.cpp`,
`src/d/d_save.cpp`, and the extracted GZ2E01 BMG prove the generic raw-bit
mechanism but do not by themselves prove each physical interrupt. A production
pack must compose both kinds of evidence under the exact supported context.

## Acceptance assertions

`solver::tests::text_displacement_producers_are_distinct_proofs_over_the_same_raw_bits`
verifies:

- every producer reaches the same raw B-bit goal;
- Coro and Yeta retain their specific microtrace IDs in the solve proof;
- Auru takes two ordered message actions;
- Zombie Ooccoo takes the one-time interrupted action and then the advancement;
- placing Link inside Auru's excluded trigger blocks his route; and
- deleting Coro's witness blocks Coro without changing the consumer bit or any
  other producer definition.

This is deliberately the producer half of the graph. Gor Coron's raw query011
consumer, persistent access-control effects, wall/elevator actors, live NPC
blockers, and reload reconstruction remain separately modeled 11J tasks.
