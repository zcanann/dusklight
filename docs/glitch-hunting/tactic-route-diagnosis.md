# Native terminal-route diagnosis

Use this report after a scratch campaign has retained its complete route report,
terminal results, and an independently authenticated ordinary demonstration:

```text
huntctl learn diagnose-tactic-terminal-routes \
  --scratch-report build/scratch/report.json \
  --demonstration-report build/demonstration/demonstration-report.json \
  --demonstration-corpus build/demonstration/demonstration-training.dtqc \
  --output build/scratch/route-diagnosis.json \
  --repository-root .
```

The command fails if the scratch and demonstration evidence differ in native
execution binding, objective, root checkpoint, or feature schema. It loads each
terminal seed's graph-selected best result rather than a copied tape name.

For the human route and every scratch route, the report retains:

- native ticks and option-duration distribution;
- typed option counts, roll options, raw A-button and camera-modifier ticks;
- native path length, option-local displacement and excess path length;
- measured speed, velocity retention inputs, stalls, collision correction,
  and total momentum loss;
- wall-contact overlap with commanded motion and momentum loss measured on
  exactly those ticks;
- neutral controller frames and repeated consecutive options.

For scratch routes it also joins every graph path transition back to the exact
proposal expansion and reports whether the selected action was available,
whether it had fitted value support, and whether roll and camera-modifier
actions were available at that boundary. The join uses typed descriptors and
state/result identities, not option-name parsing.

Demonstration capture does not have policy-ranking authority, so the report
sets `demonstration_action_surface_available` to false. It does not reconstruct
an action mask after the fact.

This is descriptive evidence. It neither scores similarity to the human route
nor creates reward components. A faster human action pattern can motivate an
action/feature coverage experiment, but it cannot become a waypoint,
privileged policy target, or promotion authority.
