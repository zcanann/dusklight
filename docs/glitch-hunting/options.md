# Semi-Markov option executions

`dusklight-option-execution/v1` is the portable record of one realized
temporally extended controller option. An option is a search and authoring
convenience; the canonical raw tape remains replay and proof authority.

Each record contains:

- a stable option ID and typed option kind;
- a bounded map of typed parameters using booleans, integers, exact binary32
  bits, exact binary32 vectors, text, or content digests;
- minimum, maximum, and realized 30 Hz durations;
- one declared termination condition and a bounded ordered set of cancellation
  conditions;
- a typed end reason, including the exact cancellation-condition index;
- every emitted raw four-port `InputFrame`;
- the realized half-open `[start_frame, end_frame_exclusive)` tape range; and
- the SHA-256 digest of the complete canonical DUSKTAPE artifact.

Conditions are versioned typed values: duration elapsed, an authenticated
milestone predicate, an authenticated observation expression, target reached,
or target lost. Predicate and observation identities require nonzero content
digests. Target names and option/parameter identifiers are bounded portable
ASCII rather than host paths or pointers.

The duration and tape range must equal the emitted-frame count. Completed and
terminated options cannot finish before their declared minimum. Cancellation
may finish early but must reference a declared cancellation condition. A
maximum-duration result must end at the exact maximum. Every emitted frame must
be absolute; conditioned/wait frames are rejected.

`OptionExecution::capture` copies the exact raw frames from an existing tape
range and authenticates the complete tape. `validate_against_tape` recomputes
the canonical DUSKTAPE digest, bounds-checks the range, and compares every raw
frame. Changing an unrelated tape frame therefore invalidates the digest, while
changing a frame inside the realized range also fails the exact frame check.
This prevents an adaptive controller from reporting a cleaned-up trajectory
that differs from what the game actually consumed.

Option kinds cover the current and planned tactic families—movement, alignment,
rolls, combat, items, traversal, paths/curves, and feedback controllers—with a
versioned custom namespace for experiments. Adding an option executor does not
change the proof format: successful execution must always finish by capturing
its realized raw range.

## Typed roll planning

`dusklight-roll-option/v1` defines a bounded roll before it becomes raw input:

- camera-relative direction in integral degrees (`0` forward, `+90` right) and
  main-stick magnitude;
- the exact zero-based option frame that presses the GameCube A action button;
- an exact count of direction-only recovery frames after the A frame;
- an absolute spacing period and phase, requiring
  `absolute_button_tick % period_ticks == phase_tick`; and
- a bounded ordered set of the same typed cancellation conditions used by the
  option-execution record.

The planned duration is `button_frame + 1 + recovery_frames` and is capped at
10,000 ticks. A cancellation hit is sampled before input for its declared local
tick, must reference a declared condition, and truncates the half-open output
at that tick. Consequently, cancellation at the button frame does not emit B.
Tick zero and cancellation at or after normal completion are rejected instead
of producing empty or falsely cancelled executions.

Spacing phase is evaluated on the absolute tape timeline, not relative to each
roll. A phase mismatch is an authoring/scheduling error; the planner never
silently inserts neutral frames or shifts the requested button. Period one,
phase zero is the unconstrained compatibility behavior used by legacy search
roll macros. Search candidates may now provide an explicit button frame and
spacing pair, and their compilation uses this same planner.

`RollOptionPlan::capture_execution` re-realizes the plan at the requested tape
offset, compares every frame in the range, and then produces the authenticated
`OptionExecution`. Direction, magnitude, button/recovery timing, and spacing
period/phase are retained as typed parameters while the raw frames and full
tape digest remain authoritative.

### Proof-anchored local golf

`dusklight-option-relative-golf/v1` builds a finite semantic neighborhood around
a successful roll execution. The golfer first validates the execution against
its complete tape and requires the supplied roll plan plus cancellation hit to
reproduce that execution exactly. A stale plan, cleaned-up raw range, changed
parameter, or mismatched cancellation therefore produces no proposals.

