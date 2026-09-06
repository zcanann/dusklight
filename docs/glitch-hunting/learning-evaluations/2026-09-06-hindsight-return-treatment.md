# Delayed-value exploration treatment

The existing pre-success “reachability” treatment ranks immediate goal-relative
motion and calibrates that same one-step metric. Its name did not imply that
delayed achieved-goal returns actually selected actions.

`--value-treatment hindsight_return_knn` is an explicit comparison treatment.
It uses the existing state/action factor representation and nearest-neighbor
regression, but trains and ranks conditional negative time-to-go:

- Native-terminal-connected actions receive the complete authenticated return.
- Other observed actions teach reaching their own endpoints, even when the
  overall episode is unfinished.
- Earlier connected actions also learn returns to sampled achieved endpoints.
  Unconnected continuations are unknown, not zero-cost successes or failures.
- Authored-goal and hindsight-coordinate tasks have distinct model inputs.
  Before native success, queries use achieved-goal experience; after success,
  queries request the authored goal. Both kinds of experience remain in training.
- The model's value ordering can select the primary proposal without passing
  an immediate-motion calibration gate. Epsilon exploration, executable-action
  checks, graph scheduling, and exact terminal incumbents remain in place.

This is one conditional return regressor, not another reward bonus or an
additional terminal Double-Q model. It supplies the shared snapshot, live
action-selection, and learned-frontier interfaces. The established treatments
remain comparison controls; the default has not changed.

## Evidence and limitations

A connected synthetic replay has two ways to reach the same intermediate
coordinate: an initially-away 2-tick detour and an initially-toward 21-tick path.
With no native success in either path, the motion control selects toward;
the delayed-return model selects away. If only the longer path satisfies the
authored goal, the new model selects it instead of aliasing coordinate arrival
with native success. The terminal distinction comes from recorded predicates,
not a fabricated distance reward.

An orchestration check fits an immutable snapshot from retained sibling
experience, selects an unseen executable action through the production online
controller without motion calibration, checks the emitted input, and verifies
that epsilon still controls exploratory decisions.

These are wiring and controlled learning checks, not native route results.
All 460 learning and 500 orchestration library tests passed with two build jobs
and two test threads. No native search was launched during this implementation.
The CLI all-target compilation check also passed; it reported unused-code
warnings in the existing `harness_cli` test file, which was not changed here.
This remains conditional value learning from observed continuations, not proof
of effective exploration, broad transfer, or superior sample efficiency.
Nearest-neighbor interpolation may still be weak on unseen geometry and action
compositions. The next useful evidence is a bounded native comparison of this
treatment against the existing policy and a nonlearning control, including
route outcomes, policy-driven choices, checkpoint use, and wall time. Do not
promote it to the default based on the synthetic fixture alone.
