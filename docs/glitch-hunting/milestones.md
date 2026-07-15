# Authored milestones

Milestones turn a read-only game-state predicate into a named, reproducible
boundary. The Rust compiler accepts a small source language and emits a compact
`DMSP` program for native evaluation. The language has no input actions,
assignments, calls, loops, or memory-write operations.

## Source format

A file starts with `milestones 1.0` and contains one or more named definitions:

```text
milestones 1.0

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
operators `==`, `!=`, `<`, `<=`, `>`, and `>=`. Their precedence from highest
to lowest is parentheses, `!`, `&&`, then `||`. A bare Boolean field is shorthand
for `field == true`. Ordered comparisons are available only for numeric fields;
Boolean and symbolic fields accept only `==` and `!=`.

There are no implicit coercions. Integers must fit the field's exact signed or
unsigned width, floats must be finite `f32` values, and symbols must be quoted.
The compiler rejects NaN, infinity, negative zero in a programmatic AST,
unknown fields, excessive nesting, and excessive operations. A program is
bounded to 256 definitions and 1 MiB; each expression is bounded to 256
operations and depth 32.

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
program hash, every definition name and hash, and reconstructed canonical
source.

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
