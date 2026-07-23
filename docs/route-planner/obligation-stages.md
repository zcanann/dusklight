# Feasibility obligation stages

Mechanics catalog v29 assigns every `FeasibilityObligation` exactly one stage:

| Stage | Question answered | Typical detail |
| --- | --- | --- |
| `reach` | Can the propagated player state reach this exact approach? | collision regions, approach geometry, actor-front planes |
| `activate` | Can the interaction or trigger accept the action? | talk/attention volumes, facing, control/form predicates |
| `effect` | Does the authorized interaction reach and commit its modeled state operation? | actor event phases, item-grant execution, queued-write completion |
| `interrupt` | Can the required interruption occur at the named boundary? | temporal windows, inline interrupt operations, witnessed microtraces |

The stage is independent of `obligation_kind`. Kind describes the subject—such
as geometry, interaction, timing, or actor state—while stage describes where it
blocks the candidate pipeline. An unresolved actor-state question can therefore
be an activation obligation or an effect-commit obligation without relying on
its label for meaning.

Catalog validation enforces two cross-record invariants. A geometry detail must
name the exact `approach_id` of every transition that binds it, and an effect
obligation cannot be attached to a transition with no state operations.
Interruption obligations may remain unresolved: an inline `Interrupt` operation
or a matching `WitnessedMicrotrace` can discharge a temporal requirement, but
absence of either remains a real feasibility unknown rather than invalidating
the catalog.

`obligation-coverage/v1` is a canonical, mechanics-digest-bound inventory with
one row for every candidate transition. Each row separates the four obligation
ID lists, explicit unknown activation requirements, state-operation count,
inline interruption action IDs, and matching microtrace IDs. This includes
message actions, item grants, NPC/actor producers, cleanup actions, and other
state-producing transitions; it is not limited to map exits and doors.
