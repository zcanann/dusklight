# Generation-barrier learning schedule

Date: 2026-08-01

An explicit learned generation-barrier schedule was tested as an alternative
to sequential live replay. Each lane learned from its own trajectory while
running; experience from all lanes was merged deterministically between
generations.

A four-seed, two-generation smoke passed campaign completion and the V6 scratch
audit. Both lanes in generation zero consumed the same empty learner snapshot.
Both lanes in generation one consumed the same 15-row snapshot produced at the
first barrier, proving deterministic merge and inheritance.

The matched treatment used the retained two-seed, 24-decision, four-proposal,
two-worker campaign configuration. Only the learning schedule changed.

| Measurement | Sequential live replay | Parallel generation barrier | Change |
| --- | ---: | ---: | ---: |
| Campaign wall time | 163.317378 s | 118.523308 s | -27.4% |
| Unique useful expansions | 192 | 191 | -1 |
| Useful expansions/s | 1.176 | 1.611 | +37.0% |
| Native ticks | 3,904 | 3,680 | -224 |
| Native ticks/s | 23.904 | 31.049 | +29.9% |
| Worker busy share | 55.59% | 84.29% | +28.70 points |
| Aggregate worker idle | 137.443 s | 34.608 s | -74.8% |
| Terminal seeds | 1 | 0 | regression |
| Best authenticated tick | 314 | none | regression |

Completion validation and the V6 scratch audit passed for the treatment. The
report, summary, and audit SHA-256 values are respectively
`43e1671cb44d9e30c5aae0ace1908bea430280dda3a4ae6ef8e2cb5a44c847ac`,
`2e2444f4d547dc355cbd8ccce1dc925a2871a96a949f68b939f1841b1e29c610`,
and `6cd49f225d973b297499b22aba1787706e88869bed66dcbd8c1c86818a3b037d`.

The treatment is rejected despite the real utilization gain. Delaying
cross-seed experience until a generation boundary removed a known terminal,
so it cannot replace fresh sequential learning. Future parallel schedules must
preserve rapid propagation of valuable experience or demonstrate equal/better
terminal discovery under matched native budgets.
