# Releasing

Releases are driven by pushing a `vX.Y.Z` tag. The
[`Release`](.github/workflows/release.yml) workflow then verifies, publishes
every crate to crates.io, publishes `omnilua` to npm, and creates the
GitHub release.

The public crates are `omnilua` (the embedding library, directory
`crates/lua-rs-runtime/`) and `omnilua-cli` (the CLI, directory
`crates/lua-cli/`, binary `omnilua`); the npm package is `omnilua`
(directory `packages/omnilua/`). The internal crates (`lua-vm`, `lua-gc`,
`lua-types`, `lua-parse`, `lua-stdlib`, `lua-rs-hlua-shim`, etc.) keep their
names and version in lock-step with the workspace.

## One-time setup

The workflow needs two repository secrets:

- `CARGO_REGISTRY_TOKEN` — a crates.io API token with publish scope.
  `gh secret set CARGO_REGISTRY_TOKEN`
- `NPM_TOKEN` — an npm automation token for `omnilua` (already set).

## The 0.1.0 rebrand release

The first release under the omniLua names is **0.1.0**. It publishes the new
crate trio (`omnilua`, `omnilua-cli`) and the renamed npm package (`omnilua`)
for the first time, so there is no "already on crates.io / npm" entry to skip —
the first publish of each new name is the real one. The tag push is
**irreversible**: crates.io and npm do not allow re-publishing or deleting a
published version, and the name is claimed on first publish. Treat the 0.1.0 tag
push as the user's explicit, final call.

## Cutting a release

1. Open a version-bump PR: set the new version in `Cargo.toml`
   (`[workspace.package]` and every `[workspace.dependencies]` entry),
   `crates/lua-rs-runtime/README.md`, and `packages/omnilua/package.json`.
   Merge it to `main`.
2. Tag the merge commit and push the tag:
   ```bash
   git tag vX.Y.Z origin/main
   git push origin vX.Y.Z
   ```
3. The `Release` workflow runs:
   - **verify** — the tag must match the workspace version, then `make test`
     (rust + conformance) must pass.
   - **publish-crates** — `cargo publish` for all crates in dependency order.
     Idempotent: any crate already on crates.io at this version is skipped, so a
     re-run after a partial failure is safe.
   - **publish-wasm** — builds and publishes `omnilua@X.Y.Z` to npm.
   - **github-release** — creates the GitHub release with generated notes (only
     if one does not already exist for the tag).

The crate publish order encodes the dependency graph; `lua-rs-derive`'s only
internal dependency on `omnilua` is a path dev-dependency, which cargo strips on
publish, so there is no real cycle.

## Legacy crates (optional follow-up, user-triggered)

The old names `lua-rs-runtime`, `lua-cli`, and the `lua-rs-wasm` npm package are
not republished by the rebrand release. If we want existing dependents to find
the new home, the optional follow-up is to publish final **0.0.34 pointer
releases** of `lua-rs-runtime` / `lua-cli` / `lua-rs-wasm` whose only change is a
README that says "renamed to omnilua — see https://github.com/ianm199/omnilua".
These are README-only courtesy releases; they ship no code and are entirely the
user's call to trigger. They are deliberately out of scope for the 0.1.0 release
workflow above.

**Mechanics.** The live workspace no longer contains crates named
`lua-rs-runtime` or `lua-cli` — they were renamed to `omnilua` / `omnilua-cli`,
so the pointer releases **cannot** be cut from `main`. The crates.io and npm
registries currently hold the old names at **0.0.33**, so the pointer version is
**0.0.34**. Do this only **after** the 0.1.0 publish has landed (so the redirect
points at a home that already exists):

1. From a throwaway scratch checkout of the last old-name commit (the commit
   immediately before the rebrand renamed the manifests), or a minimal stub crate
   that keeps the old `name` and `version = "0.0.34"`:
   - replace the crate's `README.md` with a one-line redirect:
     `# lua-rs-runtime → renamed to omnilua — see https://github.com/ianm199/omnilua`
     (and the equivalent for `lua-cli` / `lua-rs-wasm`);
   - leave the code as-is (or stub it to a single `compile_error!`-free no-op —
     these are pointer releases, not functional ones);
   - ensure the manifest still carries `name = "lua-rs-runtime"` (resp. `lua-cli`)
     and `version = "0.0.34"`, with `description`/`license` intact.
2. `cargo publish -p lua-rs-runtime` then `cargo publish -p lua-cli` (crates.io),
   and for npm: bump `packages/.../package.json` `name` back to `lua-rs-wasm`,
   `version` to `0.0.34`, then `npm publish --access public`.
3. Verify each old name now shows 0.0.34 with the redirect README on its registry
   page. Discard the scratch checkout — none of this lands on `main`.
