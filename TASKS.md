# Route learning

## Goal

Build the smallest generic learner that starts when Link gains control, chooses
actions until the native Ordon Springs load-zone predicate fires, and minimizes
native elapsed ticks.

The scored campaign may use generic observations and a fixed route-agnostic
action library. It may not use a human demonstration, inherited route, authored
waypoints, route-specific reward, proxy terminal, or an Ordon-derived tactic.

The known human route is 125 ticks. The targets are 124, then 123, then 120 or
less. A result counts only when its selected controller tape reaches the real
terminal and cold-replays twice from the named root with identical terminal
identity and tick count.

The retained 190-tick route proves execution and replay, not learning quality.
It is not an input to the scored campaign.

## Rules

- Add no mechanism until a measured failure of the simpler system requires it.
- The objective is terminal success with minimum ticks. Motion, velocity,
  camera, rolling, collision, and history are observations, not authored reward
  terms.
- Run bounded experiments and reject failed approaches. Do not accumulate
  treatment versions.
- Use at most two native or build workers on this machine and own every child
  process directly.
- Keep the workspace clean and commit/push completed milestones.

## Work queue

### 1. Run one plain scratch learner

- [x] Add a thin scratch mode that reuses the existing native worker, terminal
  predicate, observation capture, action execution, and exact replay, while
  bypassing demonstrations, inherited routes/policies, graph scheduling,
  frontier critics, calibration, save-state branching, and tactic mining.
- [x] Give it one small generic option set alongside raw controller input:
  move along a chosen heading, roll along a chosen heading, camera-align plus
  movement, and camera-align plus roll. Options are bounded and interruptible;
  heading and duration are parameters.
- [x] Represent state with coarse generic motion cells: position, velocity,
  facing, camera orientation, prompted-action availability, and a short recent
  input/motion history.
- [x] Implement one semi-Markov Q-style loop:
  epsilon-greedy option selection, native tick cost on every transition, the
  authenticated terminal as success, a single small value table/approximator,
  and one backward return pass through completed episodes. Do not add another
  model, policy partition, or shaped reward.
- [x] Start every episode from the authenticated root and allow at least 900
  ticks. Retain all unique transitions and the fastest successful tape. Avoid
  exact duplicate episodes; add no novelty objective yet.
- [x] Make each learner update affect the next eligible action choice. Report
  completed episodes, unique transitions, terminal episodes, fastest selected
  ticks, updates, changed choices, native time, wall time, and time to first
  terminal.
- [x] Automatically cold-replay every strict winner twice.

Implementation checkpoint (2026-08-02): `huntctl learn scratch-route` owns one
native child per cold-root episode, uses a fixed 256-action route-agnostic
catalog and one coarse tabular learner, and persists a checksummed binary
checkpoint. A two-episode Windows smoke resumed that checkpoint, executed
1,800 logical native ticks, retained 200 unique transitions, applied 210
backward-return updates, and changed 141 greedy choices in 81.6 seconds total
wall time. It found no terminal; this is execution evidence, not the
intermediate gate.

- [x] Intermediate gate: run five fixed ten-minute seeds and reach the real
  load zone from scratch in minutes, not hours. If fewer than three seeds find
  a terminal, stop and diagnose section 2 instead of extending the run.

Gate result (2026-08-02): failed at 2/5 seeds. Each seed ran 36 cold-root,
900-tick episodes. Seeds 130363 and 181081 produced two authenticated terminal
episodes each, improving 624 to 481 ticks and 876 to 561 ticks respectively;
seeds 104729, 155921, and 208609 produced none. Across the gate the learner
retained 11,815 unique transitions in 2,601 seconds wall time. This proves
occasional scratch discovery, but not the required reliability or route
quality.

Exit: the same minimal learner selects and reproduces a zero-shot route of 124
ticks or less.

### 2. Diagnose the first failed gate

Do not implement every branch. Collect enough evidence to select exactly one.

