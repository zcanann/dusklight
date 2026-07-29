# Learning framework clean-checkout audit

Run the complete local quality and retained-evidence gate from the repository
root:

```text
python ci/source_quality/audit_learning_framework.py
```

The command checks:

- the production Rust physical-line limit and debt-exception ledger;
- repository Rust formatting;
- the complete `huntctl` workspace build;
- every `dusklight-orchestration` unit and documentation test; and
- every tracked native scratch evidence bundle whose manifest uses
  `dusklight-native-tactic-scratch-evidence-bundle/v1`.

Scratch discovery validation publishes a movable bundle with:

```text
huntctl learn validate-tactic-scratch-discovery \
  --request OPTIMIZATION.json \
  --execution EXECUTION.json \
  --report ROUTE-REPORT.json \
  --output ACCEPTANCE.json \
  --bundle BUNDLE-DIR \
  --repository-root REPOSITORY
```

The bundle retains content-addressed request, execution, plan, route, seed,
checkpoint, terminal tape/result, and small source-authority artifacts. It can
be copied away from its originating build tree and validated independently:

```text
huntctl learn validate-tactic-scratch-bundle --bundle BUNDLE-DIR
```

Executable and game-image bytes are deliberately not duplicated into the
bundle. Their exact SHA-256 identities, runtime dependency identities, native
source boundary, objective, process tape, milestone program, world context,
and card-fixture manifest remain sealed in the retained execution and request
authorities.
