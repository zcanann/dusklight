# Experimental interventions

Glitch discovery sometimes needs a question answered before anyone knows how
to produce the setup naturally: can an enemy impact push Link through this
fence, can a moving object lift him into a loading plane, or can a particular
relative velocity produce a novel collision response?

Typed game-state interventions are appropriate for those causal experiments.
They are not ordinary TAS input and must be impossible to mistake for it.

## Artifact separation

A treatment run has four independently retained inputs and outputs:

1. scenario and checkpoint identity;
2. Link's exact `DUSKTAPE` input;
3. a separate intervention tape; and
4. observations, events, oracle result, and mutation audit trace.

The corresponding control run uses the same scenario and controller tape with
the intervention tape disabled. Comparing the two can establish that a declared
mutation caused a result. It cannot establish that the mutated setup is
naturally attainable.

The intervention format should be a compact canonical binary such as
`DUSKINTR`, compiled from a readable timeline DSL. It must not overload
controller fields or emit a supposedly realized input tape that omits writes.

`intervention` implements `DUSKINTR` v1.0 as a 32-byte header followed by
canonical fixed-size records. The decoder re-encodes and rejects any reserved,
trailing, out-of-order, or otherwise noncanonical bytes. It is capped at 1,024
writes and one million ticks and has magic distinct from both `DUSKTAPE` and
`DUSKCTRL`. The initial readable DSL is line-oriented and requires an explicit
timeline, tick range, `before_game_tick` phase, exact process or placed actor
selector, `actor_exists` precondition, and typed operation. Comments and source
line order do not affect canonical bytes; source size and line count are
bounded before parsing.

## Gating

The implementation should require all of the following:

- a build compiled with `DUSK_ENABLE_EXPERIMENTAL_INTERVENTIONS`;
- an explicit runtime `--allow-gameplay-writes` switch;
- an intervention artifact supplied on the command line;
- a build-capability and fidelity flag in every resulting artifact; and
- a mutation trace that cannot be disabled while writes are active.

Normal builds should compile the executor out. Reactive controllers and actor
catalogs remain read-only regardless of this capability.

The native CMake option `DUSK_ENABLE_EXPERIMENTAL_INTERVENTIONS` defaults off,
is included in build identity, and cannot be enabled without both automation
observers and explicit fidelity models. Rust admission likewise requires the
non-default `experimental-interventions` feature, runtime write opt-in, the
`experimental_typed_gameplay_writes` fidelity, a canonical artifact digest, the
single supported phase and precondition, and a nonempty audit destination.
Every scheduled application consumes an audit entry. Applied writes require a
resolved target and before/written/after values; missing entries prevent audit
completion. A normal build cannot admit the same request.

## Typed operations before raw memory

Start with operations whose semantics and simulation phase can be stated:

- set or add actor position at a pre-simulation boundary;
- set or add linear velocity;
- set facing toward a coordinate or actor;
- move an actor along a bounded curve; and
- establish supported actor-specific intent, such as targeting Link, only when
  the actor implementation provides a typed field or method.

Every operation uses the same actor selectors as reactive controllers: exact
runtime process, placed identity, or an explicit query. It records the resolved
process ID and the value before the write. Target loss is a terminal or declared
no-op result, never an implicit switch to the nearest actor.

Arbitrary address writes should remain a later, separately named unsafe lab
capability. They are difficult to replay across builds and make causal claims
ambiguous.

## Timing and composition

Every write declares a phase such as `before_game_tick` or `after_game_tick`.
The initial implementation should support only one phase so ordering is
unambiguous. Overlapping writes to the same field are rejected unless a later
format defines a canonical composition rule.

An intervention timeline may coexist with a reactive controller, but the two
remain separate streams keyed to the same simulation tick. The controller sees
the observation produced by the previous completed tick; the intervention
executor applies at its declared boundary.

## Fence-push experiment

A useful first benchmark is:

1. restore one validated boundary before the fence encounter;
2. run a no-intervention control with fixed Link input;
3. select one enemy by placed identity or exact process ID;
4. apply a bounded velocity/intent intervention toward Link;
5. record both actors, collision contacts, fence polygon, and Link coordinates;
6. classify whether Link crossed the fence without the normal traversal; and
7. search and minimize intervention timing, direction, and magnitude.

If treatment runs cross the fence and controls do not, we have evidence that
the collision mechanism permits it. The next search objective is to replace the
intervention with achievable enemy manipulation and produce an ordinary cold-
replayable input tape.
