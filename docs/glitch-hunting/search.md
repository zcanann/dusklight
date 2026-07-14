# Milestone-backed route search

Route search is a finite-sample optimization loop over deterministic controller
programs. It does not use DDQN yet. With only tens of clients, retaining proven
elites and making structured mutations spends samples on roll timing, headings,
segment lengths, and boot-menu timing instead of relearning controller basics.

C++ is the scoring authority: it reports the first simulation tick at which a
memory-backed milestone is hit. Rust owns candidate generation, compact tape
compilation, ancestry, ranking, and evolution.

## Candidate IR

A candidate uses schema dusklight-search-candidate/v1. Its typed macros compile
to an ordinary DUSKTAPE, which remains the replay authority:

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

The segment profiles are:

- boot_to_fsp103: process boot through restored control in F_SP103;
- fsp103_to_fsp104: direct F_SP103 start through entry into F_SP104.

## File-oriented generation loop

Generate a reproducible initial population:

    cargo run --release --manifest-path tools/huntctl/Cargo.toml -- search seed --segment fsp103_to_fsp104 --output build/search/g0 --size 16 --rng-seed 1

build/search/g0/manifest.json names every candidate JSON and compiled tape.
Evaluate those tapes with tools/glitch-hunting/evaluate-candidate.ps1, passing
the manifest candidate_id unchanged. Evaluations are independent files, so
10-20 native clients can run them concurrently without per-frame orchestration.

Collect repeated evaluator artifacts:

    cargo run --release --manifest-path tools/huntctl/Cargo.toml -- search collect --population build/search/g0/manifest.json --input build/search/evaluations/a.json --input build/search/evaluations/b.json --output build/search/g0-results.json

Rank and generate the next population:

    cargo run --release --manifest-path tools/huntctl/Cargo.toml -- search rank --population build/search/g0/manifest.json --results build/search/g0-results.json

    cargo run --release --manifest-path tools/huntctl/Cargo.toml -- search evolve --population build/search/g0/manifest.json --results build/search/g0-results.json --output build/search/g1 --size 16 --elites 4 --rng-seed 2

Ranking is lexicographic: deepest verified milestone, success rate across
repeated restores, median first-hit tick, best first-hit tick, then shorter
tape. A candidate that merely approaches an exit cannot outrank one which
activates the exact F_SP103 to F_SP104 transition.

For an engine-free plumbing test, search mock-evaluate assigns deterministic
scores from tape length. It exercises seed, tape compilation, ranking, and
evolution without claiming anything about game behavior.

Current mutations adjust macro duration, analog heading and magnitude, insert
rolls, split/delete movement segments, and shrink explicit waits. Candidate IDs
hash segment plus input program, so identical tapes deduplicate even if separate
search branches rediscover them; ancestry records the retained parent and
mutation for every new generation.