- [x] Determine whether failure is caused by action expressivity, insufficient
  exploration, incorrect value/return propagation, or insufficient native
  samples. Prove the classification with the retained episode stream and a
  focused deterministic test.
- [x] Record the diagnosis and the smallest proposed intervention in this file
  before implementing it.

Diagnosis (2026-08-02): **insufficient exploration before the first terminal**.
The unchanged action catalog reached and cold-replayed the real load zone four
times, so basic expressivity is present. In both successful seeds the later
terminal was substantially faster than the first and thousands of deployed
updates changed greedy choices, so backward credit is operating. Native work
consumed 2,407 of 2,601 wall seconds and still yielded thousands of unique
transitions per seed, so orchestration overhead is not the first failure.
However, an equal-horizon censored episode gives every root action the same
-900 return regardless of where its trajectory went; a deterministic learner
test now fixes that fact. Until a sparse terminal is found, ordinary Q value
therefore contains no directional discovery signal and three seeds never
escaped that condition.

Smallest proposed intervention: until the first authenticated terminal only,
train the existing Q table on a count-based novelty return over coarse position
cells. Infrequently visited cells provide the only exploration value and native
ticks remain the only cost. Persist the cell counts. On the first terminal,
clear the novelty-trained values, seed the same table from that successful
episode using the ordinary terminal/tick return, and permanently disable
novelty. This adds no route coordinate, waypoint, collision heuristic, second
model, or retained trajectory input. Merely choosing the least-visited action
in the current full state is not an intervention: the plain learner already
prefers its unvisited zero-valued actions over negatively valued failures.

Intervention result (2026-08-02): **passed, 5/5 successful seeds** under the
same ten-minute wall budget. The v3 binary checkpoint persists coarse cell counts;
the first authenticated terminal clears the temporary values and permanently
returns selection to the terminal/tick learner.

| Seed | Control terminals | Treatment terminals | Treatment best | First terminal |
| ---: | ---: | ---: | ---: | ---: |
| 104729 | 0 | 3 | 540 | 76.7 s |
| 130363 | 2 | 2 | 481 | 266.9 s |
| 155921 | 0 | 3 | 368 | 516.0 s |
| 181081 | 2 | 2 | 561 | 35.5 s |
| 208609 | 0 | 8 | 488 | 50.2 s |

The treatment produced 18 terminal episodes versus 4 in control across 208
cold-root episodes and 13,671 unique transitions. Its median per-seed best was
488 ticks and its best was 368. Eleven strict winners each cold-replayed twice.
Seed 181081 reached the terminal on episode zero with the
same action-sequence digest as control, retained zero novelty cells, and then
reproduced the same 561-tick best; this proves the exploration rule does not
leak past immediate discovery. Retain the intervention.

Conditional interventions:

- **Action expressivity:** add only the missing generic option or parameter and
  prove it can improve held-out trajectories.
- **Exploration:** add one coarse state/trajectory novelty rule until the first
  terminal, then disable it for tick optimization.
- **Credit assignment:** correct the Q/return implementation or replace the
  single estimator; do not add parallel critics.
- **Throughput:** measure one versus two workers and root replay versus retained
  save states, then keep only the change that improves unique authenticated
  transitions per second end to end.
- **Local optimum after success:** add one simple successful-trajectory mutation
  loop (delete, shorten, change heading, or change roll timing) while keeping
  terminal success as a hard constraint.

Exit: the selected intervention makes the failed gate pass under the same fixed
budget. Remove it if it does not.

### 3. Establish the benchmark result

- [x] Add one deterministic post-success mutation loop over the fastest
  successful action sequence: delete one option, cold-root evaluate the
  candidate, and accept it only when the real terminal still fires at a strict
  lower tick. Enumerate each deletion once before repeating any; failed
  mutations cannot alter the incumbent or Q table. Do not add another edit
  family until deletion is measured.
- [x] Prove with a focused deterministic test that deletion candidates are
  complete, non-duplicated, resumable, and terminal failures are rejected.
