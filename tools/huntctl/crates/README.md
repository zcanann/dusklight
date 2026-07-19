# huntctl crate boundaries

These crates exist to make domain ownership a compiler rule. A folder name is
not an architectural boundary if any sibling module can import through it.

The root package is only the executable adapter and compatibility facade.
Command implementations are grouped below `src/cli/` by domain; reusable
behavior belongs in one of the crates below. The crate-boundary test also
ratchets the root adapter and each CLI module by line count, so a large command
cannot quietly turn `main.rs` or a replacement file back into a flat dumping
ground. Crate entry points and non-test implementation modules have separate
ceilings as well; large test suites live in sibling `tests.rs` modules instead
of obscuring production ownership. The budgets are ceilings to lower during
extraction, not targets. The current policy caps the executable adapter and
crate entry points at 2,500 lines and every non-test implementation module at
3,000 lines. Bounded search, evaluation, finalist reduction, harness runtime,
orchestration, proposal, proposer tournament, and workbench source inventories
are closed lists: adding a sibling module requires an explicit ownership-policy
change.

```text
dusklight-huntctl (CLI and domain orchestration)
├── dusklight-bounded-search ─────────┤
│   ├── dusklight-evaluation ──────────┤
│   ├── dusklight-learning ────────────┤
│   └── dusklight-search ──────────────┤
├── dusklight-control ────────────────┐
├── dusklight-evidence ───────────────┐
├── dusklight-evaluation-plan ────────┤
├── dusklight-evaluation ─────────────┤
│   ├── dusklight-evaluation-plan ────┤
│   ├── dusklight-harness-runtime ────┤
│   ├── dusklight-proposals ──────────┤
│   └── dusklight-search ─────────────┤
├── dusklight-finalist-reduction ─────┤
│   ├── dusklight-control ─────────────┤
│   ├── dusklight-evaluation ──────────┤
│   └── dusklight-search ──────────────┤
├── dusklight-harness-contracts ──────┤
├── dusklight-harness-runtime ────────┤
│   ├── dusklight-harness-contracts ──┤
│   ├── dusklight-objectives ─────────┤
│   └── dusklight-trace ──────────────┤
├── dusklight-interventions ──────────┤
├── dusklight-learning ───────────────┤
│   ├── dusklight-control ────────────┤
│   ├── dusklight-evidence ───────────┤
│   ├── dusklight-objectives ─────────┤
│   ├── dusklight-trace ──────────────┤
│   └── dusklight-world ──────────────┤
├── dusklight-objectives ─────────────┤
│   └── dusklight-trace ──────────────┤
├── dusklight-oracles ────────────────┤
│   └── dusklight-trace ──────────────┤
├── dusklight-orchestration ──────────┤
│   ├── dusklight-bounded-search ──────┤
│   ├── dusklight-control ─────────────┤
│   ├── dusklight-evaluation ──────────┤
│   ├── dusklight-finalist-reduction ──┤
│   ├── dusklight-harness-runtime ─────┤
│   ├── dusklight-learning ─────────────┤
│   ├── dusklight-proposer-tournament ─┤
│   └── dusklight-search ──────────────┤
├── dusklight-proposals ──────────────┤
│   ├── dusklight-evidence ───────────┤
│   ├── dusklight-learning ───────────┤
│   └── dusklight-search ─────────────┤
├── dusklight-proposer-tournament ────┤
│   ├── dusklight-evaluation ──────────┤
│   ├── dusklight-harness-contracts ───┤
│   ├── dusklight-learning ─────────────┤
│   └── dusklight-search ──────────────┤
├── dusklight-routes ─────────────────┤
│   ├── dusklight-control ─────────────┤
│   ├── dusklight-objectives ──────────┤
│   └── dusklight-search ──────────────┤
├── dusklight-route-workbench ─────────┤
│   ├── dusklight-evidence ─────────────┤
│   ├── dusklight-harness-contracts ────┤
│   └── dusklight-routes ───────────────┤
├── dusklight-search ─────────────────┤
│   └── dusklight-control ─────────────┤
├── dusklight-semantic-novelty ───────┤
│   └── dusklight-trace ──────────────┤
├── dusklight-worker-protocol ────────┤
├── dusklight-world ──────────────────┤
└── dusklight-automation-contracts ◄──┘
```

## `dusklight-automation-contracts`

Owns portable value contracts: artifact/build identity, exact actor identity,
candidate/proposer envelopes, compatibility modes, observation schemas,
scenario fixtures, and DUSKTAPE. It has no filesystem orchestration, process
control, search, learning, route, workbench, or native-runtime dependency.

