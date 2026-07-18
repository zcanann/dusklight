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
- neutral input, a canonical input tape, or a compiled controller seed;
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

Validation fails closed on unknown JSON fields, invalid ordering or bounds,
zero identities, stale suite content identity, path traversal, symlink escape,
missing artifacts, or mismatched artifact hashes. It then validates the actual
bound formats:

- scenario JSON must be a valid `dusklight-scenario-fixture/v1`;
- milestone source must compile to the declared program digest and contain the
  declared goal;
- the observation view must validate, match its semantic digest, and name that
  same goal;
- a tape seed must decode and have the declared boot origin; and
- a controller seed must decode as a bounded `DUSKCTRL` program.

If a tape embeds a scenario fixture, it must equal the separately referenced
suite fixture. There is no ambiguous precedence between two fixture sources.
Successful validation emits a small machine-readable report with suite identity
and positive/negative case counts.

## Current boundary

The schema and validator exist, including unit and CLI integration coverage.
The checked-in stage-ready, reach-point, talk-to-NPC, and pick-up-object cases
do not exist yet, and `huntctl` does not yet execute an entire suite. Those are
separate active tasks so schema validation is not mistaken for conformance.