- [x] Re-run five fixed zero-shot seeds with deletion enabled and retain the
  full result distribution, not only the luckiest seed.
- [x] Correct the measured scheduler failure without adding an edit family:
  after the first terminal, alternate one ordinary Q episode with one pending
  deletion candidate. A new strict Q winner resets deletion enumeration to the
  better incumbent. Failed deletion candidates still make zero Q updates.
- [x] Re-run the same fixed five seeds with alternation. It did not improve on
  the 360-tick best and again regressed three seeds, so reject the fixed cadence.
- [x] Replace fixed cadence with one event-driven deletion attempt immediately
  after each strict Q winner. A deletion winner cannot schedule another
  deletion; prove the persisted last episode makes this resumable and keeps Q
  dominant without changing the operator or Q updates.
- [x] Re-run the same five seeds with event-driven deletion. It preserved three
  seeds but exposed that deletion episodes consume global episode ordinals and
  therefore perturb every later seeded Q selection; do not treat it as an
  isolated scheduler result.
- [x] Give Q selection a learning-only episode ordinal derived from persisted
  episode modes. A deletion episode must not consume or renumber Q's seeded
  exploration stream; prove this across resume without adding checkpoint state.
- [x] Re-run affected seeds 181081 and 208609 first. Both exactly recovered
  their 561- and 488-tick novelty-only results.
- [x] Complete the isolated five-seed distribution by rerunning 104729, 130363,
  and 155921. Retain sparse deletion only if the full result preserves the
  broad Q gains and supplies useful accepted local improvements.
- [x] Diagnose the retained 360-tick winner before spending more mining time.
  Distinguish missing control expressivity from an incumbent that deletion
  alone cannot repair.
- [x] Add exactly one next local edit family over the learner's own incumbent:
  replace one option's heading with either adjacent heading already present in
  the fixed 16-heading catalog while preserving its family and duration.
  Enumerate candidates once, persist progress, cold-root evaluate, and accept
  only a strict twice-replayed terminal improvement. Failed candidates make no
  Q update. Do not add fine headings or another global action yet.
- [x] Run the bounded adjacent-heading pass on the 360-tick incumbent. Retain
  the edit only if it improves the real terminal tick; exhaust or reject it
  before considering finer heading parameters.
- [x] Add one finer local parameter treatment over the correctly exhausted
  adjacent-heading incumbent: map the 16-heading sequence losslessly into a
  32-heading catalog and propose only each option's two 11.25-degree neighbors.
  Preserve family and duration, keep the global learner at 16 headings, bind
  the source checkpoint, resume deterministically, and require two exact cold
  replays.
- [x] Run the bounded 32-heading pass to exhaustion. Retain it only if it
  improves the real terminal tick; otherwise reject it before adding a
  duration, boundary, or multi-option edit family.
- [x] Add read-only incumbent introspection and compare the corrected 333-tick
  sequence with the real but mischaracterized 299-tick sequence. Select the
  next single edit family from observed family, duration, and boundary changes,
  not from another undirected mining run.
- [x] Add one duration-shortening edit family over the 333-tick incumbent.
  Preserve heading, tactic family, and lock/roll timing while moving only to
  the next shorter duration or recovery parameter already in the catalog.
  Enumerate once, resume deterministically, and require two exact cold replays;
  failed candidates make no Q update.
- [x] Run the bounded duration pass to exhaustion. Retain it only if it improves
  the real terminal tick; diagnose its result before permitting cross-family
  roll/raw substitution or a multi-option edit.
- [x] Add one roll-promotion edit family over the 333-tick incumbent: replace a
  single non-roll movement option with the shortest roll at the same semantic
  heading. Do not change the option boundary or angle in the same candidate.
  Persist deterministic progress and require two exact cold replays for a
  strict terminal-tick improvement.
- [x] Run the bounded roll-promotion pass to exhaustion. Retain it only if it
  improves the real terminal tick; diagnose it before permitting non-local
  headings, raw substitutions, or multi-option edits.
