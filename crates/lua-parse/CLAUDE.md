# lua-parse — parser and bytecode codegen

Recursive-descent parser + single-pass codegen (the front end pairs with
`lua-lex` for the lexer and `lua-code` for opcode encoding). Read the root
`../../CLAUDE.md` first.

## This is where syntax/codegen version seams live

Per-version differences in *parsing* and *what bytecode/line-info gets emitted*:

- Name resolution / `_ENV` threading; `global` declarations (5.5);
  attribute syntax (`<const>`/`<close>`); for-var const-ness (5.5); goto/label
  scoping (block-scoped pre-5.4 vs function-wide 5.4+); operator availability
  (`//`/bitwise rejected pre-5.3).
- **Line attribution** (drives `debug.sethook(f,"l")` traces, errors, tracebacks)
  is version-specific. Two examples from issue #92:
  - `forbody`/`fixforjump`: the numeric-`for` jump structure is emitted the same,
    but ≤5.3 vs 5.4 interpret it differently in the VM (see `lua-vm/CLAUDE.md`).
  - `test_then_block`: 5.5 attributes an `if`/`elseif` conditional `TEST`/`JMP` to
    the condition-expression line; ≤5.4 to the `then`-keyword line. Capture the
    line *before* `check_next(TK_THEN)` and gate on V55.

## Error wording goes through the lexer formatter

Syntax-error messages (`'<name>' expected near '<eof>'`) are built by
`lua_lex::lex_error` / `token2str` / `txt_token`. These read the active version
from the **`version` field on `LexState`** (snapshotted at lexer setup) so the
formatter doesn't need a `&LuaState`. 5.1 quotes the special multi-char tokens
(`<eof>`, `<name>`) that 5.2+ leave bare — that gate is in `token2str_raw`.

## Gotchas

- Codegen is single-pass; `fixforjump`/patch-lists backpatch jump targets. Get the
  `Bx` sign/offset right (`OFFSET_S_BX`) — a wrong jump is a silent infinite loop
  or skipped body.
- `luaK_fixline`-style line fixups matter for traceback fidelity; the active
  TODO/seam is around the conditional `TEST`/`JMP`.

## Test
`cargo test -p lua-parse`; behavior + line-trace assertions in
`multiversion_oracle.rs`; `harness/run_official_test.sh
reference/lua-c/testes/{constructs,errors,goto,db}.lua`.
