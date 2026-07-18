# Fork-only observer boundary

Gameplay observation is an external instrument, not a gameplay feature. The
decompiled game remains the subject being measured. We do not edit its logic,
object layout, initialization, control flow, or state transitions to make a
query easier.

## Build boundary

Trace v2 reads live in `src/dusk/automation/gameplay_trace_observer.cpp`.
Milestone, reactive-controller, and Eye Shredder gameplay telemetry reads live
in `src/dusk/automation/game_state_observer.cpp`; actor-catalog reads remain in
the fork-owned `src/dusk/automation/actor_catalog.cpp`. These translation units
are guarded by `DUSK_ENABLE_AUTOMATION_OBSERVERS`; when the option is off they
do not include game headers and expose unavailable/no-op implementations. The
option is off by default. Glitch-hunting tools opt in explicitly while keeping
unrelated code mods off.

The old non-`const` event-name lookup has been removed. Milestone-boundary v2
records explicit hash absence and never invents zero as a present hash.

Integration hooks in `m_Do_main.cpp` pass automation-owned context at audited
pre-input/post-simulation boundaries. The main loop contains no field reads,
actor walks, collision queries, or game-specific interpretation. Name-entry
and file-select private-state producers remain in their declaring translation
units as known migration debt. Every current hook is visibly compiled behind
the observer gate, but the target architecture moves the capture implementation
to a narrow fork-owned friend/read adapter and leaves only a side-effect-free
sampling call at the native phase boundary.

Gameplay/decomp translation units are never acceptable query implementation
sites. An unavoidable sampling call uses the dedicated
`DUSK_ENABLE_AUTOMATION_OBSERVERS` preprocessor gate; a runtime conditional,
generic `TARGET_PC` block, or `IF_DUSK` branch is insufficient. With the gate
off, the native statements and control flow surrounding that block remain
unchanged.

Eye Shredder's original-memory consequence is not an observer. Its control-flow
and J2D color-channel mutation is separately compiled behind the default-off
`DUSK_ENABLE_AUTOMATION_FIDELITY_MODELS` option, which requires observers and
still requires the explicit runtime `--cursor-breakout-shadow` capability.
Search builds force this write-capable option off.

No `friend` declaration or private-access shim exists for Trace v2. Adding one
is an exception requiring an access-manifest entry, an explanation of why an
out-of-line public/const read is impossible, and observer-on/off replay parity.
It may never change object size, member order, virtual dispatch, initialization,
or gameplay control flow. A friend grants read access only: it does not make a
mutating helper acceptable and it must not add a convenience method to the
gameplay class.

## Initial access manifest

All entries are sampled after the completed simulation tick and before
presentation interpolation.

| Channel/fact | Native access | Side-effect audit | Portability |
|---|---|---|---|
| Stage/current and pending transition | Existing `dComIfGp_*` value accessors | Copies already-realized scalar/name state | Stage tuple portable; no pointers |
| Applied PAD, four ports | `JUTGamePad::mPadStatus` copied through the stable automation PAD codec | Direct fixed-size copy; no PAD write or clamp | Exact game-visible PAD ABI normalized to 12-byte wire records |
| Player motion | `dComIfGp_getPlayer(0)` held as `const fopAc_ac_c*`; direct transform/speed fields | Direct POD copies only | Process ID is explicitly session-local; actor/profile ID is build-relative |
| Link procedure/action | `const daAlink_c*`; public procedure/mode/raw context, const timer getters, six animation lanes | Direct fields and audited pure const getters | Procedure/animation IDs require build identity |
| Event control | `const dEvt_control_c*`; public state and const getters | Direct copies only | Event ID is session/build-relative |
| Event name | Not observed | `getRunEventName()` is logically read-only today but is a non-const gameplay API over private manager state, so Trace v2 does not call it | Event-name-hash presence flag remains false |
| RNG | `capture_game_rng_snapshot()` / `cM_getRndState()` | Fixed-size copies; tests prove neither stream advances | Snapshot and algorithm versions are explicit |
| Camera | `const camera_process_class*`; realized view POD and `mCamera.U2() const` | Direct view copies plus pure controlled-yaw getter; an invalid/unrealized view basis is `Unavailable` and its payload is not copied | Actual view yaw and controlled yaw are distinct fields |
| Nearest scene exit | `fopAcIt_Executor` with a callback that casts every candidate to `const`; direct actor fields | Bounded read-only traversal and arithmetic; no actor method call or allocation | Process ID is session-local; actor/profile ID is build-relative |
| Milestone boundary | Fixed copies of stage, pending stage, Link, event-control, and RNG facts in `game_state_observer.cpp` | Same direct fields/pure getters as Trace v2; event-name access is explicitly unavailable | Boundary fingerprint v2 authenticates presence as well as values |
| Reactive controller | Fixed Link/camera snapshot and bounded 256-actor direct-field copy | Pre-input read only; no allocation and deterministic lowest-process-ID retention | Process selector is session-local; placed selector includes stage/set/home-room |

## APIs observers must not call

- setters, non-const action helpers, or methods whose side effects have not been
  proved absent;
- lazy getters that initialize resources or fill caches;
- fresh collision/ray/ground queries merely to create an observation;
- normal collision-hit getters known to clear or update hit caches;
- allocation from a game heap;
- RNG generation or restoration;
- render/view setup that temporarily rewrites camera or presentation state;
- arbitrary memory-read scripting.

Future collision channels should copy already-resolved `mLinkAcch` contact and
polygon state. Future actor channels should copy bounded direct state through
typed adapters. Static geometry belongs in an offline world inventory rather
than a per-tick game query.

## Acceptance

Compilation is only the first gate. For every observer expansion, run the same
build/scenario/absolute tape with observation disabled and enabled. Canonical
game-state hashes, RNG snapshots/counters, events, terminal proof, and realized
input must match exactly. Any first difference is an observer/framework bug;
the affected capture configuration is quarantined until the cause is fixed.
