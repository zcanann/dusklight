# Native tactic launch smoke

The supported-platform launch smoke is a deliberately tiny native campaign,
not a policy-quality benchmark. It proves that a current sealed executable can:

1. advertise persistent and compact suffix-batch capabilities;
2. authenticate the requested root boundary and root checkpoint;
3. accept the execution-plan-derived checkpoint-cache capacity;
4. execute one compact, cached native tactic proposal;
5. retain complete native timing and headless-audit telemetry;
6. commit exactly one graph lease with no unresolved work; and
7. pass the campaign resource audit.

First materialize an execution from the platform's current native build:

```text
huntctl campaign materialize-native-residual-execution \
  --request OPTIMIZATION.json \
  --game DUSKLIGHT \
  --dvd GAME.iso \
  --world-context WORLD.json \
  --output build/native-launch-smoke/execution
```

Then run the fixed one-worker, one-decision, one-proposal cell and publish its
portable evidence:

```text
huntctl learn run-tactic-launch-smoke \
  --request OPTIMIZATION.json \
  --execution build/native-launch-smoke/execution/execution.json \
  --output build/native-launch-smoke/run \
  --bundle benchmarks/native-tactic-launch-smoke/PLATFORM-ARCH \
  --seed 155921
```

The command fixes the topology and native-controller execution strategy.
`--memory-bytes` and `--wall-micros` may override their bounded defaults.
`--seed` is mandatory so platforms execute the same sealed cell.

The bundle carries the optimization request, execution binding, binary
execution plan, route report, resource audit, native worker build/capability
hello, initial cached-root request/result, compact proposal envelope/result,
binary lease journal, graph checkpoint snapshot, and all small source
authorities. Executable, runtime-library, and game-image bytes are identified
by digest rather than copied into Git.

Validate a copied bundle without its originating build directory:

```text
huntctl learn validate-tactic-launch-smoke \
  --bundle benchmarks/native-tactic-launch-smoke/PLATFORM-ARCH
```

If a native run completed but evidence publication was interrupted, seal its
existing report with `seal-tactic-launch-smoke`. The validator fails on stale
or absent compact-worker capabilities, detached execution identities, invalid
binary plans, missing compact transport, incomplete native result telemetry,
cache-capacity drift, graph/checkpoint drift, lease loss or duplication, and a
failed resource audit.

Every committed launch-smoke manifest is rediscovered and validated by:

```text
python ci/source_quality/audit_learning_framework.py
```

Windows and macOS evidence is required for the same optimization request and
seed. A single-platform result keeps the P0 task open.