- [x] Make authenticated refinement outputs composable. A strict winner from a
  heading or local-option pass must expose one common incumbent contract that
  another existing edit family can consume by exact checkpoint hash, without a
  manual tape/action migration or weakening v1 diagnostic-only rejection.
- [x] From the retained 242-tick roll incumbent, run the existing coarse-heading,
  duration, and roll families as deterministic coordinate descent. Exhaust each
  frontier, accept only twice-replayed strict terminal improvements, and stop
  when a complete cycle makes no improvement before adding a new edit family.
- [x] Run standalone single-option deletion from the authenticated 229-tick
  fixed point. Retain only twice-replayed strict terminal improvements; if it
  changes the incumbent, recompute heading, duration, roll, and deletion to a
  new full-cycle fixed point before selecting another family.
- [ ] Reproduce 124 ticks or less, then continue the unchanged generic process
  to 123 ticks and 120 ticks or less.
- [ ] Verify that ordinary seed ordering and update cadence do not erase the
  useful behavior.

Exit: a zero-shot route reaches 120 ticks or less and cold-replays twice with
identical evidence.

Deletion checkpoint (2026-08-02): the v4 learner persists a deterministic set
of unique single-option deletions for the current fastest authenticated action
sequence. Each candidate runs from the cold root; failed or non-improving
candidates make zero Q updates and cannot replace the incumbent. A four-episode
seed-181081 native smoke handed off from learning to three deletion attempts:
the first removed option 9, reached the real terminal, cold-replayed twice, and
improved 876 to 868 ticks; the next two failed candidates made zero learner
updates. Focused tests cover complete deterministic enumeration, duplicate-free
resume, failure rejection, and incumbent reset after acceptance.

Exclusive-deletion result (2026-08-02): measured and rejected as a scheduler,
not as an operator. Across the five fixed ten-minute seeds it attempted 159
deletions, 33 still reached the terminal, and 30 were strict cold-replayed
winners. Per-seed best ticks were 567, 589, 360, 852, and 391, compared with
540, 481, 368, 561, and 488 for continued Q learning. Deletion established a
new overall best by 8 ticks and helped two seeds, but regressed three seeds and
worsened the median from 488 to 567 because it monopolized every post-terminal
episode. The smallest correction is deterministic Q/deletion alternation; do
not add shortening, heading, roll-timing, another model, or more wall time yet.

Alternation checkpoint (2026-08-02): cadence is derived from the last persisted
episode mode, so checkpoint resume needs no mutable scheduler field. A fresh
four-episode seed-181081 native smoke ran learning, deletion, learning,
deletion. The accepted deletion improved 876 to 868 ticks and cold-replayed;
the intervening learning episode made 112 Q updates, while the failed final
deletion made zero. The full learning-framework audit passed 431 orchestration
tests before the native smoke.

Fixed-alternation result (2026-08-02): measured and rejected. The same five
ten-minute seeds produced best ticks of 391, 614, 377, 868, and 383 (median
391). This improved the deletion-only median of 567 and novelty-only median of
488, but did not beat the 360 deletion-only best and regressed three of five
seeds against both prior treatments. Alternation spent 82 of 224 episodes on
deletion even though Q produced only one or two strict winners per seed. The
next bounded treatment schedules exactly one deletion candidate after a strict
Q winner; it does not let deletion success chain or add another edit family.

Event-driven checkpoint (2026-08-02): a fresh three-episode seed-181081 native
smoke ran learning, deletion, learning. The initial Q winner reached 876 ticks;
its single deletion improved to 868 and cold-replayed twice; the next episode
returned to ordinary Q and made 112 learner updates. Unit coverage proves that
only a persisted strict learning winner opens a deletion slot, including after
resume; deletion success, deletion failure, and non-winning learning all return
to Q. The full 431-test framework audit passed.

