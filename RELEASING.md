# Releasing

Releases are driven by pushing a `vX.Y.Z` tag. The
[`Release`](.github/workflows/release.yml) workflow then verifies, publishes
every crate to crates.io, publishes `lua-rs-wasm` to npm, and creates the
GitHub release.

## One-time setup

The workflow needs two repository secrets:

- `CARGO_REGISTRY_TOKEN` — a crates.io API token with publish scope.
  `gh secret set CARGO_REGISTRY_TOKEN`
- `NPM_TOKEN` — an npm automation token for `lua-rs-wasm` (already set).

## Cutting a release

1. Open a version-bump PR: set the new version in `Cargo.toml`
   (`[workspace.package]` and every `[workspace.dependencies]` entry),
   `crates/lua-rs-runtime/README.md`, and `packages/lua-rs-wasm/package.json`.
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
   - **publish-wasm** — builds and publishes `lua-rs-wasm@X.Y.Z` to npm.
   - **github-release** — creates the GitHub release with generated notes (only
     if one does not already exist for the tag).

The crate publish order encodes the dependency graph; `lua-rs-derive`'s only
internal dependency on `lua-rs-runtime` is a path dev-dependency, which cargo
strips on publish, so there is no real cycle.
