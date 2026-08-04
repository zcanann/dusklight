# Route-learning integration audit

## Canonical learning path

`huntctl learn tactic-route` is the existing zero-shot route learner. Its
authoritative pieces are:

- `NativeTacticExecutionPlan`: sealed seeds, budgets, proposal width, branch
  cadence, refit cadence, worker-local checkpoint ownership, and root-replay
  fallback;
- `TacticQCampaign` and `StateGraph`: exact content-addressed states, root tick
  costs, authenticated action expansions, parent/child routes, terminal paths,
  training replay, and frontier scheduling;
- `NativeTacticProposalPool`: persistent owned native workers, worker-grouped
  sibling proposals, live-endpoint capture, direct process-local restore, and
  measured root-replay fallback;
- `CampaignTacticLearnerAuthority`: shared authenticated transition replay and
  learned policy publication;
- `macro_discovery`: mining and held-out validation of reusable action
  subsequences;
- route reconstruction and cold-replay evidence derived from graph authority.

This path already supports a learned policy without a human demonstration.
That configuration is the scored scratch campaign.

## Duplicate path being retired

`huntctl learn scratch-route` invokes `native_scratch_learner`. It owns a
separate tabular checkpoint and launches a fresh native worker for every
episode. Within an episode it retains only the latest live endpoint and walks
one linear chain. It does not admit its states to `StateGraph`, revisit an
earlier decision state, evaluate a sibling batch, share graph transitions, or
participate in tactic discovery.

The heading, duration, roll, and deletion refiners consume artifacts from this
path and evaluate complete route variants. They remain artifact diagnostics;
they are not the route-learning architecture and must not be used for further
scored mining.

The public `scratch-route` command is retired. `tactic-route` with the learned
proposal policy and no demonstration is the only canonical zero-shot entry
point.

## Existing save-state behavior

Every `StateGraphNode` binds an exact state fingerprint, authenticated route
checkpoint, full observation, best root tick cost, restoration locator, and
incoming/outgoing graph evidence. The graph reconstructs the lowest-cost
terminal route through these edges.

At a decision, each native process assigned sibling work materializes the
selected source state when necessary and repeatedly restores its process-local
checkpoint for the siblings assigned there. The selected nonterminal endpoint
is retained as a single-use live continuation for the following decision.
Every evaluated sibling is admitted to graph/training evidence even though only
the policy-selected sibling moves the current cursor.

The remaining locality limitation is concrete: the route coordinator keeps
only one `CachedTacticFrontier`, representing the latest selected endpoint, and
that endpoint is a single-use continuation. Historical graph nodes have
portable replay authority but normally no retained process-local handle.
Moreover, distributing a width-two branch across two processes makes the
counterfactual worker replay and capture the same prefix owned by the selected
worker. An exact-plan portable-owner treatment reduced two-worker
materializations from 18 to 3 and replayed prefix ticks from 1,206 to 148, but
serializing both separate requests on one owner was about 3% slower than the
optimized parallel control. The required fix is native multi-candidate sibling
execution from one checkpoint or concurrent expansion of independent graph
nodes, followed by a bounded node-indexed ownership cache; another save-state
system is not required.

## Reuse and retirement decision

- Reuse `NativeTacticExecutionPlan`, `TacticQCampaign`, `StateGraph`, the
  persistent proposal pool, learner authority, macro discovery, reconstruction,
  and cold-replay proof.
- Retire the public cold-root `scratch-route` command immediately.
- Keep its artifact readers and refiners only while retained historical
  evidence needs them; do not integrate their separate Q table or checkpoint
  into the canonical learner.
- Preserve the measured root-materialization and direct-restore accounting.
  Make one native request restore a graph-node checkpoint across its sibling
  candidates, or run independent graph-node expansions concurrently, before
  extending the single latest-endpoint cache to bounded node-indexed
  worker-local ownership. Do not add another save-state system.
