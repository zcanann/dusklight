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

Import is lossless and deliberately narrow. It accepts neutral frames and
zero-stick typed A/B/Start pulses. It rejects reactive waits, analog movement,
secondary-port state, unusual pad state, and anything else whose intent would
be ambiguous.

The segment profiles are:

- boot_to_fsp103: process boot through restored control in F_SP103;
- fsp103_to_fsp104: direct F_SP103 start through entry into F_SP104.

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
attempt evidence, results, and leaderboard. The final root contains
champion.tape and run.summary.json.

Individual primitives remain available:

    huntctl search seed --segment fsp103_to_fsp104 --output build/search/g0 --size 16 --rng-seed 1
    huntctl search rank --population build/search/g0/manifest.json --results build/search/g0/results.json
    huntctl search evolve --population build/search/g0/manifest.json --results build/search/g0/results.json --output build/search/g1 --size 16 --elites 4 --rng-seed 2

Ranking is lexicographic: deepest verified milestone, success rate across
repeated isolated trials, median first-hit tick, best first-hit tick, then
shorter tape. A candidate that merely approaches an exit cannot outrank one
which activates the exact F_SP103 to F_SP104 transition.

Current mutations adjust macro duration, analog heading and magnitude, insert
rolls, split/delete movement segments, and shrink explicit waits. Candidate IDs
hash segment plus input program, so identical tapes deduplicate even if separate
search branches rediscover them; ancestry records the retained parent and
mutation for every generation.
