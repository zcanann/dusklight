# Headless audio suppression before/after

Date: 2026-08-01

Native subsystem evidence showed that CPU draw traversal cannot be suppressed:
the isolated treatment changed native state, the applicable action surface, and
terminal evidence. Deterministic audio emulation and the game audio update were
safe to suppress on the same 16-tick workload. The v3 production treatment and
both retained-audio comparators emitted identical native state trajectory,
action-surface, controller-output, and terminal-evidence identities.

The passing parity report is
`build/diagnostics/current-native/draw-audio-parity-v2-report.json`
(`6d10f6d0860f372f55705355f0c8ea823bebcb4edcfdeff10bde66e4e122e6ea`).
The earlier diagnostic report that rejected CPU draw suppression is
`build/diagnostics/current-native/draw-audio-parity-v1-report.json`
(`8c2408e94ca900a5d38562f43e6cf67b097286e8fbd55c78e1d4a5944bba18c1`).

## Matched campaign

The end-to-end treatment used one worker, seeds 130363 and 181081, 24
decisions per seed, four proposals per decision, branch cadence 8, refit
cadence 4, epsilon 350000 per million, and the same 1 GiB campaign memory
budget. The control and treatment both produced 48 decisions, 192 unique
useful graph expansions, 3,904 native ticks, one terminal, and a 314-tick best
route. Every decision's selected option, frontier, before/after native state,
reward, goal distance, proposal outcome, tick count, and terminal flag matched.
The winning tapes were byte-identical with SHA-256
`5c279145712dff9434ceae6c0e5dfbf0b2394513d64dd8d0c533883d1072987c`.

| Measurement | Audio retained | Audio suppressed | Change |
| --- | ---: | ---: | ---: |
| Campaign wall time | 220.831690 s | 188.477574 s | -14.7% |
| Useful expansions/s | 0.869440 | 1.018688 | +17.2% |
| Native wait | 140.335034 s | 115.625795 s | -17.6% |
| Candidate ticks | 9,749 | 9,749 | exact |
| Deterministic audio emulation | 3.041532 s | 0.000004 s | effectively removed |
| Game audio update | 0.130036 s | 0.000001 s | effectively removed |
| CPU draw traversal | 7.240936 s | 5.374903 s | retained; timing varied |
| Corpus encoding | 13.138727 s | 10.698814 s | timing varied |

The control report is
`build/diagnostics/current-native/v44-compact-graph-identity-release-d24-v1/w1-r1/report.json`
(`0a01525d8e17d8e2368771070feff4c3c93dc4c4763b15569fb267cd010a6a19`).
The treatment report is
`build/diagnostics/current-native/v44-audio-suppressed-release-d24-v1/w1-r1/report.json`
(`76bcee97563aa4197db2199ed8d4011dcab26b19207b50a6d4297fc68ec00374`).
Both completion validation and the v6 scratch audit passed for the treatment.

Only the 3.17 worker-seconds removed from the two audio phases are directly
attributable to this change. The larger wall-time difference includes variation
in draw, simulation, corpus encoding, persistence, and model timing, so it is a
directional end-to-end result rather than a claim that audio alone caused the
entire 14.7% reduction.

## Worker scaling after suppression

A matched two-worker treatment used the same binding, plan, seeds, decisions,
and budgets as the one-worker treatment. Both produced the same 192 expansions,
3,904 native ticks, 314-tick terminal, decisions, states, and winning tape. Both
completion validation and v6 scratch audit passed.

| Measurement | One worker | Two workers | Change |
| --- | ---: | ---: | ---: |
| Campaign wall time | 188.477574 s | 158.700811 s | -15.8% |
| Useful expansions/s | 1.018688 | 1.209823 | +18.8% |
| Native wait | 115.625795 s | 87.402178 s | -24.4% |
| Worker busy share | 63.22% | 56.40% | -6.82 points |
| Aggregate worker idle | 66.396977 s | 131.058529 s | +97.4% |
| Proposal queue wait | 84.903567 s | 39.223623 s | -53.8% |

Two workers remain materially underutilized. The second process improves useful
throughput by only 18.8% because each seed generation and each decision's
learning, evidence, and persistence work remain serialized around native
proposal batches. Further worker-count increases are not justified until that
controller work overlaps independent native execution while preserving a
deterministic learner-publication order.
