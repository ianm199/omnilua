# omniLua rebrand spec — 2026-06-12

Owner: Fable (supervision, merges, repo-level actions). Execution: Opus
subagents per packet. This is the plan of record for the lua-rs → **omniLua**
rebrand and visibility push. Name availability verified 2026-06-12: crates.io
`omnilua`/`omnilua-cli` 404, npm `omnilua` 404, zero GitHub repos, no
colliding project.

Status checklist (tick with evidence):

- [x] N0: name availability verified (crates.io / npm / GitHub, 2026-06-12)
- [x] R0: repo renamed `ianm199/omnilua`, description + 9 topics set
      (2026-06-12; github.com redirects verified)
- [x] R1: mechanics merged (PR #165, 78 files / 8 commits): crates
      omnilua/omnilua-cli, binary omnilua, OMNILUA_VERSION canonical with
      fallback, npm omnilua 0.1.0, 30 scripts, workflows. Gates: workspace
      379/0, official full pass, check.sh x5, canaries 36/0, wasm pkg ok,
      env precedence smoke. Post-merge main verify: oracle 165/0, official
      44/44, wasm pkg ok.
- [x] R2: playground merged (PR #166, 31 KB single file): five-column hero
      (5.4 LIVE in wasm, others verified-ref labelled honestly), permalinks,
      6-snippet gallery. Supervisor live-reviewed production
      (ianm199.github.io/omnilua): Run works, outputs correct, zero console
      errors. R2b (multi-version wasm backend per
      specs/WEBLUA_MULTIVERSION_API_SPEC.md) in flight to light up all five
      columns; page needs zero changes when it lands. → R2b MERGED
      (PR #169): all five backends selectable through the wasm ABI
      (+720 B), version-smoke committed and wired into the package check,
      production browser-verified 2026-06-12: ALL FIVE COLUMNS LIVE
      (custom no-reference snippet diverges correctly per version).
- [x] R3: docs merged (PR #167): README rewrite (744 words, wedge-first,
      verified API examples, link-check table), npm README in
      packages/omnilua/, CHANGELOG entry, identity lines, examples READMEs
      (merge-time fixup). Badges 404 until first publish — expected.
- [ ] R4: versions already at 0.1.0 on main via R1; release.yml publish
      list updated. REMAINING = the user's irreversible step per
      RELEASING.md: tag the chosen merge SHA (`git tag v0.1.0 <sha> && git
      push origin v0.1.0`) → publishes omnilua/omnilua-cli/internals to
      crates.io and omnilua to npm. R2b HAS MERGED — ready to tag at any
      merge SHA ≥ 4b411d0. The first published omnilua npm package will be
      multi-version.

## Canonical name map (every packet uses EXACTLY these)

| surface | old | new |
|---|---|---|
| project wordmark | lua-rs | **omniLua** (display), `omnilua` (identifiers) |
| GitHub repo | ianm199/lua-rs | ianm199/omnilua (github.com redirects; Pages path does NOT — update all links) |
| Pages site | ianm199.github.io/lua-rs | ianm199.github.io/omnilua |
| embedding crate | lua-rs-runtime | **omnilua** (lib name `omnilua`; dir `crates/lua-rs-runtime/` UNCHANGED) |
| CLI crate | lua-cli | **omnilua-cli** (dir `crates/lua-cli/` unchanged) |
| CLI binary | lua-rs | **omnilua** |
| npm package | lua-rs-wasm | **omnilua** (dir `packages/lua-rs-wasm/` → `packages/omnilua/`) |
| version env var | LUA_RS_VERSION | **OMNILUA_VERSION** (new canonical; LUA_RS_VERSION still read as fallback, documented) |
| workspace version | 0.0.33 | **0.1.0** |
| internal crates | lua-vm, lua-gc, lua-types, lua-parse, lua-stdlib, lua-hlua-shim | UNCHANGED (implementation details; not marketing surfaces) |

## Copy deck (use verbatim; consistency beats creativity)

- One-liner (gh description, Cargo `description`, npm `description`):
  **"Every Lua, everywhere — pure-Rust Lua 5.1–5.5, suite-passing,
  LuaRocks-compatible, wasm-ready."**
- README header: `# omniLua` then: **"Every Lua, everywhere.** One pure-Rust
  runtime for Lua 5.1, 5.2, 5.3, 5.4, and 5.5 — as a standalone interpreter,
  embedded in Rust, or in the browser. No C dependency, no unsafe FFI. Passes
  the official PUC-Rio test suites. Runs the stock LuaRocks client."
- The wedge paragraph (README, npm, playground): "If your Rust program — or
  your game — ships a wasm build, a C-backed Lua binding can't follow it.
  omniLua is pure Rust: the same scripting runtime compiles natively and to
  `wasm32-unknown-unknown`, with no Emscripten and no toolchain gymnastics."
- The receipts line: "Conformance you can check: the official Lua test suites
  pass on every supported version, benchmarks vs reference C are published
  per-commit, and we measured what memory safety costs — see
  `docs/PERFORMANCE_MODEL.md` (§Safety-tax ablation): ~0% of wall time."
- Honesty constraints (keep, verbatim or near): "~1.3× geomean of reference C
  wall time — competitive, not faster, and not LuaJIT. If you need LuaJIT
  speed or a decades-mature binding, use mlua."
- GitHub topics: lua, rust, wasm, webassembly, interpreter, scripting,
  game-development, bevy, sandbox, lua54, lua51

## Surfaces that MUST NOT change (historical record)

- `CHANGELOG.md` existing entries, `docs/` evidence specs
  (ISSUE_BURNDOWN_SPEC, PERF_SPRINT_2_*, EXACT_ROOTING_SPEC, memos),
  `specs/` research docs, `ANALYSES/`, PORT STATUS trailers, commit history
  references. Only LIVING/PUBLIC surfaces rebrand. No blind repo-wide sed.
- `reference/` (vendored upstream Lua) — never touched.
- Local directory name `lua-rs-port/` and sibling-repo paths — unchanged
  (private workspace paths, not public surfaces).

## Packets

### R1 — mechanics (Opus, isolated worktree, branch `rebrand/mechanics`)

1. Cargo: `crates/lua-rs-runtime/Cargo.toml` package+lib → `omnilua`;
   `crates/lua-cli/Cargo.toml` package → `omnilua-cli`, `[[bin]] name =
   "omnilua"`; fix every internal `lua_rs_runtime::` use-site; workspace
   version 0.1.0 everywhere (internal crates too); descriptions from the copy
   deck; repository/homepage URLs to the new repo.
2. Env var: find where `LUA_RS_VERSION` is read; make `OMNILUA_VERSION`
   canonical with `LUA_RS_VERSION` fallback (single resolution point, no
   fallback-chain sprawl — read new, else old, else default).
3. Binary-name plumbing: every live script that invokes
   `target/{debug,release}/lua-rs` (harness/**, specs/oracle/**, Makefile,
   workflows) updated to `omnilua`. Mechanical but LARGE — grep first, sed
   carefully, run the gates to prove it.
4. npm: `packages/lua-rs-wasm/` → `packages/omnilua/`, package name
   `omnilua`, version 0.1.0, description per copy deck; fix
   `scripts/build-wasm.mjs` and `.github/workflows/static.yml` paths;
   `check_wasm_package.sh` paths.
5. `RELEASING.md`: new names, the 0.1.0 plan, and a "legacy crates" note —
   final `lua-rs-runtime`/`lua-cli`/`lua-rs-wasm` 0.0.34 pointer releases
   (README-only: "renamed to omnilua") are listed as an optional follow-up
   the user triggers.
6. Gates (all must pass at tip): cargo test --workspace ·
   harness/run_official_all.sh (44/44) · specs/oracle/check.sh 5.1–5.5 ·
   harness/canaries/gc/run_canaries.sh · `WASM_SKIP_BROWSER=1
   ./harness/check_wasm_package.sh` · `cargo check -p lua-vm --target
   wasm32-unknown-unknown` · `cargo run -p omnilua-cli --` smoke (`-e
   'print(_VERSION)'` under OMNILUA_VERSION=5.1 and LUA_RS_VERSION=5.1).

### R2 — playground (Opus, isolated worktree, branch `rebrand/playground`)

Rebuild `index.html` (single static file, no build step, loads the wasm
artifact static.yml builds at the NEW `packages/omnilua/` path) into a
destination:
1. **The hero demo: one snippet, five Luas.** Editor + "Run on all versions"
   → five output columns (5.1→5.5). Ship a default snippet that genuinely
   diverges across versions (integer division, `\u{...}` escapes, goto,
   `__gc` on tables, integer/float `tostring`) so the first paint shows the
   product's uniqueness without a click.
2. Single-version mode with a version selector for normal play.
3. Permalinks: encode source + version selection in the URL fragment
   (base64), restore on load. No backend.
4. Examples gallery: ~6 curated snippets (version-divergence showcase,
   metatables/OOP, coroutines, string patterns, a tiny game-loop-ish demo,
   sandbox/pcall) loadable with one click.
5. Branding per copy deck; title "omniLua playground — every Lua, everywhere";
   link to repo + npm + crates; the honesty line in the footer.
6. Design: clean dark single-page, system fonts or one webfont max, fast
   first paint, keyboard-runnable (Cmd/Ctrl+Enter), readable on mobile. No
   frameworks; vanilla JS. Keep total page weight (excl. wasm) tiny.
7. Validation: run the build script, serve locally, exercise via node/DOM
   smoke (or document manual steps); supervisor does a live browser review
   after deploy.

### R3 — docs/copy (Opus, isolated worktree, branch `rebrand/docs`)

1. README rewrite, structure: H1+tagline → wedge paragraph → 30-second
   try-it (playground link, cargo install omnilua-cli, npm i omnilua) →
   receipts (suite badges/claims per version, bench dashboard link,
   safety-tax link) → embedding example (`use omnilua`) → honesty/limits →
   versions table → contributing/license. Badges updated to new crate names
   and repo path (they 404 until first 0.1.0 publish — note that in the PR).
2. CONTRIBUTING.md / RELEASING.md prose identity (RELEASING content edits are
   R1's; R3 only touches branding prose if needed — coordinate via name map,
   don't both edit the same sections).
3. CLAUDE.md (repo) + `../CLAUDE.md` (tree): update the project-identity
   lines (what the artifact is called publicly); leave operational content
   and historical references alone.
4. CHANGELOG: new `### Changed` entry under Unreleased announcing the rename
   (old names → new, env var policy, 0.1.0).
5. `packages/omnilua/README.md` (npm-facing): standalone copy for web devs —
   assume the reader knows JS, not Rust.

## Sequencing

R0 (supervisor) immediately — rename first so every packet writes final URLs.
R1, R2, R3 launch in parallel (file-disjoint: R1 = Cargo/workflows/harness/
packages; R2 = index.html; R3 = *.md). Merge order R1 → R2 → R3 with
supervisor review each; Pages redeploys on merge; supervisor browser-reviews
the live playground; R4 release PR last; tag push handed to the user.
