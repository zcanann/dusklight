# Authored milestones

Milestones turn a read-only game-state predicate into a named, reproducible
boundary. The Rust compiler accepts a small source language and emits a compact
`DMSP` program for native evaluation. The language has no input actions,
assignments, calls, loops, or memory-write operations.

## Source format

A file starts with an explicit language version. New source should use
`milestones 1.4`; the compiler and native decoder continue to accept canonical
1.0 through 1.3 programs without upgrading their bytes. Version 1.0 contains the
original scalar facts, 1.1 adds gameplay/RNG/collision facts and bit masks, and
1.2 adds parameterized placed-actor and indexed-flag queries. Version 1.3 adds
inclusive range syntax, spatial relations, and bounded ordered sequences.
Version 1.4 adds named exact value-parity projections.

```text
milestones 1.4

milestone "gameplay-ready-f-sp103" {
  phase post_sim
  stable 2
  when stage.name == "F_SP103" &&
       stage.room == 1 &&
       stage.spawn == 1 &&
       player.exists &&
       player.is_link &&
       !event.running &&
       event.id == -1
}

milestone "exit-f-sp103-to-f-sp104" {
  phase post_sim
  when stage.name == "F_SP103" &&
       next_stage.enabled &&
       next_stage.name == "F_SP104" &&
       next_stage.room == 1 &&
       next_stage.spawn == 0
}
```

Names may be bare words or quoted strings. `#` and `//` start line comments.
Properties inside a milestone may appear in any order; `phase` and `when` are
required, while `stable` defaults to `1`. Duplicate names or properties and
unknown properties are errors.

Expressions support parentheses, unary `!`, `&&`, `||`, and the comparison
operators `==`, `!=`, `<`, `<=`, `>`, and `>=`. Unsigned 32- and 64-bit fields
also support `has_all` and `has_any` with a nonzero mask. Their precedence from highest
to lowest is parentheses, `!`, `&&`, then `||`. A bare Boolean field is shorthand
for `field == true`. Ordered comparisons are available only for numeric fields;
Boolean and symbolic fields accept only `==` and `!=`.

There are no implicit coercions. Integers must fit the field's exact signed or
unsigned width, floats must be finite `f32` values, and symbols must be quoted.
The compiler rejects NaN, infinity, negative zero in a programmatic AST,
unknown fields, excessive nesting, and excessive operations. A program is
bounded to 256 definitions and 1 MiB; each expression is bounded to 256
operations and depth 32.

## Named value-parity projections

A 1.4 milestone may capture up to eight named value axes from the same immutable
observation that produces its first hit:

```text
milestones 1.4

milestone handoff {
  phase post_sim
  when stage.name == "F_SP103" && stage.room == 1
  projection "handoff-state" {
    rng primary
    rng secondary
    actor_population "F_SP103" 1
    flag event 821
    flag switch 1 239
  }
}
```

Each projection contains one to 32 exact items. RNG items retain algorithm
version, all three state words, and call count. Actor populations are canonical
sorted multisets for one exact stage and home room; each member retains actor
type, map set ID, home/current room, position bit patterns, health, and status.
Runtime generation/process IDs are excluded because they are not portable
identities. Flag items retain one exact indexed Boolean and switch flags also
bind their room.

The projection name and item specification have a standalone SHA-256 identity.
At a hit, schema-v5 native results include inspectable values and an XXH3-128
fingerprint over the canonical exact value encoding. A truncated actor catalog,
wrong live stage, missing flag snapshot, off-room switch snapshot, or invalid RNG
snapshot makes the projection unavailable; it never silently compares equal.

Parity comparison has exactly three outcomes. Matching projection identities
and value fingerprints are `equal`; matching identities with different value
fingerprints are `different`; different identities or unavailable captures are
`incomparable`. Route ancestry, parentage, and boundary topology are not inputs.

## Evaluation phases and boundary numbers

`phase pre_input` evaluates immediately before a frame's input is consumed.
`phase post_sim` evaluates after that input has driven one simulation tick.
Boundary numbers identify the gap between ticks:

- Pre-input boundary `0` is before the first input. Its kind is `"boot"`, and
  `tape.frame` is unavailable.
