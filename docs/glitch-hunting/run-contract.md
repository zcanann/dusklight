# Core harness run contract

`dusklight-harness-run-request/v2` and
`dusklight-harness-run-result/v2` are the canonical materialized boundary for
one harness attempt. They describe execution and evidence; they do not yet
mean that every legacy tape, search, or learning command executes through this
boundary.

## Request

A request content-binds the Dusklight executable, game data, complete build and
protocol identity, boot and scenario, objective program, observation view,
action schema, required query facts, input seed, RNG seed, logical-tick and host
budgets, fidelity mode, and artifact destination. The input seed uses the same
neutral, authored tape, or reactive-controller references as an objective-suite
case.

Observation dependencies use
`dusklight-objective-observation-requirements/v1`: a sorted list of exact fact
paths plus the minimum version of every required family. Suite validation
derives the facts directly from the selected milestone definition and rejects
both omitted and invented dependencies. Reward features and proposer scores do
not enter this derivation or the objective identity.

Protocol capabilities are a sorted versioned list with their own digest. This
makes an unsupported query fact different from a worker lacking an execution
capability. The complete artifact identity must agree with every explicit
scenario, predicate, action, observation, protocol, build, and game-data
binding in the request.

Authored request paths are canonical repository-relative paths. The game-data
path may be a repository-relative symlink to an ignored external disc image,
which supports the normal macOS checkout layout; validation still hashes the
resolved bytes and rejects a stale digest. Other authored inputs cannot escape
the repository root.

## Result

A result binds the exact request digest and attempt number, the worker's build
and protocol identity, a typed terminal reason, objective evidence, artifact
references, and logical/host timing. Terminal reasons distinguish reached,
exhausted, impossible, unsupported observation, capability or identity
mismatch, host timeout, cancellation, worker or game crash, protocol failure,
hang, target loss, nondeterminism, and rejected execution.

Workers expose a versioned observation inventory with present, absent,
not-sampled, unavailable, truncated, stale, or invalid status. Present and
semantic absence satisfy admission; a missing, old, unavailable, truncated,
stale, or invalid required family produces a typed `unsupported` result naming
the affected family and facts. It cannot become an objective miss or success.

`reached` is deliberately strict. It requires a first-hit tick, objective
evidence, boundary fingerprint, realized input tape, gameplay trace, objective
result, and a complete-artifacts marker. Failure results may retain
authenticated partial artifacts, but cannot carry partial success proof or be
reported as complete success. Artifact paths are resolved beneath the result's
declared artifact root and their exact bytes are verified.

## Commands

Drafts use an all-zero `content_sha256`. Sealing computes that identity,
validates every referenced file, writes a new file, and refuses to overwrite an
existing contract.

```text
huntctl harness seal-run-request --input DRAFT.json --output REQUEST.json \
  --repository-root DIR
huntctl harness validate-run-request --request REQUEST.json \
  --repository-root DIR

huntctl harness seal-run-result --input DRAFT.json --output RESULT.json \
  --request REQUEST.json --artifact-root DIR --repository-root DIR
huntctl harness validate-run-result --result RESULT.json \
  --request REQUEST.json --artifact-root DIR --repository-root DIR

huntctl harness execute --request REQUEST.json --repository-root DIR \
  [--attempt N]
```

The native executor currently routes neutral, compiled TAS-source, absolute
tape, and reactive-controller seeds through this boundary. It materializes or
records one absolute tape, launches an isolated native process, authenticates
the milestone result, realized tape, gameplay trace and observation inventory,
classifies the terminal, and seals `result.json` beneath the requested
destination. Exact controller target loss is distinct from input exhaustion.
Search and learned-proposal adapters remain open.

`harness inspect-objective` prints the full source objective, program and
definition identities, phase and stability, required families and facts,
current terminal progress, first-hit fingerprint, and missing evidence. With no
result it reports the run as pending; with `--result` it authenticates the
referenced artifacts before presenting them.

Unit coverage verifies identity disagreement, stale bytes, external game-data
symlinks, strict reached proof, partial crash artifacts, and distinct
unsupported/capability-mismatch results. CLI integration coverage seals and
validates a complete request/result pair, executes a tape request through a
mock native process, executes reactive control into a realized tape, proves
exact target loss and missing trace families remain typed failures, and verifies
overwrite refusal. A host-timeout integration case retains authenticated
stdout/stderr while refusing complete replay-proof status; crash-unit coverage
does the same for whatever partial artifacts exist before failure.