Unisolated event-driven result (2026-08-02): best ticks were 405, 481, 360, 868,
and 722 (median 481). Only 8 of 214 episodes were deletions, and three seeds
matched or improved their novelty-only result, but seeds 181081 and 208609
regressed by 307 and 234 ticks. Inspection found that the global episode index
seeds Q selection, so inserting even one forced deletion changes the random
stream of every subsequent Q episode. Separate the learning ordinal before
drawing any conclusion about the sparse scheduler.

Q-ordinal isolation checkpoint (2026-08-02): Q selection now derives its
episode ordinal by counting persisted learning modes, so deletion does not
consume an exploration seed and resume needs no added checkpoint state. The
previously regressed seeds 181081 and 208609 exactly recovered their
novelty-only bests of 561 and 488 ticks. They spent only 2 and 3 episodes on
deletion; seed 181081 accepted one local winner, while seed 208609 accepted
none. The full audit passed 432 orchestration tests before the native runs.

Isolated sparse-deletion result (2026-08-02): retained. The five fixed seeds
produced best ticks of 540, 481, 360, 561, and 488 (median 488). Every seed
exactly preserved its novelty-only Q best; seed 155921 additionally improved
368 to 360 through two accepted deletions that each cold-replayed twice. Across
the distribution, deletion consumed only 11 episodes instead of 82 under fixed
alternation. This is the final measured scheduler treatment; move to closing
the 360-to-124 route-quality gap rather than tuning cadence again.

Route-quality diagnosis (2026-08-02): the 360-tick winner contains 44 options
and 23 distinct frame inputs. It holds camera-relative forward for 299 of 361
route frames and presses a button for 43 frames, so the generic camera-lock and
roll actions are present and selected; basic expressivity is not missing. A
retained 128-frame fast-route artifact uses 81 distinct inputs and changes
input on 112 of 127 frame boundaries, while the scratch route changes on only
99 of 360. Deletion can remove a whole bad option but cannot correct its
heading. First measure adjacent substitutions already available in the fixed
catalog; only a failure there justifies finer headings or a parameter learner.

Adjacent-heading checkpoint (2026-08-02): `huntctl learn
refine-scratch-headings` reads the authenticated binary scratch checkpoint and
enumerates both neighboring headings for every incumbent option while retaining
its exact family and duration. Its separate checksummed, compressed binary
checkpoint binds the source checkpoint, request, execution, action universe,
seed, incumbent, attempts, and sealed report. Failed candidates never touch Q;
strict winners must cold-replay twice before resetting enumeration around the
new incumbent. The corrected implementation addresses headings through stable
option IDs rather than sorted catalog positions, supports an authenticated
16-to-32-heading refinement chain without enlarging the learner's action space,
and versions invalid v1 frontiers out of resume. Focused tests prove exact
family-preserving neighbors, lossless coarse-to-fine mapping, deterministic
resume, and corruption rejection. The full framework audit passed 437
orchestration tests and the 589-file source-size gate. A corrected seed-155921
native smoke resumed from attempt one to two, reduced the remaining frontier
from 87 to 86 without a duplicate, and preserved the 360-tick incumbent across
two non-winning terminal candidates.

Invalidated adjacent-heading result (2026-08-02): the 274-candidate run reached
a real, twice-replayed 299-tick route, but a focused coarse-to-fine catalog test
exposed that `TacticAssetCatalog` sorts entries by option ID. The original edit
decoded sorted action indexes with division and modulo as though catalog
insertion order were preserved, so it did not reliably retain family or
duration and is not evidence for heading-only refinement. Do not use its
checkpoint as the fine-heading source. Semantic option-ID addressing replaces
the invalid index geometry, the checkpoint format advances to v2 so the old
frontier cannot resume under new semantics, and the bounded adjacent pass is
reopened from the authenticated 360-tick scratch incumbent.

