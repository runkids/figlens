# Release and distribution

Load this for packaging, publishing a version, the install paths and `figlens update`.

## Release

- **Version:** Release Please owns it. Every push to `main` updates the Release PR (`.github/workflows/release-please.yml`, config `.github/release-please-config.json`, last released version in `.github/release-please-manifest.json`): from Conventional Commits it bumps `Cargo.toml`, `Cargo.lock` and `npm/package.json` together and writes `CHANGELOG.md`. Pre-1.0, `feat` and breaking changes bump minor, `fix` and `perf` bump patch; force a version with a `Release-As: X.Y.Z` footer on a commit that reaches `main`. Do not edit the manifest or the versions by hand.
- **Draft:** merging the Release PR tests the merge commit (`test.yaml`), then Release Please tags `v<version>` and creates a **draft** release with the changelog, and calls `.github/workflows/release.yaml` (a `GITHUB_TOKEN` tag starts no workflow by itself). It checks the tag against `Cargo.toml` and `npm/package.json`, then `scripts/build-dist.sh` in the `dist` compose service (`Dockerfile.dist`, base image pinned by digest; cargo-zigbuild) builds `dist/figlens-<version>-<target>.tar.gz` for macOS arm64/x86_64 and Linux musl arm64/x86_64 and `.zip` for `x86_64-pc-windows-gnu`, each with the complete `skills/figlens/` directory (entry and workflow references), plus `SHA256SUMS`, into the draft. Rerun it for an existing draft with workflow_dispatch; it refuses a published release. Binaries are unsigned. Locally: `scripts/dist.sh` (a few GB free in the Docker VM).
- **Repository setting:** Settings > Actions > General must allow GitHub Actions to create pull requests, or no Release PR appears.
- **Skill:** `figlens-release` walks an agent through these steps.
- **Host integrations:** the existing `@runkids/figlens` npm package carries independent Pi, OpenCode V2 and OMP entries plus the shared skill. Before publishing, `scripts/prepare-npm.cjs` stages maintained `pi/`, `opencode/`, `omp/`, `skills/` and the root `.mcp.json` into ignored npm copies. Keep the CLI root CommonJS, host module boundaries and SDK imports independent; do not publish a second package or edit generated copies. All entries use the same native launcher/version/cache. The private root supports source discovery/tests. CI retains CLI/Pi floors and adds fixed OpenCode/OMP installed-host checks using the same candidate artifact. See [integrations](integrations.md) and [installation acceptance](installation.md) for manifests, permissions, current runtime evidence and untested boundaries.
- **Publish:** check the draft, then run "Publish Release" (`publish-release.yml`, workflow_dispatch with the tag): it verifies the assets against `SHA256SUMS`, publishes, then runs `update-homebrew.yml` (formula from `scripts/formula.sh` pushed to `runkids/homebrew-tap`) and `npm publish` of `npm/`. figlens is not on crates.io (`publish = false`): one token fewer to keep.
- **Secrets:** `HOMEBREW_TAP_SSH_KEY` (private half of a write deploy key on runkids/homebrew-tap; replace both halves together). npm has no secret: a Trusted Publisher on npmjs.com (`runkids/figlens`, `publish-release.yml`, npm publish allowed) lets the `npm` job publish through OIDC, with provenance.
- **Pinned toolchain:** shipped binaries build from `Dockerfile.dist`'s digest-pinned base. To move Rust, change the tag and digest together (`docker buildx imagetools inspect rust:<version>`), and the toolchain in the Windows test job.

## Release notes

- The GitHub release title is exactly `v<version>`, with no feature name or subtitle.
- The release body is reviewed English copy for users, separate from the generated CHANGELOG. Explain the concrete problem and resulting behavior; include useful examples or update commands and relevant verification or limitations. Avoid internal implementation details unless users need them.
- Keep the source copy at ignored `samples/release-v<version>/notes.md` and apply it to the completed draft before Publish Release. That workflow preserves the draft body. The `figlens-release` skill contains the commands and read-back checks.
- Verify every claim against the release range and executed checks. Keep private designs, exports and local-only report links out of public notes; preserve contributor credits and provide migration guidance when applicable.

## Install paths

- macOS/Linux: `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh | sh` (latest release via the `releases/latest` redirect, checked against `SHA256SUMS`, into `~/.local/bin` or `$FIGLENS_BIN_DIR`).
- Windows: `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 | iex` (into `%LOCALAPPDATA%\Programs\figlens`, added to the user PATH; ARM64 runs the x64 build).
- Homebrew: `brew install runkids/tap/figlens`. npm: `npx @runkids/figlens ...` (npm refuses the unscoped `figlens` as too close to `figlet`; the command is still `figlens`) (`npm/bin/figlens.js` downloads the release archive for its own version on first run, checks it against `SHA256SUMS`, caches it under the user cache dir; no install script).
- `figlens update` reruns the matching install script into the running binary's directory; on Windows the old exe is renamed aside, since a running exe cannot be overwritten.
- The repo and its releases must be public for every path above; while the repo is private none of them can download.
