# Ordinary Fishing Rod producers

`OrdinaryFishingRodProfile` compiles the ordinary Ordon quest into six candidate
transitions instead of one semantic “get rod” edge:

1. the vine-route dialogue produces `vine_guidance`;
2. the vine route produces `hawk_perch_reached`;
3. the hawk command changes the cradle from `held-by-monkey` to `grounded`;
4. pickup produces the explicit `carrying_cradle` state;
5. Uli's return interaction clears carry state and produces `cradle_returned`;
6. Uli's reward phase writes item ID `0x4a`, routes it through the generic
   item-bit operation, and commits `reward_claimed`.

Every guard reads the state written by the previous producer. No step checks a
friendly `ordinary_rod_quest_complete` flag, and the reward does not overwrite
the inventory byte vector: `SetBitFromValue` adds the selected item while
preserving unrelated owned-item bits. Every producer also retains the explicit
`F_SP103` stage guard, so initializing compatible-looking quest fields in an
unrelated title/file state cannot manufacture the route.

The profile is evidence- and exact-scope-parameterized. It does not claim that
the current world extractor has audited these actor families. Vine talk, hawk
command, cradle pickup, Uli interactions, collision reachability, and reward
commit are separate staged unresolved obligations. Consequently the upper-bound
projection demonstrates the causal producer chain, while modeled-feasible
search stops at the first missing physical proof. Future actor/event imports can
replace those obligation details without changing the downstream state graph.

## Chicken and OOB branches

`compile_with_chicken_routes` adds four candidates over the same component:

- chicken pickup produces `carrying_chicken`;
- the vine bypass consumes that carry and produces the ordinary
  `hawk_perch_reached` prerequisite;
- the direct OOB branch instead consumes it to produce `carrying_cradle`, marks
  `reload_required`, and clears `uli_interaction_ready`; and
- actor reload preserves cradle carry, clears the reload requirement, and
  restores Uli interaction readiness.

The vine bypass therefore mixes with the ordinary hawk displacement and cradle
pickup transitions without a special hybrid-route flag. The OOB branch joins
later, at the shared cradle-return transition, but cannot do so before its
explicit reload. Regression searches remove the unused alternatives and prove
both the mixed chicken-plus-hawk route and the direct OOB/reload route reach the
same unchanged Uli reward transition.

## File 0 is a producer cut, not a rod-loss state

The acceptance query also runs from a title-origin file at `F_SP102`. All
ordinary and chicken candidates are hard-guarded to Ordon `F_SP103`, so an empty
file-0 inventory has no reachable producer in this bounded catalog and reports
`unreachable_under_model`. The same title-origin snapshot with item bit `0x4a`
already set satisfies `goal.obtain-fishing-rod` with no action. Runtime-file
origin never appears in the goal predicate, so Back in Time does not fabricate
a second `rod_lost` condition; availability is exactly the current inventory
bit plus the reachability of ordinary alternate producers.

## Shared goal and residual safety

The catalog exposes one `goal.obtain-fishing-rod`, defined only by ownership of
item bit `0x4a`. Ordinary, chicken-plus-hawk, and direct OOB/reload executions
all satisfy that predicate through the same Uli reward operation. The Auru
model's generic recent-item consumer targets the same bit through the same
`SetBitFromValue` semantics.

The friendly goal does not erase method state. Regression execution retains
each complete quest component and proves the three Ordon outcomes remain
distinct: ordinary guidance, chicken vine bypass, and OOB reload history do not
collapse into one snapshot. A pre-owned unrelated inventory bit also survives
all three grants. A UI region may therefore label each proof “Obtain Fishing
Rod” while continuation identity still decides whether their residual states
are interchangeable downstream.
