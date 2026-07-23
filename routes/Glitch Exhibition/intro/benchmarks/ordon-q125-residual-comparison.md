# Ordon q125 residual baseline comparison

This report compares two independent real native campaigns against the exact
125-tick incumbent. Both requests seal the same 160-tick exploration horizon,
promotion threshold (`< 125`), 4,096-candidate budget, 1,310,845-tick maximum,
terminal predicate, process-boot tape identity, world context, executable, and
game data. The only intended treatment difference is random sampling versus CEM.

| Measure | Random | CEM |
| --- | ---: | ---: |
| Completed / declared candidates | 4,096 / 4,096 | 4,096 / 4,096 |
| Unique compiled tapes | 4,096 | 4,096 |
| Exact successes | 2,881 | 3,620 |
| Exact failures | 1,215 | 476 |
| Successful-episode rate | 70.3369% | 88.3789% |
| Successful behavior classes | 1,458 | 2,380 |
| Best first hit | 125 | 125 |
| Candidate simulated ticks | 753,798 | 611,497 |
| Total charge, including demonstration | 753,924 | 611,623 |
| Successes per million candidate ticks | 3,821.98 | 5,919.90 |
| Strict sub-125 improvements | 0 | 0 |

CEM produced 739 more successful candidates, a success-rate advantage of
18.042 percentage points, 922 more successful behavior classes, and 1.549 times
as many successes per charged candidate tick. These are sample-efficiency and
basin-retention improvements, not a route-time improvement: both arms tie the
incumbent and neither authorizes promotion or an optimality claim.

## First-hit distributions

Counts are exact successful candidates at each first-hit tick.

| Tick | Random | CEM |
| ---: | ---: | ---: |
| 125 | 2,150 | 2,647 |
| 126 | 279 | 478 |
| 127 | 122 | 173 |
| 128 | 60 | 75 |
| 129 | 29 | 30 |
| 130 | 23 | 17 |
| 131 | 30 | 23 |
| 132 | 17 | 8 |
| 133 | 14 | 4 |
| 134 | 9 | 4 |
| 135 | 13 | 4 |
| 136 | 10 | 5 |
| 137 | 13 | 9 |
| 138 | 20 | 30 |
| 139 | 49 | 77 |
| 140 | 26 | 31 |
| 141 | 11 | 1 |
| 142 | 5 | 2 |
| 143 | 1 | 2 |

## Action and temporal coverage

Every declared action family, start octant, and temporal basis had nonzero
candidate and component coverage in both arms. Candidate counts are shown here;
the sealed audits additionally retain component counts.

| Action family | Random | CEM |
| --- | ---: | ---: |
| button press | 1,952 | 536 |
| button release | 1,565 | 250 |
| camera X | 1,107 | 303 |
| camera Y | 1,112 | 359 |
| main X | 1,109 | 1,514 |
| main Y | 1,126 | 2,175 |

| Temporal basis | Random | CEM |
| --- | ---: | ---: |
| button hold | 2,828 | 720 |
| cubic control curve | 555 | 1,700 |
| exact frame | 622 | 1,251 |
| piecewise-linear ramp | 610 | 205 |
| window 2 | 588 | 581 |
| window 4 | 578 | 286 |
| window 8 | 587 | 153 |
| window 16 | 604 | 186 |
| window 32 | 552 | 99 |

Random start-octant candidate counts for octants 0 through 7 are
`1393, 1155, 1055, 983, 1099, 1078, 852, 550`; CEM counts are
`701, 386, 700, 705, 508, 1505, 403, 279`. CEM concentrated heavily while
retaining 2,380 successful behavior identities. Its final categorical
concentration diagnostic is 1,000,000 millionths and its proposal rejection
rate is 652,793 millionths (11,797 attempted genomes, 461 invalid, 7,240
duplicate). Random's rejection rate is 152,843 millionths (4,835 attempted,
682 invalid, 57 duplicate). Thus the run records both CEM concentration and the
fact that it did not collapse to one realized tape or behavior.

## Evidence and interpretation

Both audits diagnose `completed_success_without_improvement`, declare the full
budget complete, have no pending candidates, and contain an empty
`improvement_by_simulated_tick` sequence. This rules out a truncated budget,
broken tape generator, absent successes, or zero-coverage action surface. It
does not establish that tick 125 is optimal; it establishes only that these two
sealed residual campaigns did not improve it.

- Random request: `1a56225cac097fed85bece1e95f7e00ef1d1fbaab1c79292efb607544a8a078a`
- Random execution: `9fe9add02f42533acf0ee035cbd4d2362a5543798ea05a8f2f0e840ea487f74d`
- Random audit: `52cc8cc7a7b249b909fd568bff2edfaadd09c15f4e70e80642f6813577f344af`
- Random final checkpoint: `b538e11dab0cb6ac561a49aa57e7497ac0201c3cc62faeb6a2fc73ef0cd0183d`
- CEM request: `0c71fb96d17e6371f0badbd3514361b7683b5935ec5f24e08767764cbac2863f`
- CEM execution: `0a5f9427f0ac392e05179b40b089f3db15604ba18db8b9a8982affaae4cb12ed`
- CEM audit: `0514072776833c285df36d055367bcd38bf96ecb9c4b4324d9744baa19559998`
- CEM final checkpoint: `dd8e3f7a5e5f78284ce44ef2dea0d1fa9ae5f53dee31ff628e8142e4ae887c84`

The resumable evidence roots remain under
`build/campaigns/ordon-q125-residual-random-v1` and
`build/campaigns/ordon-q125-residual-cem-v3`. Superseded optimizer snapshots
are pruned only after their replacements and journal events become durable;
the hash-chained journals retain their historical identities.
