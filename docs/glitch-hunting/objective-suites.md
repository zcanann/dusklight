# Objective conformance suites

`dusklight-objective-suite/v2` is the authored contract for cheap end-to-end
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
- `dusklight-objective-observation-requirements/v1`, containing sorted exact
  fact paths and the minimum version of every required observation family;
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
  declared goal, whose derived fact dependencies must exactly match the
  declared observation requirements;
- the observation view must validate, match its semantic digest, and name that
  same goal;
- a tape seed must decode—or an authored TAS seed must compile—and have the
  declared boot origin; and
- a controller seed must decode as a bounded `DUSKCTRL` program.

If a tape embeds a scenario fixture, it must equal the separately referenced
suite fixture. There is no ambiguous precedence between two fixture sources.
Missing, old, unavailable, truncated, stale, or invalid required families are
unsupported execution, not a false objective. Reward features and proposer
scores are outside the derived objective dependencies and have no success
authority.

Successful validation emits a small machine-readable report with suite identity
and positive/negative case counts.

## Campaigns

Resolve one checked-in case before spending simulator budget:

```sh
cargo run --manifest-path tools/huntctl/Cargo.toml -- \
  campaign \
  --suite tests/fixtures/automation/objective_conformance_suite.json \
  --case reach-point-ordon-ranch \
  --output build/harness/reach-point-campaign \
  --proposer scripted \
  --proposer structured \
  --dry-run
```

The `dusklight-campaign-plan/v1` JSON resolves the suite, scenario, objective,
observation view, and seed paths; prints their bound identities; lists exact
required facts and capabilities; expands repetition and proposer budgets; and
shows the request, episode, finalist, replay, and report destinations. Campaign
outputs must be canonical repository-relative paths beneath ignored `build/`.
Dry runs validate every suite artifact but create no directories or files.

Execute the same plan by supplying one sealed run-request template and an
equal-budget proposer-tournament definition:

```sh
cargo run --manifest-path tools/huntctl/Cargo.toml -- \
  campaign \
  --suite tests/fixtures/automation/objective_conformance_suite.json \
  --case stage-ready-f-sp103 \
  --output build/harness/stage-ready-campaign \
  --run-request build/harness/run-request.json \
  --definition build/harness/tournament.json \
  --workers 4
```

The suite case replaces the template's boot, scenario, objective, observation,
action schema, seed, and budgets before the request is resealed. The tournament
definition selects the proposer lanes and their populations. The command ranks
the equal-budget native results, independently cold-replays every proved lane's
content-addressed finalist for the case repetition count, and writes
`dusklight-campaign-report/v1` to `report.json`. The report retains request and
tournament identities, charged budgets, objective hits, useful-boundary count,
replay verdicts, exact boundary proof, best proved tape, and the selected
winner. A missed expected terminal returns a failing exit status after writing
the diagnostic report. When a run is unsupported or reports an identity or
capability mismatch, `first_blocker` names the first exact fact, capability, or
terminal and links its authenticated harness-result artifact; the CLI repeats
that diagnostic and path on stderr.

## Shortest macOS operator loop

Use the dry run to inspect the exact case before spending budget, then execute
the campaign as above. The executing command performs the independent finalist
replays; they are not inferred from the tournament score. Inspect the compact
result and the winning replay with:

```sh
CAMPAIGN=build/harness/stage-ready-campaign
jq '{passed, winner_proposer, winner_tape, rows}' "$CAMPAIGN/report.json"

PROPOSER=$(jq -r .winner_proposer "$CAMPAIGN/report.json")
cargo run --manifest-path tools/huntctl/Cargo.toml -- \
  harness inspect-objective \
  --request "$CAMPAIGN/requests/$PROPOSER/replay-001.json" \
  --result "$CAMPAIGN/replays/$PROPOSER/attempt-001/result.json" \
  --artifact-root "$CAMPAIGN/replays/$PROPOSER/attempt-001" \
  --repository-root .
```

That inspection revalidates the retained request, result, objective evidence,
and artifact hashes. Keep a winner only by copying its reported tape into the
appropriate authored `routes/<route>/segments/` location, attaching its exact
proof through the Route Workbench, and committing those route files. To discard
it, reveal the ignored campaign directory with
`open -R "$CAMPAIGN/report.json"` and move that campaign directory to Trash in
Finder. Leaving a result under ignored `build/` is evidence retention, not
promotion; neither a report row nor a learner score edits an authored route.

## Current boundary

The schema, validator, sealer, campaign resolver, and executing proposer
campaign exist, including unit and CLI integration coverage.
`tests/fixtures/automation/objective_conformance_suite.json` contains a
fixture-bound direct `F_SP103` boot, a 30-tick authored tape whose compiled
frames are all neutral, a stable three-tick `stage_ready` objective, and a
minimal authenticated observation view. A wrong-stage neutral negative control
uses the same objective and budget and must end in `objective_miss`. The suite
validator authenticates and compiles all of those inputs.

The second case starts from an `F_SP104` point-0 fixture and binds a 799-tick
authored movement seed to the documented Ordon ranch region near
`(-1600, 200, -9050)`. Its semantic objective requires Link to remain inside a
bounded AABB for five post-simulation ticks, and its observation view retains
the stage, player identity, and exact position features used by that contract.
Its same-boot neutral negative control proves that stage readiness and spawn
proximity alone cannot satisfy the region objective.

The native query seam needed by the remaining interaction cases now exists.
Player-action channel v2 records the realized A-button status, exact placed
talk partner, and exact placed grabbed actor at the post-simulation boundary.
Milestone language 1.5 exposes those fields under
`player.interaction.do_status`, `player.interaction.talk_partner.*`, and
`player.interaction.grabbed_actor.*`. A talk objective can therefore require an
event edge and the correct partner identity; a carry objective can use an
ordered absent-to-present sequence and the correct object identity. Session
process IDs are retained for diagnostics but are not objective selector facts.

`objective_interaction_parity.{json,milestones,dmsp}` is the game-data-free
cross-runtime regression fixture for this surface. Rust recompiles the source,
requires byte-for-byte equality with the checked DMSP, decodes the fixture as a
normal multi-record `DUSKTRCE`, and evaluates it offline. The native milestone
test decodes that same DMSP and consumes the same JSON boundaries. Both must
report the exact first-hit boundaries for stage readiness, stable region
arrival, exact-NPC talk followed by event 17, and absent-to-exact-object carry.
Earlier wrong-stage, outside-region, wrong-NPC, and wrong-object boundaries
make the selector controls executable instead of implied.

The talk-to-NPC and pick-up-object positive/control pairs do not exist yet.
Those remain separate active tasks so an executing campaign is not mistaken
for native conformance evidence that has not actually been authored and run.
