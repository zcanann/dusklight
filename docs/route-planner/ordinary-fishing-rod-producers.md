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
preserving unrelated owned-item bits.

The profile is evidence- and exact-scope-parameterized. It does not claim that
the current world extractor has audited these actor families. Vine talk, hawk
command, cradle pickup, Uli interactions, collision reachability, and reward
commit are separate staged unresolved obligations. Consequently the upper-bound
projection demonstrates the causal producer chain, while modeled-feasible
search stops at the first missing physical proof. Future actor/event imports can
replace those obligation details without changing the downstream state graph.