## `dusklight-evidence`

Owns immutable evidence and storage: content-addressed blobs, recorded tape
corpora, transition corpora, exact phase/provenance joins, episode manifests,
and repetition ledgers. It may depend on contracts, control formats, and trace
decoding. It cannot depend on proposers, learners, search ranking, route
mutation, worker processes, or the CLI. This prevents evidence truth from
acquiring algorithm-specific authority. Exact trace/evidence comparison also
lives here because it consumes only immutable artifacts and has no execution or
proposal authority.

## `dusklight-bounded-search`

Owns ordinary evolutionary, beam, continuous CEM/CMA-ES, and Bayesian driver
loops whose rankings come exclusively from authenticated native evaluation. It
may consume portable contracts, evaluation, learned Q priors, and pure search
algorithms. It cannot own campaigns, anchored-generation learning admission,
novelty archives, finalist reduction, route/workbench state, harness runtime
internals, or CLI parsing.

## `dusklight-evaluation`

Owns authenticated population evaluation, anchored generation evaluation,
trial evidence extraction, native-result admission, and explicit learned-
proposal fact/holdout gates. It may execute already-materialized candidates
through the harness runtime and consume proposal policies, but it cannot define
harness request truth, train models, author proposal or tournament policy,
schedule general optimizer loops or top-level objective campaigns, or choose
how finalists are reduced. A prepared anchored evaluator exposes repeated proof
against one authenticated objective without granting orchestration access to
evaluation internals.

## `dusklight-evaluation-plan`

Owns the deterministic prelaunch mapping from declared trial identities to
worker lanes, its portable JSON contract and digest, and validation of completed
worker claims. It may depend only on portable automation contracts. It cannot
execute trials, interpret objectives, rank candidates, schedule optimizer loops,
or parse CLI commands.

## `dusklight-finalist-reduction`

Owns exact bounded boot and anchored-route finalist reduction, checkpointed
recovery, and independent final proof. It may consume contracts, control tape
composition, authenticated evaluation, and portable search candidates. It
cannot own campaigns, novelty archives, learning policy, general optimizer
loops, harness runtime internals, route/workbench state, or CLI parsing.

## `dusklight-harness-contracts`

Owns conformance-objective suites, observation admission, and the authenticated
run request/result boundary. It may depend on contracts, control formats, and
objective compilation. It cannot launch native processes, schedule campaigns,
rank candidates, train models, or parse CLI commands. Root adapters execute
these contracts but cannot redefine them.

## `dusklight-harness-runtime`

Owns native process launch, isolated artifact capture, result sealing, and
human-readable inspection for authenticated harness requests. It may consume
contracts, controller input, objective compilation, and trace decoding. It
cannot schedule campaigns, rank candidates, propose actions, train models, or
parse CLI commands. Native execution therefore remains usable without granting
the evaluator or campaign scheduler ownership of the process boundary.

## `dusklight-interventions`

Owns the typed intervention timeline/DSL, bounded parameter search, explicitly
gated runtime write-audit contract, and control/treatment evidence formats. It
may depend only on portable automation contracts. It cannot execute the game,
rank candidates, inspect route state, train models, or parse CLI commands. The
experimental runtime remains disabled unless the root forwards the matching
feature explicitly.

## `dusklight-control`

Owns tape authoring/editing/composition, static controller compilation, typed
option evidence, the reusable tactic catalog, and bounded roll/path/tactic
realization. It may depend on contracts. It cannot depend on objective truth,
evidence, search, learning, routes, workers, native process execution, or CLI
parsing.

## `dusklight-learning`

Owns immutable dataset construction, deterministic learner implementations,
model identity and lineage, calibration, advisory action guidance, readiness
gates, and bounded model artifacts. It may consume contracts, control
descriptions, immutable evidence, objectives, traces, and read-only world
representations. It cannot depend on search candidates or ranking, native
execution, route/workbench state, or CLI parsing. Learned models remain
advisory values until the proposal-policy crate explicitly turns them into
ordinary candidates.

## `dusklight-worker-protocol`

Owns framed and NDJSON worker protocols, transports, the checked client, and
the local worker pool. It may depend on contracts. It cannot depend on search,
learning, evidence storage, routes, workbench code, or CLI parsing.

## `dusklight-trace`

Owns the versioned gameplay-trace wire decoder and the lossless projection from
a decoded trace boundary into shared typed facts. It may depend on contracts.
It cannot depend on objective semantics, search, learning, harness execution,
routes, workbench code, or CLI parsing.

