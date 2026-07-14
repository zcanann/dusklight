# Milestone-backed route search

Route search is a finite-sample optimization loop over deterministic controller
programs. It does not use DDQN yet. With tens of clients, retaining proven
elites and making structured mutations spends samples on roll timing, headings,
segment lengths, and boot-menu timing instead of relearning controller basics.

C++ is the scoring authority: it reports the first simulation tick and complete
boundary fingerprint for each memory-backed milestone. Rust owns candidates,
compact tape compilation, native process scheduling, evidence, ranking, and
evolution. Python and PowerShell are not in the execution path.

## Candidate IR

A candidate uses schema dusklight-search-candidate/v1. Its typed macros compile
to DUSKTAPE, which remains replay authority:

    {
      "schema": "dusklight-search-candidate/v1",
      "segment": "fsp103_to_fsp104",
      "actions": [
        { "op": "neutral", "frames": 180 },
        { "op": "move", "angle_degrees": 0, "magnitude": 127, "frames": 30 },
        { "op": "roll", "angle_degrees": 4, "magnitude": 127, "recovery_frames": 12 }
      ],
      "ancestry": { "generation": 0 }
    }

Zero degrees is forward and positive 90 degrees is right. A roll presses B on
its first frame and holds its analog direction during recovery. Press supports
typed A, B, and Start pulses for boot-menu optimization. Neutral makes startup
and inter-input waits explicit and evolvable.

An existing absolute boot tape can be imported without hand-authoring JSON:

    huntctl search import-tape --segment boot_to_fsp103 --tape build/boot.tape --output build/boot.candidate.json

Boot import is lossless and deliberately narrow. It accepts neutral frames and
zero-stick typed A/B/Start pulses. The anchored tunnel profile also accepts an
absolute port-one movement tape: it run-length encodes the complete raw pad
state as `pad_run` actions, including analog samples and trigger values, and
verifies that compilation reproduces every source byte. Reactive waits and
noncanonical secondary-port state remain rejected.

The segment profiles are:

- boot_to_fsp103: process boot through restored control in F_SP103;
- fsp103_to_fsp104: direct F_SP103 start through entry into F_SP104;
- link_control_to_tunnel_crawl_start: an anchored suffix from the checked-in
  Link-control boundary to `crawl_start` in F_SP104 room 1 spawn 0.

## Anchored clean-boot suffix search

The tunnel objective uses the anchored library evaluator rather than the
legacy direct-stage evaluator. `AnchoredObjectiveConfig` supplies an immutable
absolute prefix tape, compiled DMSP, source milestone and boundary fingerprint,
and goal milestone. `AnchoredEvaluateConfig` and `AnchoredSearchRunConfig` are
the public wiring surfaces for the CLI and route workbench.

The promoted initial suffix is
`routes/intro/variants/link_control_to_tunnel_crawl_start/human-420.tape`. It is
421 frames and imports losslessly; this profile has no synthetic baseline, so
an anchored run fails configuration validation unless an observed suffix was
explicitly imported as its seed.

Every trial concatenates the same immutable prefix with one candidate suffix
and boots that complete tape in a clean process. It does not pass `--stage`.
The native run receives the compiled milestone program and exactly the source
and goal milestones. A result is accepted only when all of the following match:

- DMSP program and source/goal definition digests;
- the source milestone's final prefix frame, boundary index, and pinned
  boundary fingerprint;
- the goal evidence's F_SP104 room 1 spawn 0, Link identity, and procedure 53
  (`crawl_start`).

The content-derived objective digest covers the prefix bytes, DMSP bytes,
game executable and DVD SHA-256 identities, source proof, and goal. Anchored
mode rejects extra game arguments entirely, so stage, timing, and CVar changes
cannot escape that contract. The identity is stored beside the population and
in anchored results, preventing results from being reused after proof or
execution inputs change.
Ranking records goal time relative to the source boundary. The winner emits
both `champion.suffix.tape` for continuation work and a composed
`champion.tape` for clean-boot visual playback.

The route-aware command derives the prefix, source fingerprint, milestone
program, and observed seed from the checked-in timeline and lineage:

    huntctl search run-route --timeline routes/intro.timeline --lineage main --segment link_control_to_tunnel_crawl_start --game build/windows-clang-debug/dusklight.exe --dvd game.iso --output build/search/tunnel --generations 4 --size 16 --elites 4 --workers 8 --repetitions 3 --rng-seed 1

It refuses a timeline segment that is not immediately after the requested
lineage prefix. The compiled DMSP and materialized prefix are retained in the
sibling `build/search/tunnel.objective/` directory; attempt and champion
artifacts remain below the requested output root.

## Native evaluation

Both the game executable and disc image are explicit. There is no saved-config
fallback:

    huntctl search evaluate --population build/search/g000/manifest.json --game build/windows-clang-debug/dusklight.exe --dvd orig/GZ2E01/GZ2E01.iso --output build/search/g000/evaluations --results build/search/g000/results.json --workers 8 --repetitions 3 --timeout-seconds 300

