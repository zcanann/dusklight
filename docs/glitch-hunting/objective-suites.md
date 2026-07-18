# Objective conformance suites

`dusklight-objective-suite/v1` is the authored contract for cheap end-to-end
harness cases. It describes what must be run and proved; it is not a run result,
search definition, model configuration, or promotion artifact.

Each suite is content-addressed and contains one or more cases sorted by stable
ID. A case binds:

- positive or negative-control role, with every negative control naming its
  positive case;
- process boot or exact stage, room, point, layer, and optional save slot;
- a repository-relative scenario-fixture JSON artifact and SHA-256;
- a milestone source artifact, raw SHA-256, compiled program SHA-256, and exact
  goal name;
- an observation-view JSON artifact, raw SHA-256, and semantic schema SHA-256;
- action-schema name and digest;
- sorted required query-fact names;
- neutral input, an authored TAS program, a canonical input tape, or a compiled
  controller seed;
- logical-tick budget, host safety timeout, and repetition count; and
- the expected reached, objective-miss, unsupported, or impossible class.

Every case requires at least two repetitions. Positive cases cannot expect an
objective miss; negative controls must expect one. This makes the ordinary
false case part of the authored contract instead of an informal test note.

## Validation

Run:

```sh
cargo run --manifest-path tools/huntctl/Cargo.toml -- \
  harness validate-suite \
  --suite path/to/suite.json \
  --repository-root .
```

Authors can seal a zero-identity draft only after every referenced artifact
validates:

```sh
cargo run --manifest-path tools/huntctl/Cargo.toml -- \
  harness seal-suite \
  --input path/to/suite.draft.json \
  --output path/to/suite.json \
  --repository-root .
```

Sealing refuses to overwrite an existing output.

Validation fails closed on unknown JSON fields, invalid ordering or bounds,
zero identities, stale suite content identity, path traversal, symlink escape,
missing artifacts, or mismatched artifact hashes. It then validates the actual
bound formats:

- scenario JSON must be a valid `dusklight-scenario-fixture/v1`;
- milestone source must compile to the declared program digest and contain the
  declared goal;
- the observation view must validate, match its semantic digest, and name that
  same goal;
- a tape seed must decode—or an authored TAS seed must compile—and have the
  declared boot origin; and
- a controller seed must decode as a bounded `DUSKCTRL` program.

If a tape embeds a scenario fixture, it must equal the separately referenced
suite fixture. There is no ambiguous precedence between two fixture sources.
Successful validation emits a small machine-readable report with suite identity
and positive/negative case counts.

## Current boundary

The schema, validator, and sealer exist, including unit and CLI integration
coverage. `tests/fixtures/automation/objective_conformance_suite.json` contains
the first case: a fixture-bound direct `F_SP103` boot, a 30-tick authored tape
whose compiled frames are all neutral, a stable three-tick `stage_ready`
objective, and a minimal authenticated observation view. The suite validator
authenticates and compiles all of those inputs.

The second case starts from an `F_SP104` point-0 fixture and binds a 799-tick
authored movement seed to the documented Ordon ranch region near
`(-1600, 200, -9050)`. Its semantic objective requires Link to remain inside a
bounded AABB for five post-simulation ticks, and its observation view retains
the stage, player identity, and exact position features used by that contract.

The talk-to-NPC, negative-control, and pick-up-object cases do not exist yet,
and `huntctl` does not yet execute an entire suite. Those are separate active
tasks so authored-contract validation is not mistaken for native conformance
evidence.