- At pre-input boundary `B > 0`, frame `B - 1` has completed and is exposed as
  `tape.frame`; frame `B` has not yet been consumed.
- Post-sim evaluation for tick/frame `N` occurs at boundary `N + 1` and exposes
  `tape.frame == N`.
- Every boundary after boot has kind `"tick"`.

`boundary.reached` is true whenever the evaluator is invoked. It is provided as
a typed Boolean primitive for generated expressions. If `tape.frame` is
unavailable, every direct comparison involving it evaluates false, including
`tape.frame != value`. Ordinary `!`, `&&`, and `||` behavior applies to that
Boolean result afterward.

This phase distinction prevents a proof from quietly moving across input
application or simulation. A pre-input and post-sim milestone with otherwise
identical text are different definitions with different identities.

## Fields and types

| Field | Type | Accepted comparison values |
| --- | --- | --- |
| `boundary.kind` | enum | `"boot"`, `"tick"`, `0`, or `1` |
| `boundary.index` | `u64` | unsigned integer |
| `boundary.reached` | Boolean | `true` or `false` |
| `tape.frame` | optional `u64` | unsigned integer; unavailable as described above |
| `stage.name` | stage symbol | quoted stage ID |
| `stage.room` | `i32` | signed integer |
| `stage.layer` | `i32` | signed integer |
| `stage.spawn` | `i32` | signed integer |
| `player.exists` | Boolean | `true` or `false` |
| `player.is_link` | Boolean | `true` or `false` |
| `player.position.x` | finite `f32` | numeric literal |
| `player.position.y` | finite `f32` | numeric literal |
| `player.position.z` | finite `f32` | numeric literal |
| `player.speed` | finite `f32` | numeric literal |
| `player.procedure` | procedure enum | `u32` or quoted procedure token |
| `event.running` | Boolean | `true` or `false` |
| `event.id` | `i32` | signed integer, including `-1` |
| `next_stage.enabled` | Boolean | `true` or `false` |
| `next_stage.name` | stage symbol | quoted stage ID |
| `next_stage.room` | `i32` | signed integer |
| `next_stage.layer` | `i32` | signed integer |
| `next_stage.spawn` | `i32` | signed integer |

Language 1.1 adds these scalar facts:

| Fact family | Fields |
| --- | --- |
| Player identity and motion | `player.process_id` (`u32`), `player.actor_name` (`i32`), `player.velocity.{x,y,z}` (`f32`), `player.current_angle.{x,y,z}` (`i32`), `player.shape_angle.{x,y,z}` (`i32`) |
| Player state | `player.mode_flags` (`u32`), `player.timer.damage_wait` (`i32`), `player.timer.ice_damage_wait` (`i32`), `player.timer.sword_change_wait` (`u32`) |
| Event state | `event.mode`, `event.status`, `event.map_tool_id` (`u32`), `event.name_hash.present` (Boolean), `event.name_hash.fnv1a32` (optional `u32`) |
| RNG | `rng.{primary,secondary}.state{0,1,2}` (`i32`) and `rng.{primary,secondary}.calls` (`u64`) |
| Player geometry | `collision.{ground,wall,roof,water}.contact`, `collision.water.in` (Boolean), `collision.{ground,roof}.height`, and `collision.ground.clearance` (`f32`) |

Event-name hash has explicit availability. It remains unavailable in live
observation because the existing game accessor is non-const; a comparison with
the hash therefore fails unless a fixture or future audited adapter supplies
it. `event.name_hash.present` lets predicates require that evidence instead of
confusing absence with hash zero.

## Stable actor and indexed flag queries

Language 1.2 parameterizes facts without admitting arbitrary memory reads. A
placed actor is selected by exact stage, home room, map set ID, and actor type:

```text
milestones 1.2

milestone local_actor_goal {
  phase post_sim
  stable 3
  when actor.placed.exists("F_SP103", 0, 7, 42) &&
       actor.placed.distance_to_player("F_SP103", 0, 7, 42) <= 125.0 &&
       actor.placed.health("F_SP103", 0, 7, 42) > 0 &&
       flag.event(821) &&
       flag.switch(0, 239)
}
```

