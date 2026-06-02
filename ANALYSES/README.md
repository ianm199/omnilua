# ANALYSES — pre-computed cross-file lookups

The Bun project pre-computed `LIFETIMES.tsv` so per-file translation agents
didn't have to re-derive ownership decisions for every file. Same principle
applies here. These TSVs are **lookup tables**, not inference targets.

| File | Purpose | Status |
|---|---|---|
| `macros.tsv` | Every public macro in `lobject.h` / `lstate.h` / `llimits.h` → Rust equivalent | **populated** |
| `types.tsv` | Each C struct → Rust struct, field-by-field, with chosen Rust type | **populated** |
| `error_sites.tsv` | Every `luaG_runerror` / `luaD_throw` / `luaO_pushfstring`-then-throw → `Err(LuaError::...)` | **populated** |
| `file_deps.txt` | Header inclusion graph + canonical crate assignment | **populated** |

These are the real cross-file lookup tables — **look up, don't re-derive**. When
working in a translated file, consult them rather than re-inferring a macro/type/
error-site mapping the project already settled.

## Format

All TSVs: tab-separated, first row is a header row beginning with `#`. Comments start with `#` at line start.