Corrected adjacent-heading result (2026-08-03): retained and exhausted. The
semantic seed-155921 pass evaluated 438 family-and-duration-preserving
candidates; 418 reached the real load-zone terminal and 16 strict improvements
each cold-replayed twice. It reduced the authenticated scratch incumbent from
360 to 333 ticks and stopped with zero candidates remaining. The run consumed
152,125 native ticks. A host suspension inflated one slice's native and total
wall clocks, so this artifact is valid route-quality evidence but must not be
used as throughput evidence. Coarse heading repair is useful but supplies only
27 of the 236 ticks needed to reach 124; proceed to the already bounded
32-heading midpoint treatment rather than mining the exhausted frontier.

Fine-heading checkpoint (2026-08-03): `huntctl learn
refine-scratch-fine-headings` authenticates both the original scratch source and
the exhausted semantic-heading checkpoint, maps every retained 16-bin action to
its identical even 32-bin action, and proposes only the two 11.25-degree
neighbors. The learner's catalog remains unchanged. A native seed-155921 smoke
resumed from attempt one to two, reduced the frontier from 87 to 86 without a
duplicate, reached the real terminal twice, and preserved the 333-tick
incumbent.

Fine-heading result (2026-08-03): rejected and exhausted. The 32-heading pass
evaluated all 88 midpoint candidates from the corrected 333-tick incumbent;
84 reached the real load-zone terminal, none was a strict improvement, and the
frontier stopped with zero candidates remaining. It consumed 29,765 native
ticks over 928 seconds of wall time. Angular resolution is not the active local
bottleneck. Diagnose the retained option sequence against the real 299-tick
artifact before choosing the next single edit family; do not add more headings
or resume either exhausted frontier.

Incumbent diagnosis (2026-08-03): the checksummed read-only inspector accepts
v1 checkpoints for diagnosis but execution still rejects them. The valid
333-tick and legacy 299-tick routes both contain 44 options, with nominal
durations of 368 and 345 ticks. Seventeen options differ. Four structural
changes explain the full 23-tick nominal reduction: two 8-tick camera moves
became rolls, one 8-tick move became a 1-tick raw input, and one 16-tick camera
roll became an 8-tick camera roll; the other thirteen are heading changes. The
valid incumbent has only four same-family shortening opportunities: one
16-tick camera move, two 16-tick camera rolls, and one 8-tick move. Measure
those duration parameters first. Cross-family roll/raw substitutions and
non-local heading changes remain separate hypotheses.

Duration-refinement checkpoint (2026-08-03): `huntctl learn
refine-scratch-durations` accepts only an exhausted authenticated v2 heading
source and enumerates the next shorter same-family catalog parameter while
preserving heading and lock/roll schedule. Its separate compressed,
checksummed, atomic binary checkpoint binds the source, request, execution,
action universe, incumbent, attempts, and sealed report. Failed candidates do
not update Q, and strict winners must replay exactly twice before resetting the
small frontier. Focused tests cover semantic shortening, duplicate-free resume,
binary round trip, and corruption rejection. The full framework audit passed
440 orchestration tests and the 590-file source-size gate.

Duration-refinement result (2026-08-03): rejected and exhausted. All four
same-family shortening candidates reached the real load-zone terminal, at 334,
342, 333, and 343 ticks, but none strictly improved the 333-tick incumbent.
The pass consumed 1,356 native ticks over 99 seconds and stopped with zero
candidates remaining. The equal 333-tick camera-roll shortening remains
rejected under the strict speed criterion. Shorter option duration is not the
active bottleneck. The real 299-tick artifact has two more rolls than the valid
incumbent, so isolate same-heading roll promotion next rather than combining
family and heading changes.

Roll-refinement checkpoint (2026-08-03): duration shortening and roll promotion
now share one 933-line local-option refinement engine rather than duplicating
execution, persistence, replay, and report code. The checkpoint and report bind
the edit kind, so `shorten_duration` cannot resume as `promote_roll`. Roll
candidates replace one non-roll option with `r03` at the same parsed semantic
heading and change no other boundary or angle. Focused tests cover mapping,
edit-kind persistence, resume, and corruption rejection. The full framework
audit passed 441 orchestration tests and the 590-file source-size gate.