Rust starts at most the requested number of isolated Dusklight processes. Every
attempt receives its own automation state, stdout, stderr, native milestone
result, boundary fingerprints, and attempt evidence. Timeouts kill the child.
Any launch failure, timeout, missing result, malformed schema, contradictory
milestone sequence, or evidence-write failure cancels the population and makes
the command fail. A legitimate goal miss remains a valid partial sample.

For the F_SP103 route, entry into F_SP104 proves the destination while the
earlier verified source-exit tick is the score. This keeps host loading latency
out of route golf. The full ready, exit, and entered boundary fingerprints stay
in attempt evidence for lineage compatibility decisions.

## Complete generation loop

The native command owns seed, evaluate, rank, evolve, and champion promotion:

    huntctl search run --segment fsp103_to_fsp104 --game build/windows-clang-debug/dusklight.exe --dvd orig/GZ2E01/GZ2E01.iso --output build/search/intro --generations 4 --size 16 --elites 4 --workers 8 --repetitions 3 --rng-seed 1

Each generation contains its manifest, candidates, compact tapes, isolated
attempt evidence, results, and leaderboard. The final root contains the exact
`champion.candidate.json`, its compiled `champion.tape`, and `run.summary.json`.
To continue mining from an existing candidate instead of restarting from the
built-in baseline, pass `--candidate FILE` to `search run`. The candidate is
validated and must match `--segment`.

For a successful boot tape, use the native reducer before spending samples on
more evolution:

    huntctl search minimize-boot --candidate build/dense.candidate.json --game build/windows-clang-debug/dusklight.exe --dvd orig/GZ2E01/GZ2E01.iso --output build/search/boot-minimized --workers 16 --repetitions 3

The reducer first proves the source against the corrected route-control oracle.
It neutralizes chunks of active button frames without shifting the timestamps
of surviving input, then removes individual pulse frames. A deletion is kept
only when every repetition produces identical milestone depth, goal outcome,
ticks, tape frames, and boundary fingerprints and still reaches the exact goal.
The source proof's goal simulation tick, tape frame, and boundary fingerprint
become an immutable reduction target; a deletion that succeeds later or reaches
a different boundary is rejected. Among exact-target equivalents, candidates
with fewer pulse frames win.

Finally, it truncates the tape to `goal tape_frame + 1` and proves that exact
artifact again. The output contains `minimized.candidate.json`,
`minimized.tape`, `proof.json`, and `minimize.summary.json`; intermediate ddmin
rounds remain under the output root for audit.

After reduction, golf the absolute timing of the surviving boot pulses without
changing their order:

    huntctl search golf-boot --candidate build/search/boot-minimized/minimized.candidate.json --game build/windows-clang-debug/dusklight.exe --dvd orig/GZ2E01/GZ2E01.iso --output build/search/boot-golfed --workers 16 --repetitions 3

This is exhaustive coordinate descent, not evolution or random sampling. Each
round tests every legal earlier absolute frame for every existing pulse,
starting with the final pulse. A candidate is eligible only when all repeated
runs agree exactly, it reaches the source proof's boundary fingerprint, and it
does not regress the current goal tick. Selection minimizes goal tick first,
then the sum and lexicographic vector of pulse timestamps. Consequently an
earlier same-tick move is retained: it may open space for an earlier neighboring
pulse and expose a faster pair on the next round. Golfing stops only when no
single coordinate has an eligible earlier move, then runs a separate exact
proof after truncating the winner to `goal tape_frame + 1`.

The output contains `golfed.candidate.json`, `golfed.tape`, `proof.json`, and
`golf.summary.json`. Every tested round remains below `rounds/`, including the
source proof, manifests, per-attempt evidence, and results. This proves a local
single-coordinate minimum for the fixed ordered pulse sequence; it does not
claim a global optimum across different buttons, added/deleted pulses, or
coordinated moves that require a temporarily later goal tick.

Both boot proof tools require at least two repetitions; `--repetitions 1` is
rejected rather than silently weakening determinism into a single observation.

Individual primitives remain available:

    huntctl search seed --segment fsp103_to_fsp104 --output build/search/g0 --size 16 --rng-seed 1
    huntctl search rank --population build/search/g0/manifest.json --results build/search/g0/results.json
    huntctl search evolve --population build/search/g0/manifest.json --results build/search/g0/results.json --output build/search/g1 --size 16 --elites 4 --rng-seed 2

Ranking is lexicographic: deepest verified milestone, first-hit tick, then
shorter tape. Repetitions are a hard determinism check, not a probabilistic
ranking dimension: identical trials must agree on milestone depth, goal
outcome, every hit's simulation tick and tape frame, and boundary fingerprints.
Any disagreement rejects the evaluation. Deterministic all-miss candidates are
valid evidence and remain below candidates that reach a milestone.

Current mutations adjust macro duration, analog heading and magnitude, insert
rolls, split/delete movement segments, and shrink explicit waits. Boot mutation
directly shifts and shrinks the neutral gaps attached to menu button presses;
it does not spend most samples perturbing only the initial boot wait. Candidate
Pad-run populations additionally perturb exact raw stick samples and toggle B
on selected runs, so importing a human tape does not reduce mining to duration
deletion alone. Candidate IDs hash segment plus input program, so identical
tapes deduplicate even if
separate search branches rediscover them; ancestry records the retained parent
and mutation for every generation.
