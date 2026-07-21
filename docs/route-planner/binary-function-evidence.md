# Binary function evidence

Status: the planner can bind an exact DOL and symbol table to one bounded text
function without depending on decompiler output or another project at runtime.

`binary-function-evidence/v1` records:

- SHA-256 identities for the complete DOL and symbol table;
- the exact symbol name, virtual address, and declared size;
- the containing DOL text-section index and computed file offset;
- the selected bytes and their digest; and
- a deliberately narrow instruction-shape classification.

The v1 extractor recognizes only an exact four-byte PowerPC `blr`
(`4e800020`) as `immediate_return`; every other body is `other`. It does not
infer what the function was intended to write, whether a call site executes,
or what a larger function does. Those are semantic/source/trace bindings layered
over this artifact. This separation prevents a friendly function name from
being treated as executable proof and lets the same audit be repeated for each
retail build.

```text
route-planner extract-function-evidence \
  --dol orig/GZ2E01/sys/main.dol \
  --symbols config/GZ2E01/symbols.txt \
  --symbol dComIfGp_ret_wp_set__FSc \
  --output return-place-writer.json
```

The parser rejects missing or duplicate symbols, non-function/non-text records,
overflowing virtual ranges, functions split across or outside text sections,
truncated DOL ranges, forged byte digests or classifications, and noncanonical
serialized artifacts.
