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

Trace inspection changes the interpretation of the terminal regression. The
314-tick route was produced by two unselected `batch_coverage` proposals at
decision 12. The policy selected `neutral` with reason
`unsupported_bootstrap`, and that selected trajectory did not terminate. The
control therefore proves that exploration can discover and authenticate a
terminal, but it does not prove that live replay learned, selected, or
reproduced the route. The barrier treatment lost a stochastic coverage hit,
not an established learned behavior.

The schedule is not promoted as the default: the utilization gain is real, but
neither treatment supplies causal learning evidence. Future schedule decisions
must report discovery separately from policy adoption and compare learned,
frozen, and random-valid treatments under matched native budgets.

## Multi-generation follow-up

A four-seed comparison used two generations, 14 decisions per seed, four
proposals per decision, two workers, branch cadence 7, and refit cadence 2.
The second barrier generation consumed the merged first-generation replay.
Both cells passed completion validation and the V6 scratch audit.

| Measurement | Sequential live replay | Two-lane generation barrier | Change |
| --- | ---: | ---: | ---: |
| Campaign wall time | 208.666 s | 138.042 s | -33.8% |
| Unique useful expansions | 224 | 223 | -1 |
| Useful expansions/s | 1.073 | 1.615 | +50.5% |
| Native ticks | 4,998 | 4,726 | -272 |
| Native ticks/s | 23.952 | 34.236 | +42.9% |
| Worker busy share | 51.57% | 77.35% | +25.78 points |
| Aggregate worker idle | 194.174 s | 58.793 s | -69.7% |
| Terminal seeds | 0 | 0 | no proof |

The sequential cell also lost the previously observed coverage terminal after
adding two earlier seeds and changing update cadence. This prevents a
terminal-quality claim for either schedule. More importantly, no retained cell
yet proves that experience caused a policy to adopt terminal-producing
behavior. Improving raw capacity alone will not make the framework useful; the
next evaluation must distinguish exploration luck from learning and show that
successful experience changes subsequent choices under ordinary curriculum and
cadence changes.

The sequential report/audit SHA-256 values are
`a66abfaa769a6c28058edb58c34bd94247d66bcd6dec40bf379ddd08a7229744`
and `452ae4365d84ce817378590c206f068557fb954760c66b0231a8bae1895b4758`.
The barrier report/audit values are
`69f203a0526331c0388ca4c6a323712de45a4556dd42b120e5becc70ab04bcf1`
and `2afe6322bdeb1d669b8e8dd5ba18925213a589a1ba08a75070b268d346b5da74`.