The neighborhood changes one axis at a time in both directions: heading,
magnitude, duration, absolute spacing phase, button timing, and cancellation
tick. Button-timing proposals preserve total option duration. Phase proposals
shift the absolute option start and update the declared congruence while
leaving local button timing alone. Cancellation proposals exist only when the
proved seed declared and realized a valid cancellation. Bounds simply omit an
invalid neighbor rather than clamping two proposals into a disguised duplicate.

Every proposal includes its typed roll plan, absolute start tick, optional
cancellation hit, and exact raw realization. It is still only a candidate: the
experiment must place it in context, evaluate the goal, capture a new option
execution, and cold-replay the resulting absolute tape before promotion.

Generate a JSON proposal manifest without launching the game:

```text
huntctl search golf-option --plan build/roll.json \
  --execution build/roll.execution.json --tape build/success.tape \
  --output build/roll-golf/proposals.json \
  --cancellation-tick 5 --condition-index 0
```

## Game-specific tactic plans

`dusklight-game-tactic/v1` gives each currently required game tactic a typed,
bounded parameter tuple and deterministic raw expansion:

- jump attack holds L targeting, presses A on its declared attack phase, and
  retains the declared movement direction through windup and recovery;
- normal attacks and 2–8-hit combos use explicit B press, gap, and recovery
  counts;
- shield, target, interact, and transform use the canonical R, L, A, and
  D-pad-down bindings respectively;
- generic item use, boomerang, clawshot, and spinner require an explicit X or Y
  assignment—semantic item identity never guesses the equipped slot;
- crawl, climb, and swim carry camera-relative direction, magnitude, duration,
  and whether A is held; and
- Epona movement carries the same direction tuple plus an exact initial count
  of A spur frames.

Every duration is in 30 Hz ticks, every direction is an integral degree in
`[-180, 180]`, and every expansion is capped at 10,000 ticks. These are input
recipes, not claims that the current player procedure will accept an action.
Preconditions and completion remain authenticated predicates/observations, and
the game-consumed raw tape is still proof authority.

All tactic plans support the same pre-input typed cancellation convention as
roll plans. `GameTacticPlan::capture_execution` compares the exact shortened or
completed frame range and records semantic parameters—item slot, combo count,
phase durations, direction, magnitude, action hold, or spur count—in the common
option execution. Static search candidates expose plans as `game_tactic`
macros but reject reactive cancellation declarations; adaptive executors must
materialize those through execution capture.

## Exact static motion paths

`dusklight-motion-path/v1` compiles four bounded stick-space path forms directly
to canonical raw frames:

- waypoint paths hold uniformly distributed integer stick points;
- rails interpolate linearly through every point;
- splines use uniform Catmull–Rom interpolation with duplicated endpoint
  controls; and
- Bézier paths use exactly four cubic control points.

Points cover the complete signed raw main-stick range `[-128, 127]`. Duration
is the exact output count, from 1 through 10,000 ticks. Sampling phase is the
rational `numerator/denominator`, with a bounded positive denominator and
`0 <= numerator <= denominator`. Phase zero samples tick starts; the distinct
`denominator/denominator` phase samples tick ends and therefore reaches the
final control point on the last tick.

Linear and cubic evaluation uses integer numerators/denominators and defined
ties-away-from-zero rounding before the raw-stick clamp. It does not depend on
host floating-point interpolation. Multi-segment rails and splines distribute
the rational global phase uniformly across their segments, with the exact path
endpoint assigned to the final segment.

These paths are deliberately controller/stick-space programs. They do not
claim to follow world geometry without observations; world/player/camera
targets belong to the reactive controller path. Static search exposes paths as
`motion_path` macros and rejects reactive cancellation declarations. Adaptive
realizations can cancel before a tick and authenticate their exact prefix with
`MotionPathPlan::capture_execution`.

