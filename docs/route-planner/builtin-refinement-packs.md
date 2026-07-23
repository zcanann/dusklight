# Built-in refinement packs

The planner ships two opt-in GZ2E01/English refinement packs. They are ordinary
canonical `dusklight.route-planner.refinement-pack/v15` artifacts: there is no
hidden solver allow-list or implicit movement bypass. Authors may inspect,
export, compose, replace, or omit them with the same rules as any external pack.

| Pack ID | Contents | Evidence behavior |
| --- | --- | --- |
| `builtin.gz2e01.ordinary-movement` | A controlled-movement technique and its explicitly named approach obligation | Established; only transitions that name the obligation can use it |
| `builtin.gz2e01.selected-sequence-breaks` | Early Master Sword human-form state, Epona OOB, and rupee-clip technique records | EMS is established; Epona OOB and rupee clip are contested and therefore require research evidence mode |

Every manifest and rule is scoped to the exact supported content and runtime
configuration digests. The Epona OOB and rupee-clip records discharge only their
method-specific obligations; they do not erase collision, approach, or timing
facts globally. EMS performs the one modeled state change—setting human form
while the Faron-twilight gate is active—and does not invent inventory or story
writes.

List the registry and its canonical digests:

```sh
tools/route-planner/target/debug/route-planner \
  list-builtin-refinement-packs
```

Export a pack before composing it:

```sh
tools/route-planner/target/debug/route-planner \
  export-builtin-refinement-pack \
  --id builtin.gz2e01.selected-sequence-breaks \
  --output build/route-planner/selected-sequence-breaks.pack.json

tools/route-planner/target/debug/route-planner compose \
  --facts build/route-planner/facts.json \
  --mechanics build/route-planner/mechanics.json \
  --pack build/route-planner/selected-sequence-breaks.pack.json \
  --output build/route-planner/composed.json
```

Unknown registry IDs fail closed. Exported bytes round-trip through strict
canonical pack decoding, and composition retains the pack ID and digest in the
refinement stack used by proof and solve reports.
