# Cutscene phase programs and exceptional branches

Status: the planner has a strict, versioned cutscene-program schema and compiler.
For the post-Zelda tower sequence, planner-owned extractors now establish the
exact outer event/resource/normal-exit/skip-exit topology and a generic wrapper
join seals those records as `cutscene-wrapper-topology/v1`. A separate
`jstudio-stb-program/v1` decoder now extracts object sequences and paragraph
boundaries. A separate exact-content adaptor profile resolves their dispatch
semantics, while the actor-corruption failure boundary still requires
source/runtime evidence. Exact GZ2E01 executable evidence
proves that the room-loader's
nominal `dComIfGp_ret_wp_set` call is a retail no-op; this resolves that one
writer site without claiming that no other cutscene writer exists.

## Why this is a program

A cutscene is not represented as one milestone effect. A `CutsceneProgram`
contains sorted named phases. Each phase declares the resource archives expected
in the live flow component and one or more branches: advance, normal completion,
intentional skip, interruption, embedded scene change, or resource-load failure.

Every branch independently records:

- its exact phase and resource-request guards;
- physical obligations and unresolved requirements;
- confirmed operations in execution order;
- structured fields or raw bits affected by an unaudited suffix;
- the next phase or terminal cleanup; and
- exact-context evidence.

`route-planner compile-cutscene --program PROGRAM.json --output TRANSITIONS.json`
compiles those branches into ordinary `CandidateTransition` records. The compiler
adds the current-phase and requested-archive guards automatically. Nonterminal
branches advance the same flow component; terminal branches remove it. Search
therefore retains the cutscene phase, resource request/result fields, scheduled
cleanup, and all other backing state in its ordinary state identity.

## Failure semantics

Confirmed prefix operations remain normal ordered `StateOperation`s. An
uncertain structured suffix becomes `invalidate_field`; uncertain raw bits become
masked `invalidate_raw`. This differs from semantically clearing a known game
flag: subsequent readers see missing knowledge and return `unknown`.

Most importantly, a skipped writer is not replaced with a guessed value. If an
archive-failure branch does not contain a write to `PlayerReturnPlace`, the prior
return place survives. A later tower-to-Castle-Town savewarp must still be an
ordinary savewarp transition reading that retained backing component. The
cutscene failure branch does not grant a special warp.

## Validation and acceptance

The compiler rejects unknown phase targets, branch/phase ordering errors,
duplicate transition IDs, invalid resource fields, malformed uncertainty masks,
and branches that are both terminal and advancing. Its acceptance fixture proves
that normal scene-change and resource-failure branches compile to distinct
transition kinds, confirmed prefix writes remain ordered, the unaudited suffix
is invalidated, and no return-place write is invented.

The exact GZ2E01 wrapper is audited in
`gz2e01-zelda-cutscene-source-audit.md`: `demo07_02` selects
`Demo07_02/demo07_02.stb`, map-tool ID 4, normal SCLS exit 1 to Castle Town, and
skip SCLS exit 2 back to Zelda's tower. The room-loader return-place call is an
exact four-byte `blr`, so it preserves every incoming value generically.
The STB's 29 object streams, 387 commands, and 817 paragraph headers are bounded
and digest-sealed. A separate exact-GZ2E01 adaptor profile resolves all 695
object-specific paragraphs, including actor resource-ID requests, without
claiming they executed. Remaining Zelda-specific evidence work is to identify
the actual actor-corruption failure branch and last completed operation, and
trace any other return/restart writer and affected bit.
The wrapper topology is deliberately not compiled to executable transitions:
its two SCLS destinations are authored completion choices, while the behavior
of a failed resource load is still unresolved.
