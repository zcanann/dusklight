# huntctl crate boundaries

These crates exist to make domain ownership a compiler rule. A folder name is
not an architectural boundary if any sibling module can import through it.

```text
dusklight-huntctl (CLI and domain orchestration)
├── dusklight-control ────────────────┐
├── dusklight-evidence ───────────────┐
├── dusklight-harness-contracts ──────┤
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
├── dusklight-routes ─────────────────┤
│   ├── dusklight-control ─────────────┤
│   ├── dusklight-objectives ──────────┤
│   └── dusklight-search ──────────────┤
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

## `dusklight-harness-contracts`

Owns conformance-objective suites, observation admission, and the authenticated
run request/result boundary. It may depend on contracts, control formats, and
objective compilation. It cannot launch native processes, schedule campaigns,
rank candidates, train models, or parse CLI commands. Root adapters execute
these contracts but cannot redefine them.

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
execution, route/workbench state, or CLI parsing. The root
`learning::q_search` module is intentionally a thin adapter: it is where search
candidates and learned proposal models are allowed to meet.

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

## `dusklight-search`

Owns portable search candidates, lexicographic ranking, mutation, typed local
refinement, and bounded continuous and Bayesian optimizers. It may depend on
contracts and control formats. It cannot execute native runs, inspect evidence,
train models, mutate route/workbench state, or parse CLI commands. Root adapters
are responsible for feeding it authenticated outcomes and enforcing simulator
budgets.

## `dusklight-routes`

Owns authored timeline syntax, route validation, immutable lineages, route
objects, and named-head persistence. It may depend on contracts, control
formats, objective compilation, and portable search candidates. It cannot run
the simulator, evaluate native evidence, own interactive workbench state,
train models, or parse CLI commands. The root workbench is therefore an adapter
over route truth instead of its owner.

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

Owns composition and the executable-facing adapters. Compatibility re-exports
preserve the existing public module paths while callers migrate; they do not
restore reverse dependencies into the smaller crates.

The next crate extractions should be driven by dependency direction, not file
size alone. Native harness execution should remain an adapter around the
extracted contracts until its process boundary is independently coherent. Do
not create a crate that depends back on `dusklight-huntctl`; orchestration
adapters belong in the root instead of weakening a lower-level crate.

The boundary test also freezes the root module inventory. Adding another root
file or module directory requires an explicit ownership-policy change, so new
domains cannot silently accumulate beside the orchestration adapters.