The available actor facts are `exists` (Boolean), `position.{x,y,z}` and
`distance_to_player` (`f32`), `current_room` and `health` (`i32`), and `status`
(`u32`). Set ID `65535` is reserved, stage names use the same canonical rules
as `stage.name`, and home rooms are bounded to `-1..63`.

Actor capture is bounded to 256 immutable entries. Exact selection never falls
back to nearest: a duplicate placed identity or a truncated population makes
the query unavailable. Absence is true absence only when the complete snapshot
contains no match. `distance_to_player` is finite three-dimensional Euclidean
distance over the same snapshot.

Indexed Boolean flags use `flag.event(0..821)`,
`flag.temporary(0..184)`, `flag.dungeon(0..63)`, and
`flag.switch(ROOM, 0..239)`. Switch room is explicit and bounded to `0..63`.
The observer copies global flags plus the current room's switches at the same
phase as the rest of the milestone observation. An off-room switch query is
unavailable rather than consulting mutable live state during evaluation.

## Ranges, regions, planes, and contact relationships

Language 1.3 accepts `FIELD between MIN and MAX` for numeric scalar or query
facts. The bounds are inclusive, must have the field's exact type, and must be
ordered. It is canonical syntax sugar: formatting or decoding emits the exact
equivalent `FIELD >= MIN && FIELD <= MAX`, so there is only one bytecode
identity for the range.

`player.in_aabb(MIN_X, MIN_Y, MIN_Z, MAX_X, MAX_Y, MAX_Z)` is an inclusive
finite world-space region test. `player.plane_signed_distance(P_X, P_Y, P_Z,
N_X, N_Y, N_Z)` returns normalized finite signed distance to a world-space
plane and rejects a zero or non-finite normal. It composes with ordinary
comparisons to express side, crossing tolerance, or exact plane contact.

The existing `collision.ground.contact`, `collision.wall.contact`,
`collision.roof.contact`, `collision.water.contact`, and `collision.water.in`
facts are the exact player/background contact relationships captured by the
native collision adapter. Region and plane facts do not call collision helpers
or mutate caches; they derive from the immutable player position snapshot.

## State transitions and ordered sequences

A definition can replace one instantaneous predicate with two to sixteen
ordered steps. `within N` is mandatory when any `then` step exists:

```text
milestones 1.3

milestone entered_trigger_after_ground_contact {
  phase post_sim
  within 4
  when collision.ground.contact
  then player.in_aabb(-10.0, 0.0, -10.0, 10.0, 20.0, 10.0)
  then event.id == 17
}
```

Only one step can advance per matching-phase evaluation, so steps are always
strictly ordered in logical time. `within N` counts evaluations after the
first step and includes the evaluation at distance `N`. A two-step sequence
with `within 1` is an exact next-tick state transition. If the window expires,
the state machine resets before evaluating that tick as a possible new first
step. If a later step does not match but the first step does, the sequence
restarts deterministically at that tick; a matching expected step takes
precedence. Sequence progress is automation-owned state, resets with the
tracker, and never enters game memory.

`stable N` remains the persistence primitive for one predicate. It cannot be
combined with `then`; use predicates on consecutive sequence steps when exact
multi-state timing matters. Sequence length is bounded to 16, `within` and
`stable` are bounded to `1..65535`, and the existing bytecode operation/depth
limits apply across all steps.

Stage symbols are one to eight ASCII uppercase letters, digits, or underscores,
such as `"F_SP103"`. Procedure symbols are exact native enum tokens such as
`"PROC_WAIT"`. For crawl procedures, the author-facing aliases
`"crawl_start"`, `"crawl_move"`, `"crawl_auto_move"`, and `"crawl_end"`
compile to `PROC_CRAWL_START`, `PROC_CRAWL_MOVE`, `PROC_CRAWL_AUTO_MOVE`, and
`PROC_CRAWL_END`. The ambiguous symbol `"crawl"` is rejected. Other unknown
procedure symbols are rejected when the native loader resolves the compiled
program.

## Stability

`stable N` requires the predicate to be true for `N` consecutive evaluations
in the milestone's selected phase. A false evaluation resets that milestone's
streak, and the hit occurs on the `N`th consecutive true result. The allowed
range is `1` through `65535`; omitting the property is exactly `stable 1`.

