# Cutscene phase programs and exceptional branches

Status: the planner has a strict, versioned cutscene-program schema and compiler.
Concrete post-Zelda tower phases and archive-failure behavior still require
source/runtime evidence; the generic model deliberately does not claim them.

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

Remaining Zelda-specific evidence work is unchanged: identify the real event,
cut, phase, archive/actor request, branch point, scene-change operation, all
return/restart writers, and the exact effects reached before and after failure.
