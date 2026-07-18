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

`intervention::experiment` makes that pairing machine-checkable. One versioned
plan binds the build, scenario, parent boundary, absolute input tape,
observation schema, oracle program, and intervention identities. Both run
artifacts repeat the shared inputs, so validation rejects a control or treatment
captured under any different identity. The control must have gameplay writes
disabled and carry neither an intervention nor write audit. The treatment must
enable exactly the planned intervention and retain its mandatory audit. Both
roles retain their own run, gameplay trace, and oracle-report digests even when
their observed results are identical.

`execute_intervention_experiment` runs the write-disabled control first and the
exactly planned treatment second. Returned artifacts still pass the full pair
validation, so an executor cannot change a shared identity, role, write gate,
or intervention while constructing the result.

The intervention format should be a compact canonical binary such as
`DUSKINTR`, compiled from a readable timeline DSL. It must not overload
controller fields or emit a supposedly realized input tape that omits writes.

`intervention` implements `DUSKINTR` v1.1 as a 32-byte header followed by
canonical fixed-size records. The decoder re-encodes and rejects any reserved,
trailing, out-of-order, or otherwise noncanonical bytes. It is capped at 1,024
writes and one million ticks and has magic distinct from both `DUSKTAPE` and
`DUSKCTRL`. The readable DSL is line-oriented and requires an explicit timeline,
tick range, `before_game_tick` phase, exact process or placed actor selector,
typed existence precondition, and typed operation. Comments and source line
order do not affect canonical bytes; source size and line count are bounded
before parsing. The decoder retains v1.0 support and rejects v1.1 operations in
artifacts that claim the older version.

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

The initial catalog contains only operations whose semantics and simulation
phase can be stated:

- set or add actor position and linear velocity;
- set the signed facing yaw;
- move an actor along a four-control-point cubic curve;
- enable or disable the typed target-player intent;
- set bounded health or one of the named damage, ice-damage, and sword-change
  timers;
- set a bounded actor-status, room-switch, or event-bit flag; and
- spawn a placed actor at a bounded position or despawn an existing actor.

Position, velocity, curve, and spawn components must be finite and within the
declared ±10,000,000 game-unit component bound. Health is limited to 0 through
1,000, timers to 3,600 ticks, actor-status flags to 32 bits, and room/event
indices to 4,096. Curves require at least two ticks. Spawn alone requires
`actor_absent` and a placed selector; every other operation requires
`actor_exists`.

Every operation uses the exact runtime-process or placed-identity selector
forms shared with reactive controllers. It records the resolved process ID and
the value before the write. Target loss is a terminal or declared no-op result,
never an implicit switch to the nearest actor.

`PlacedActorSelector` is the single shared placed identity used by milestone
predicates and interventions. A fallible bridge accepts reactive controllers'
exact process and placed forms while rejecting `nearest`; the controller's
separate actor procedure becomes part of the placed identity. A run that loses
an exact target may retain that audit outcome, but cannot complete as accepted.

Arbitrary address writes should remain a later, separately named unsafe lab
capability. They are difficult to replay across builds and make causal claims
ambiguous.

## Timing and composition

Every write declares a phase such as `before_game_tick` or `after_game_tick`.
The initial implementation should support only one phase so ordering is
unambiguous. Overlapping writes to the same field are rejected unless a later
format defines a canonical composition rule.

Validation treats set/add position and cubic motion as one position field,
set/add velocity as one velocity field, and keys named timers and flags by
their typed identities. Independent fields and actors may share an interval.
Spawn and despawn are lifecycle writes and conflict with every simultaneous
write to the same placed identity.

`intervention::parameter_search` defines up to 16 typed, bounded axes over start
tick, duration, vector or cubic-curve components, facing yaw, health, and named
timer duration. It emits at most 4,096 deterministic low-discrepancy proposals,
deduplicates their canonical `DUSKINTR` bytes, and discards candidates that fail
the normal timeline, semantic-bound, or overlap validation. Parameter values
are applied to the seed's original intervention indices before canonical
reordering, so timing search cannot silently retarget an axis.

The minimizer moves each declared axis toward zero or its nearest allowed bound
and performs bounded refinement only while the caller's exact causal predicate
continues to pass. Integer-backed timing, yaw, health, and timer values use
their canonical rounded representation; the result reports the realized values,
exact minimized tape, and evaluation count.

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
