# Matched native scratch-ranking comparison

This diagnostic isolates action-ranking policy. The learned,
scheduler-only-coverage, and random-valid cells must use the same optimization
request, native execution binding, seeds, workers, proposal width, decision and
tick horizons, wall and memory limits, branch/refit cadence, action schema,
feature schema, checkpoint policy, and graph-node acquisition schedule.

Run the three cells with identical arguments except `--proposal-policy` and
`--output`. Do not provide a demonstration or promoted tactic registry.

```text
huntctl learn tactic-route \
  --request OPTIMIZATION.json \
  --execution EXECUTION.json \
  --output build/scratch-learned \
  --proposal-policy learned \
  --workers 8 \
  --decisions-per-seed 256 \
  --proposals-per-decision 4 \
  --branch-every 8 \
  --refit-every 4 \
  --memory-bytes 8589934592 \
  --wall-micros 900000000

huntctl learn tactic-route \
  --request OPTIMIZATION.json \
  --execution EXECUTION.json \
  --output build/scratch-scheduler \
  --proposal-policy structured-non-learning \
  --workers 8 \
  --decisions-per-seed 256 \
  --proposals-per-decision 4 \
  --branch-every 8 \
  --refit-every 4 \
  --memory-bytes 8589934592 \
  --wall-micros 900000000

huntctl learn tactic-route \
  --request OPTIMIZATION.json \
  --execution EXECUTION.json \
  --output build/scratch-random \
  --proposal-policy random-valid \
  --workers 8 \
  --decisions-per-seed 256 \
  --proposals-per-decision 4 \
  --branch-every 8 \
  --refit-every 4 \
  --memory-bytes 8589934592 \
  --wall-micros 900000000
```

The comparator fails closed if any sealed condition differs outside the
ranking treatment:

```text
huntctl learn compare-tactic-scratch-campaigns \
  --learned-report build/scratch-learned/report.json \
  --scheduler-report build/scratch-scheduler/report.json \
  --random-report build/scratch-random/report.json \
  --output build/scratch-ranking-comparison.json \
  --repository-root .
```

The report publishes terminal rate; median wall time, proposal evaluations,
and unique graph-authoritative expansions to first and best terminal; useful
expansions per second; simulated ticks per expansion; native-worker occupancy;
unattributed coordinator time; restore/fallback accounting; and native, IPC,
observation, encoding, extraction, admission, learner, persistence, and
orchestration phase costs.

`sample_efficiency_timeline_complete` and
`terminal_improvement_timing_complete` must both be true before drawing a
learning conclusion. Legacy reports remain inspectable but cannot silently
stand in for missing per-decision evidence.