Roll-refinement result (2026-08-03): retained and exhausted. The same-heading
pass evaluated 82 candidates; 29 reached the real load-zone terminal and six
strict winners each cold-replayed twice. The accepted camera-move-to-`r03`
promotions improved the route through 326, 302, 287, 258, 246, and finally 242
ticks, then the frontier stopped with zero candidates remaining. It consumed
14,982 native ticks over 696 seconds. The selected route now has seven rolls,
243 route frames, 22 distinct inputs, and 71 input changes. Roll availability
was a major local bottleneck, but 242 remains 118 ticks above 124. Compose the
already implemented edit families around this incumbent before inventing raw,
non-local-heading, or multi-option treatments.

Composable-incumbent checkpoint (2026-08-03): scratch, semantic-heading, and
local-option checkpoints now enter heading, duration, and roll refinement
through one authenticated incumbent contract. The contract verifies the exact
checkpoint hash, request, execution, seed, action universe, exhaustion state,
and semantic option sequence; downstream checkpoints bind that exact source
hash. Stable option IDs preserve same-resolution composition and the existing
16-to-32 mapping, while diagnostic-only v1 heading checkpoints remain rejected
for execution. The CLI accepts an exhausted option checkpoint directly via
`--option-source`, so the retained 242-tick roll result can enter the next
coarse-heading pass without tape migration. The full framework audit passed
442 orchestration tests and the 591-file source-size gate.

Incumbent-migration checkpoint (2026-08-03): the retained 242-tick option
checkpoint preserved only the optimization-request digest; its exact request
file was not retained, so it cannot honestly enter another authenticated run
under the old execution binding. `huntctl learn migrate-scratch-option-incumbent`
now provides the bounded playback-and-record migration needed for that case. It
loads the exhausted semantic option sequence without accepting it as current
authority, executes it against a retained sealed request/execution, requires
the same real terminal tick, requires two exact cold replays, and only then
writes a checksummed compressed binary incumbent checkpoint containing the
tape and replay evidence. All refinement families accept that checkpoint by
its exact hash. Corruption coverage and the full 443-test orchestration audit
pass, as does the 591-file source-size gate. The native 242-tick migration must
still succeed before coordinate descent begins.

Coordinate-descent heading result (2026-08-03): retained and exhausted. After
the 242-tick option route migrated by one exact terminal execution and two cold
replays, the first recomposed 16-heading pass evaluated 263 candidates; 247
reached the real load-zone terminal and four strict improvements each
cold-replayed twice. The incumbent improved through 240, 239, and finally 237
ticks, then stopped with zero heading candidates remaining. This confirms that
the edit families interact and that a one-pass-per-family conclusion was
premature. Continue cycle one from this exact heading checkpoint with duration
and roll refinement.

Coordinate-descent duration result (2026-08-03): retained and exhausted. The
duration pass consumed the exact 237-tick heading checkpoint, evaluated all
five same-family shortening candidates, and every candidate reached the real
terminal. Two strict improvements cold-replayed twice and reduced the incumbent
to 231 ticks; zero duration candidates remain. Continue cycle one with roll
promotion from this exact option checkpoint.

Coordinate-descent cycle-one result (2026-08-03): retained. Roll promotion
consumed the exact 231-tick duration checkpoint and exhausted all 37 candidates;
11 reached the real terminal and none improved, so the cycle-one incumbent
remains 231 ticks. Across the full cycle, recomposed headings improved 242 to
237 and duration improved 237 to 231. Because the cycle produced strict
improvements, begin another heading-duration-roll cycle from the exact
exhausted roll checkpoint; fixed point has not yet been established.

Coordinate-descent cycle-two result (2026-08-03): retained. The heading pass
evaluated 94 candidates, reached the real terminal on 86, and accepted one
twice-replayed improvement from 231 to 229 ticks before exhausting. Duration
then exhausted two terminal candidates without a winner, and roll exhausted 37
candidates, 11 terminal, without a winner. Because the cycle still improved,
run a third complete cycle from the exact exhausted roll checkpoint.

