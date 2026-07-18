# Core harness run contract

`dusklight-harness-run-request/v1` and
`dusklight-harness-run-result/v1` are the canonical materialized boundary for
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
```

Unit coverage verifies identity disagreement, stale bytes, external game-data
symlinks, strict reached proof, partial crash artifacts, and distinct
unsupported/capability-mismatch results. CLI integration coverage seals and
validates a complete request/result pair and verifies overwrite refusal.