Stability is part of the definition identity. Changing it is not merely a
display or search-policy change: it changes what the milestone proves.

## Compile, inspect, and format

From the repository root:

```powershell
cargo run --manifest-path tools/huntctl/Cargo.toml -- milestone format .\route.milestones
cargo run --manifest-path tools/huntctl/Cargo.toml -- milestone compile .\route.milestones .\route.dmsp
cargo run --manifest-path tools/huntctl/Cargo.toml -- milestone inspect .\route.dmsp
```

`format` parses and validates source, then prints its canonical textual form.
It does not edit the source file. `compile` writes canonical `DMSP` binary and
prints its size and program SHA-256. `inspect` strictly decodes a binary,
verifies its hashes and canonical encoding, and prints JSON containing the
program hash, every definition name and hash, named projection identities and
items, and reconstructed canonical source.

## Recorded-trace validation

`evaluate_recorded_trace` applies the same phase, first-hit, stability, bounded
sequence, typed comparison, range, AABB, and plane semantics to immutable
`DUSKTRCE` records. It is intended for predicate regression tests and offline
inspection; it never substitutes a trace value for a channel that was absent,
unavailable, or truncated. Actor catalogs and indexed flags are not current
trace channels, so those queries remain unavailable offline and must be covered
by native observation fixtures or a future trace channel.

The checked cross-language fixtures compile in Rust and decode/evaluate in the
native C++ test target. The recorded-trace fixture is first decoded through the
normal trace wire reader before its authored predicates are evaluated, catching
both trace-schema and predicate-semantic drift.

## Identity and proof invalidation

Every compiled definition embeds a SHA-256 over its name, phase, stability,
operation count, and canonical bytecode. The program embeds a second SHA-256
covering its canonical header and all definition records, including their
hashes. The decoder rejects a digest mismatch, noncanonical encoding, unknown
opcode or field, invalid reservation, truncation, or trailing bytes.

Comments and whitespace do not affect identity. A semantic definition edit
changes that definition's hash and the program hash. Reordering definitions
changes the program hash even when the individual definition hashes remain the
same. Evidence tied to an old program or definition hash does not prove the new
predicate; it must be replayed and observed again.

## Segment goal proofs

A route opts into authored predicates with a contained path relative to its
`.timeline` file. Predicates become route goals only when explicitly attached
to a segment; predicate definitions do not form route topology:

```text
timeline intro
predicate_program intro/milestones.milestones
origin boot predicate process_boot
segment golf439 root profile boot_to_fsp103 uses tas intro/segments/golf439.tas starts process-clean-v1 produces STATE_FINGERPRINT
goal link_control on golf439 predicate link_control
```

Evidence is a separate goal-scoped declaration. A segment may have no proof,
or independent proofs for several goals. A sibling segment may satisfy a goal
defined on the reference sibling without becoming its child:

```text
proof golf439 satisfies link_control program PROGRAM_SHA256 predicate DEFINITION_SHA256 ticks 439
```

The hashes printed by `milestone compile` describe the program to run; copying
them into a timeline is not proof that a tape reached the predicate. A proof is
accepted only after a native replay emits the same
program and definition hashes with its first-hit boundary evidence. `timeline
inspect` and the workbench report missing or stale proof per goal. This does not
invalidate the segment hierarchy, exact fingerprint chain, or ordinary
playback; it prevents the segment from claiming parity, scoring against that
goal, or using that goal for a predicate-backed recording handoff.

Program and predicate pins are a pair. Supplying only one, supplying them
without `predicate_program`, or changing the referenced definition invalidates
the relevant goal proof rather than silently blessing old evidence.

## Read-only guarantee

Native evaluation reads a `MilestoneObservation` snapshot plus phase, boundary,
and tape-position metadata. It never writes game memory or controller state.
The only mutable state is automation-owned bookkeeping such as stability
streaks and first-hit evidence. Because the bytecode contains only field loads,
constants, comparisons, and Boolean composition, authored milestone source
cannot manipulate Link, actors, events, collision, RNG, or input.
