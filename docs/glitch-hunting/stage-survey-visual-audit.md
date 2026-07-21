# Stage-survey visual audit

This audit checks that representative rendered scenes agree with the native
data retained by the stage survey. It is a human visual check of a stratified
sample, not a substitute for the authenticated all-catalog survey or proof that
every active actor is on camera.

## Method

Four terminal frames were captured at 1280x960 with the real Metal renderer,
blocking pipeline compilation, fixed 30 Hz simulation, unpaced host execution,
the survey's exact generated input tape and survey cvars. The same process also
wrote an all-channel trace and a complete terminal actor catalog. Presentation
was suppressed and the process exited at the resolved final tape frame.

The three 30-tick neutral cases use executable
`81fde22c999ee51d47e539bca16a003e4751b0c15e5e73225f2b085abef7da46`
and reproduce both ledger-bound artifacts byte for byte. The 720-tick loading
case deliberately replays the older survey tape on that current executable.
Its trace still reproduces the older ledger-bound trace byte for byte. Its
terminal actor catalog differs only in the embedded build identity: deleting
the top-level `build` object and serializing with sorted keys gives
`359b33684938b46887fe04141c60448d44481ce6acc56fffabaf56f12ea5423d`
for both catalogs. This is semantic cross-build agreement, not a claim that the
two build identities are interchangeable.

The PNGs remain below ignored `build/stage-survey/visual-audit-71eb4be/` because
they contain retail game imagery. This report records their full digests so a
local reproduction can be distinguished from a different frame.

| Sample | Requested boot / rendered terminal | PNG SHA-256 | Trace SHA-256 | Actor-catalog SHA-256 |
| --- | --- | --- | --- | --- |
| Lakebed Temple | `D_MN01/0/0/-1` / `D_MN01/0/0` at tick 29 | `6b933e4a4537b38f9a6110365dec203c4914a514c6f54f1bd138cc30cc64de28` | `be06d2291380e19e70c1702012df94aabdefefcfef07b9fc9c05195c79e813f2` | `4b1a825fab4cca1083135cd347261aee3dd0faee11e178d9c1d1fce42de53463` |
| Cave of Ordeals | `D_SB01/0/0/-1` / `D_SB01/0/0` at tick 29 | `ac7215cc3d118c2fec62afe916b8c0b6772326b8e0eb2df06239f18a206ae852` | `3f0eda4acc5db6f537a1b4ea67e675d4b253e58ce4b0f0b49b85b4684092fbd5` | `386c241f221b442b4ec13d04c87579c85fbc9a0312a45af8c102b5c941127825` |
| Ordon Village | `F_SP103/0/0/-1` / `F_SP103/0/6` at tick 29 | `5a2a9e3f9cdf1ee9ebe9aec97c63a03fe3ad6f403482506ff65ee3dc2469690d` | `43578c3b764a8fc46eb1321828c0d3d66a73595648c12d127e2f95ecfa3a952a` | `463a7a485ee45d4ff83edc39ebdfe7e855a59944f1dbfd4e89fd78a24550aaca` |
| Village-to-ranch loading sweep | `F_SP103/0/0/-1` / `F_SP00/0/12`, point 7, at tick 719 | `ac4e403ebb2054d3462782aaec8b22ca7cbfe6131fab87d98d7b6545dfc44802` | `ef5ada8e9ab73ce63e8f447d3eabfe37de7f6a4fa2070f510f82300165562634` | `9237fd25bc94e0559bd42ecb0d187013f9158310b574ead2fadb15645fc9d0e9` |

## Reconciliation

### Visible actors

- Lakebed renders Link swimming through an underwater tunnel. The terminal
  actor catalog places `Link` at the trace position and retains all 75 actors
  across 21 profiles, including 14 enemies. `Link` and `Midna` are the two
  actors with an instantiated model at that boundary.
- Cave of Ordeals renders Link standing in a narrow stone passage. The catalog
  places `Link` at `[-2750, 1100, 0]`, the exact trace position, and retains all
  24 actors across 15 profiles, including seven enemies. `Link` and `Midna` are
  the two instantiated actor models at the boundary.
- Ordon Village renders Link, the horse and village inhabitants in the expected
  outdoor scene. The catalog places `Link` at the exact trace position, names
  the visible `Horse`, and retains all 130 actors across 44 profiles. Its six
  instantiated actor models are `Link`, `Horse`, three `Ni` instances and
  `Midna`.
- The ranch terminal frame visibly contains Link and nearby inhabitants after
  the load. Its catalog retains all 60 actors across 19 profiles; instantiated
  actor models are `Link`, `Horse` and `Midna`.

In every sample, `observed_actor_count == retained_actor_count ==` the complete
learner actor population and neither the terminal catalog nor the dynamic
collider population is truncated. The compact trace's bounded
`selected_actors` diagnostic is intentionally truncated; it is not used as the
complete-population claim.

### Collision and trigger state

- Lakebed visibly places Link in water between a floor and tunnel roof. The
  trace independently reports present ground, roof and water identities at
  heights `333.70877`, `1017.0966` and `1185.0`, three realized surface
  identities, and 78 retained dynamic colliders. The actor catalog also retains
  an enabled scene-exit box and an enabled mapped-event elliptic cylinder.
- Cave of Ordeals visibly brackets Link between a floor and low stone ceiling.
  The trace reports ground `1100.0`, roof `1650.0`, explicit water absence, two
  realized surface identities, and 21 dynamic colliders. Its enabled scene-exit
  and mapped-event volumes are retained even though invisible trigger geometry
  is not rendered into the ordinary game frame.
- Ordon Village visibly places Link on sloped terrain near the village
  boundary. The trace reports the same player and ground height
  (`724.55334`), one realized ground surface, 81 dynamic colliders, and an
  enabled exit box. Link is inside that box at signed distance `-29.094116`;
  the decoded destination is `F_SP00`, room 0, point 7.
- The ranch frame visibly places Link on solid ground. The terminal trace
  reports the same player and ground height (`15000.0`), one realized ground
  surface and 67 dynamic colliders. The terminal catalog retains the enabled
  ranch event-area cylinder.

The visible scene geometry agrees with the typed collision facts. Invisible
trigger volumes are reconciled through their typed geometry and, for the Ordon
exit, through the observed transition below; a normal rendered frame alone
cannot prove an invisible volume's exact bounds.

### State transition

The loading-sweep trace has 720 records with no exhausted capacity and is
byte-identical to the earlier headless trace. It records this sequence:

1. Boundary 2 resolves the containing Ordon Village exit to `F_SP00`, room 0,
   point 7.
2. Boundary 42 enables that next-stage request.
3. Boundary 71 records the old player and collision channels as absent during
   teardown rather than fabricating zero-valued state.
4. Boundary 72 changes the observed stage to `F_SP00`, layer 12.
5. Boundary 100 restores a present player in room 0, and boundary 102 restores
   a typed ground identity.
6. Boundary 720 remains ready in `F_SP00/0/12`, point 7, matching the captured
   ranch frame and terminal actor catalog.

This closes the requested stratified visual inspection. It does not close the
separate all-successful-entry actor-population audit, all-map channel audit, or
machine-readable all-stage coverage matrix.