`dusklight-motion-path-relative-golf/v1` applies the same proof-anchored rule as
roll golf to waypoint, rail, spline, and Bézier plans. It validates the seed
execution against the full tape and reproduces it before emitting any proposal.
The deterministic neighborhood changes one point's X or Y coordinate at a
time, then path duration, rational sample-phase numerator, and an optional
cancellation tick. Every bounded proposal includes its exact realized raw
frames and remains an ordinary evaluator candidate:

```text
huntctl search golf-path --plan build/path.json \
  --execution build/path.execution.json --tape build/success.tape \
  --output build/path-golf/proposals.json --point-step 1 \
  --duration-step 1 --phase-step 1
```

Observation-free DUSKCTRL programs can likewise be flattened directly to a
canonical tape with `huntctl controller flatten`. Reactive programs are not
flattened ahead of execution; controller inspection labels each such layer with
the versioned observation-field and selector provenance it requires, and only
the realized absolute tape can be promoted as action proof.

## Tactic diagnostic records

`dusklight-option-diagnostic/v1` aligns diagnostics to an authenticated
`OptionExecution` without weakening the tape authority. The embedded execution
supplies the option start, exclusive end, typed end reason, and exact emitted
raw frames. One contiguous row per realized tick additionally records:

- the exact game-consumed four-port input frame;
- an optional binary32-bit error vector;
- whether each main-stick, sub-stick, trigger, or analog field was clamped; and
- the actual guidance action ID, recommendation decision, schema digest, and
  an explicit assertion that proof remained unrestricted;
- typed contact position/normal/surface evidence and optional normalized
  gameplay-viewport coordinates; and
- typed goal-progress samples plus an optional intended-target viewport point.

The record carries one typed intended target: none, coordinate, heading, actor
selector plus optional session process ID, or a named semantic target with
typed parameters. Validation rejects a tick-count/order mismatch, any raw frame
that differs from the authenticated option execution, invalid targets, or an
action-mask record that claims guidance restricted proof. This gives route and
overlay tools a single portable source for option intervals, goal error, mask
advice, controller output, clamps, and what the game actually consumed.

One or more records can be bound to their complete canonical tape in a
`dusklight-option-diagnostic-bundle/v1` sidecar named
`<artifact>.options.json`. The bundle repeats the tape SHA-256, validates every
embedded execution against the decoded tape, and requires option ranges to be
ordered and non-overlapping. The route workbench ignores missing sidecars, but
surfaces malformed, stale, symlinked, oversized, or digest-mismatched sidecars
as diagnostic errors instead of presenting their contents as evidence.

Route-workbench graph v8 projects only validated bundles. Graph nodes show the
option count; selection details render the full option interval strip,
game-consumed main-stick and camera curves, clamp samples, intended targets,
contact/surface evidence, and per-goal progress tracks. When a terminal-frame
thumbnail and normalized viewport evidence are both available, target and
contact markers are layered over that gameplay image. World-space evidence is
still listed when no trustworthy screen projection was recorded.

## Reusable procedure/mode tactic cases

`dusklight-tactic-test/v1` keeps deterministic tactic regressions separate from
claims about gameplay acceptance. Each reusable case names an exact native
`PROC_*` token, one of the human, wolf, horse, crawl, climb, or swim modes, and
required/forbidden `mModeFlg` bits. It pairs that context with a complete typed
tactic plan, expected option type and duration, and hand-authored PAD samples at
specific local ticks.

The built-in catalog covers all 15 current tactic families and all six modes,
including alternate procedure contexts such as climb side movement, swimming
clawshot use, wolf transformation, and horseback boomerang use. Catalog tests
reject duplicate IDs, invalid procedure/mode pairs, missing riding/swimming/
climbing flags, drift in exact button/stick recipes, and loss of any tactic
family or mode. `TacticProcedureContext::matches` lets stage-boot or tape-backed
gameplay fixtures reuse the same cases while requiring exact observed procedure
and mode flags. Passing the static catalog proves the input recipe only; a
gameplay fixture must still record its observed procedure transition and goal
evidence before promotion.
