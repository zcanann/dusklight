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
- `NativeTacticProposalPool`: persistent owned native workers, batched sibling
  proposals, live-endpoint capture, direct process-local restore, and measured
  root-replay fallback;
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

At a decision, the proposal pool materializes the selected source state once
when necessary and evaluates sibling actions by repeatedly restoring that
process-local checkpoint. The selected nonterminal endpoint is retained for a
direct restore on the following decision. Every evaluated sibling is admitted
to graph/training evidence even though only the policy-selected sibling moves
the current cursor.

The remaining locality limitation is concrete: the route coordinator keeps
only one `CachedTacticFrontier`, representing the latest selected endpoint.
Historical graph nodes have portable replay authority but normally no retained
process-local handle. Revisiting one therefore pays one authenticated prefix
materialization before its sibling batch can restore locally. Checkpoint memory
capacity exists, but the coordinator does not yet maintain a node-indexed cache
of those live handles.

## Reuse and retirement decision

- Reuse `NativeTacticExecutionPlan`, `TacticQCampaign`, `StateGraph`, the
  persistent proposal pool, learner authority, macro discovery, reconstruction,
  and cold-replay proof.
- Retire the public cold-root `scratch-route` command immediately.
- Keep its artifact readers and refiners only while retained historical
  evidence needs them; do not integrate their separate Q table or checkpoint
  into the canonical learner.
- Measure current root materialization and direct-restore costs before changing
  checkpoint retention. If historical frontier materialization is material,
  replace the single latest-endpoint cache with bounded node-indexed
  worker-local ownership rather than adding another save-state system.
