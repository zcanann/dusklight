# Recorded paths must receive complete delayed credit

## Finding

The active terminal-return builder computed reachability with repeated whole-graph
scans, then computed time-to-terminal with at most 512 synchronous backups. A
longer supported path could therefore raise “terminal-supported transition has
no finite first-hit path” despite having a recorded terminal continuation.
The achieved-goal return builder separately stopped at 16 edges and 4,096 states.
These limits counted actions, not elapsed game time: splitting movement into
shorter actions could change which earlier decisions received credit.

## Change

Both builders now share a reverse shortest-path calculation over recorded edges.
It calculates complete conditional native tick costs without a hop cutoff,
handles cycles and competing continuations, and stops native returns at the
first recorded terminal boundary. Open components still have no closed return;
they are not relabeled as failures or fabricated successes.

This removes repeated whole-graph convergence sweeps. Resource use is bounded
by the admitted graph; existing replay/sample budgets still apply. Training
iteration settings no longer truncate exact recorded path costs. Rewards,
terminal predicates, and native execution are unchanged.

## Verification scope

The focused checks cover a 2,048-action chain in both input orders, equivalence
to a single action of the same duration, achieved-goal credit across that chain,
weighted alternate routes, connected and disconnected cycles, terminal-boundary
handling, and comparison against exhaustive relaxation on varied small graphs.
Existing generalized-value tests exercise the consumers of those costs.
All 458 learning and 499 orchestration library tests passed with two build jobs
and two test threads. No native campaign was launched for this change.

This corrects delayed supervision after a continuation has been recorded. It
does **not** establish that the learner can discover a new detour or that native
campaigns run faster overall. In particular, `rank_goal_reachability` still
selects by predicted immediate target-relative motion; achieved-goal returns do
not currently determine that ordering. The connection between delayed learned
value and exploratory selection remains unfinished, not a completed capability.
