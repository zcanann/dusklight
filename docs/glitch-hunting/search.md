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

For a complete local round, use the VS Code task `Glitch Hunt: Run Search` and
pick either objective. The task builds the native client, runs two generations
of eight candidates with two trials each on four concurrent clients, writes the
leaderboards under `build/search`, and promotes the final winner for visual
playback. The equivalent command is:

    .\tools\glitch-hunting\run-search.ps1 -Segment fsp103_to_fsp104

Population size, generations, elites, repetitions, workers, seed, and output
root are command-line parameters. A native timeout, crash, or malformed result
fails the round; it is never silently counted as a poor candidate. A legitimate
goal miss remains a scored partial sample.

After the round, run `Glitch Hunt: TAS Playback` and select
`route-search-champion` or `boot-search-champion`. The former recreates the
direct `F_SP103,1,1,3` search start; the latter replays from a cold process boot.

The initial route acceptance hunt (seed 424242, four generations, twelve
candidates) reduced the verified exit-commit score from tick 571 to tick 493.
The promoted 581-frame champion repeated tick 493 in 3/3 isolated trials and
loaded `F_SP104` each time. The cold-boot baseline reached controllable
`F_SP103` at tick 649; boot timing golf now uses that memory-backed result as
its incumbent.

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
