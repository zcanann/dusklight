# huntctl crate boundaries

These crates exist to make domain ownership a compiler rule. A folder name is
not an architectural boundary if any sibling module can import through it.

```text
dusklight-huntctl (CLI and domain orchestration)
├── dusklight-evidence ───────────────┐
├── dusklight-worker-protocol ────────┤
├── dusklight-world ──────────────────┤
└── dusklight-automation-contracts ◄──┘
```

## `dusklight-automation-contracts`

Owns portable value contracts: artifact/build identity, exact actor identity,
compatibility modes, observation schemas, scenario fixtures, and DUSKTAPE.
It has no filesystem orchestration, process control, search, learning, route,
workbench, or native-runtime dependency.

## `dusklight-evidence`

Owns immutable evidence and storage: content-addressed blobs, recorded tape
corpora, transition corpora, episode manifests, and repetition ledgers. It may
depend on contracts. It cannot depend on proposers, learners, search ranking,
route mutation, worker processes, or the CLI. This prevents evidence truth from
acquiring algorithm-specific authority.

## `dusklight-worker-protocol`

Owns framed and NDJSON worker protocols, transports, the checked client, and
the local worker pool. It may depend on contracts. It cannot depend on search,
learning, evidence storage, routes, workbench code, or CLI parsing.

## `dusklight-world`

Owns read-only archive/collision geometry, static world inventory, and bounded
spatial queries. It may depend on contracts. It cannot control the game or
depend on search, learning, evidence, route, workbench, or CLI code.

## Root `dusklight-huntctl`

Owns composition and the executable-facing adapters. Compatibility re-exports
preserve the existing public module paths while callers migrate; they do not
restore reverse dependencies into the smaller crates.

The next crate extractions should be driven by dependency direction, not file
size alone. In particular, the current search/learning cycle must be broken by
giving candidate and proposal envelopes a lower-level owner before either
domain becomes a crate. Native harness contracts and native process execution
should also separate before extracting the harness domain. Do not create a
crate that depends back on `dusklight-huntctl`.