## `dusklight-objectives`

Owns the bounded milestone language, bytecode, and offline evaluation against
decoded traces. It may depend on contracts and trace. It cannot execute the
game, select candidates, mutate routes, train models, or parse CLI commands.
This direction is deliberate: observations do not acquire objective semantics,
while objectives may interpret observations.

## `dusklight-oracles`

Owns run-local semantic oracles, cross-run comparison oracles, and the pipeline
that composes their evidence. It may depend on contracts and trace. It cannot
launch the game, rank or mutate candidates, train models, own route state, or
parse CLI commands. Oracle verdicts therefore remain independent of the search
systems that consume them.

## `dusklight-orchestration`

Owns top-level objective campaign planning, conformance scheduling, cold
replay, final reporting, and the anchored generation/learning-admission loop.
It composes bounded-search, proposer-tournament, and finalist-reduction crates
with the authenticated evaluator; those lower policy crates cannot depend back
on orchestration. Nothing below orchestration may depend on it or the huntctl
executable. It is not a general-purpose home for native execution, evaluation
truth, learner implementation, optimizer loops, tournament policy, finalist
reduction, or proposal code.

## `dusklight-proposals`

Owns learned and heuristic candidate proposal policies plus the bounded
behavior archive used to preserve diverse evidence. It is the only lower-level
crate allowed to combine learned model outputs, immutable episode evidence,
and portable search candidates. It cannot execute candidates, score objective
truth, schedule campaigns, or parse CLI commands; every proposal must pass
through the ordinary evaluator afterward.

## `dusklight-proposer-tournament`

Owns equal-budget proposer selection, authenticated envelope admission,
deduplicated shared evaluation, budget accounting, replay comparison, and
proved-finalist publication. It may consume contracts, evaluation, harness
result contracts, action-schema identity, and portable search candidates. It
cannot own objective-suite campaigns, novelty archives, learning policy,
routes, workbench state, harness runtime internals, or CLI parsing.

## `dusklight-search`

Owns portable search candidates, lexicographic ranking, mutation, typed local
refinement, and bounded continuous and Bayesian optimizers. It may depend on
contracts and control formats. It cannot execute native runs, inspect evidence,
train models, mutate route/workbench state, or parse CLI commands. The
orchestration crate feeds it authenticated outcomes and enforces simulator
budgets.

## `dusklight-routes`

Owns authored timeline syntax, route validation, immutable lineages, route
objects, and named-head persistence. It may depend on contracts, control
formats, objective compilation, and portable search candidates. It cannot run
the simulator, evaluate native evidence, own interactive workbench state,
train models or parse CLI commands.

## `dusklight-route-workbench`

Owns interactive graph projection, draft editing, playback, recording,
thumbnail storage, and the local HTTP surface. It may compose route, control,
evidence, objective, search, and evaluation identity contracts, but it cannot
depend on the huntctl executable or native search orchestration. This keeps the
interactive application behind a compiler-enforced boundary instead of
letting it grow as another root module.

## `dusklight-semantic-novelty`

Owns portable semantic descriptors, novelty catalogs, discovery archives,
symptom clustering, minimization predicates, and headful/human review handoff.
It may depend on contracts and decoded traces. It cannot execute candidates,
rank native search results, mutate routes, train models, or parse CLI commands.
This keeps discovery semantics inspectable and independent of the evaluator
that consumes them.

## `dusklight-world`

Owns read-only archive/collision geometry, static world inventory, and bounded
spatial queries. It may depend on contracts. It cannot control the game or
depend on search, learning, evidence, route, workbench, or CLI code.

## Root `dusklight-huntctl`

Owns CLI parsing and the few command-specific adapters that remain.
Compatibility re-exports preserve existing public module paths while callers
migrate; they do not restore reverse dependencies into the smaller crates.

The next crate extractions should be driven by dependency direction, not file
size alone. Do not create a crate that depends back on `dusklight-huntctl`;
executable-facing composition belongs in `dusklight-orchestration` instead of
weakening a lower-level crate.

The boundary test freezes both the root module inventory and the inventories of
the integration-heavy crates. It also freezes the objective language's private
syntax/validation, formatting, compilation, codec, and recorded-trace modules
with tighter file budgets. Adding another root file, root module directory,
coordination sibling, or objective-language owner requires an explicit
ownership-policy change, so new domains cannot silently accumulate beside the
adapters.
