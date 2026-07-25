# Glitch Exhibition review route

Launch the checked-in route with the current `huntctl` source so Workbench and
timeline schema support cannot drift apart:

```sh
cargo run --manifest-path tools/huntctl/Cargo.toml -- timeline workbench \
  --timeline "routes/Glitch Exhibition/intro.timeline" \
  --game build/macos-default-debug/Dusklight.app/Contents/MacOS/Dusklight \
  --dvd "orig/GZ2E01/Legend of Zelda, The - Twilight Princess (USA).iso" \
  --state-root build/automation-state/route-workbench-current
```

The `main` continuation's promoted To Ordon Springs recording contains 126
controller-input frames and has verified approach and load-commit predicates.
The terminal is first observed at zero-based simulation tick 125; that score was
previously presented as though it were the artifact's frame count. Its sibling
`Best near miss (2.7813, unproved)` remains playable for visual comparison but
has no goal proof and is not part of the promoted continuation.