Coordinate-descent fixed-point result (2026-08-03): retained and complete at
229 ticks. Cycle three exhausted 88 heading candidates (81 terminal), two
duration candidates (both terminal), and 37 roll candidates (11 terminal)
without a strict winner. Every existing single-option family therefore reached
a full-cycle fixed point. Across the composed treatment the authenticated
incumbent improved from 242 to 229, but remains 105 ticks above 124. Diagnose
the exact 229-tick sequence before selecting one new generic edit family; do
not spend more samples on these exhausted frontiers.

Standalone-deletion checkpoint (2026-08-03): the shared local-option engine now
supports an exact one-option deletion mode in addition to duration and roll
replacement. It enumerates unique shortened sequences, persists the edit kind
and deterministic frontier in the existing compressed binary checkpoint, and
requires the same strict real-terminal improvement plus two exact cold replays.
Focused coverage proves duplicate collapse and one-option scope. The full 444-
test orchestration audit and 591-file source-size gate pass. Run it from the
authenticated 229-tick fixed point before considering the much larger
non-local-heading frontier.

Standalone-deletion result (2026-08-03): retained and exhausted. The pass
evaluated all 58 unique one-option removals; 40 still reached the real terminal
and two strict improvements cold-replayed twice. Removing action 29 improved
229 to 223 ticks, then removing action 27 improved 223 to 222 ticks. The final
selected tape hash is
`81c10aaa47f04740bc0413bc1934ab23e003a0863e5f37aef9224e61fd259685`.
Because structure changed the incumbent, recompute heading, duration, roll,
and deletion from the authenticated 222-tick checkpoint until a complete cycle
makes no improvement.

Post-deletion coordinate cycle, heading result (2026-08-03): retained and
exhausted. The regenerated coarse-heading frontier evaluated 101 candidates;
88 reached the real terminal and one strict winner cold-replayed twice,
improving 222 to 221 ticks. Continue this cycle with duration from the exact
heading checkpoint.

Post-deletion coordinate cycle, duration result (2026-08-03): exhausted with
no improvement. Both candidates reached the real terminal, but neither beat
221 ticks. Continue with roll refinement from the exact duration checkpoint.

Post-deletion coordinate cycle, roll result (2026-08-03): exhausted with no
improvement. Nine of 35 candidates reached the real terminal and none beat 221
ticks. Continue with deletion from the exact roll checkpoint.

Post-deletion coordinate cycle, deletion result (2026-08-03): exhausted with
no improvement. Twenty-two of 33 candidates reached the real terminal and none
beat 221 ticks. Because heading improved during this cycle, run one more full
heading, duration, roll, and deletion cycle from this exact checkpoint to prove
the new fixed point.

### 4. Prove learning value only after the route works

- [ ] Compare adaptive, frozen, and random-valid selection over the same seeds,
  action opportunities, and native budget.
- [ ] Require adaptive updates to improve terminals per sample, time to first
  terminal, or selected terminal ticks on held-out seeds.
- [ ] Run a separate human-demonstration ablation only after the zero-shot result
  exists. Human input may improve sample efficiency but cannot be required or
  cap the policy.

Exit: accumulated experience causally improves future terminal outcomes over
both controls.

### 5. Generalize only after Ordon passes

- [ ] Apply the unchanged contracts to a second native route.
- [ ] Add tactic mining and composition only if retained experience shows a
  repeated useful control structure; promote a tactic only on held-out gain.
- [ ] Split remaining mixed-responsibility code along execution, evidence,
  learning, persistence, and reporting boundaries; enforce source-size gates.
- [ ] Keep persistent artifacts binary, bounded, checksummed, atomic, and
  migration-tested.

Exit: the second route passes scratch discovery, learned improvement, and exact
cold replay without route-specific framework changes.
