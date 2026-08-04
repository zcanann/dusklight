# Native tactic throughput evidence

The v4 fixed-work curve is a long-work diagnostic, not a warm-process
microbenchmark. Every one- and two-worker cell must:

- execute at least 16 graph decisions with 2 proposals per decision;
- publish and consume fitted learner work;
- grow durable replay and admit graph evidence;
- perform repeated non-root restoration;
- persist evidence and exercise bounded checkpoint eviction; and
- report useful expansions, phase occupancy, memory, staleness, and native
  worker saturation.

At least two balanced repetitions are required. Odd repetitions run worker
counts in ascending order and even repetitions run them in descending order.
Every sample must realize the same content-identified useful expansion set.

Run and seal the curve together:

```text
huntctl learn tactic-throughput-curve \
  --request REQUEST.json \
  --execution EXECUTION.json \
  --output build/benchmarks/throughput-v4 \
  --seed SEED \
  --decisions-per-seed 16 \
  --proposals-per-decision 2 \
  --memory-bytes BYTES \
  --repetitions 2 \
  --bundle benchmarks/native-tactic-throughput/PLATFORM
```

`--memory-bytes` is the aggregate checkpoint-pool bound at the widest
two-worker cell, not a per-worker allowance. It must provide every fleet member
enough space to retain at least one measured native checkpoint. Fleet launch
measures the root checkpoint independently on every worker, requires identical
nonzero sizes, and rejects an undersized cache before beginning sample 1.

For the retained Windows execution on 2026-08-03, one checkpoint was
`294,721,440` bytes. The two-worker curve uses `1,342,177,280` bytes
(1.25 GiB), or `671,088,640` bytes per worker, enough for the bounded two-entry
process-local cache.

If the command is interrupted, repeat the exact invocation with `--resume`.
The output contains an authenticated binary run commit, append-only fleet
launch records, and one immutable commit per completed sample. Resume audits
and skips the contiguous completed prefix, resumes the first partial route
sample from its own durable campaign state, and rejects gaps, changed inputs,
or a committed sample whose report no longer matches. If all samples finished
but aggregate sealing was interrupted, resume atomically publishes the
aggregate without launching native workers again.
Interrupted outputs created before the run-commit protocol are rejected rather
than adopted without an authenticated identity.

An existing curve can be sealed later:

```text
huntctl learn seal-tactic-throughput-curve \
  --request REQUEST.json \
  --execution EXECUTION.json \
  --report build/benchmarks/throughput-v4/throughput-curve.json \
  --bundle benchmarks/native-tactic-throughput/PLATFORM
```

Validate without the originating build directory:

```text
huntctl learn validate-tactic-throughput-curve-bundle \
  --bundle benchmarks/native-tactic-throughput/PLATFORM
```

The portable bundle retains the request, execution binding, binary execution
plan, aggregate curve, six source authorities, every complete route report,
and a recomputed campaign/resource audit for every sample. Large reports are
individually zstd-compressed. The manifest binds both the stored compressed
blob and the original document identity and byte length.

Validation reconstructs every report, recomputes its aggregate sample fields,
checks the route/request/execution/plan identities, and validates each resource
audit. A curve may validly report saturation and therefore have `passed:
false`; that is retained diagnostic evidence, not a corrupt bundle. The
clean-checkout learning audit discovers and validates every committed
throughput bundle.
