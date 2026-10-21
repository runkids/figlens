---
name: figlens-release
description: Prepare, review and publish a figlens release through the Release Please PR, the draft release and the manual Publish Release workflow. Use when the user says "release", "prepare a release", "cut a release", "please release", or asks to publish a new figlens version.
argument-hint: "[version]"
metadata:
  targets: [claude, universal, pi]
---

Prepare or review a figlens release. A version argument is the requested version, not permission to commit, merge, push or publish.

Before acting, run `python3 scripts/ai-context.py release`. That topic is the source of truth for the automation; this skill is only the order of steps.

## 1. Find the pending release

```sh
gh release list --limit 3
gh pr list --label 'autorelease: pending' --state open
```

The open Release PR holds the next version and its CHANGELOG entry. No PR: check the latest "Release Please" run (`gh run list --workflow release-please.yml --limit 3`) and the repository setting that lets Actions create pull requests. Dispatch the workflow only when the user authorizes it.

Requested version: put a `Release-As: X.Y.Z` footer on the next commit to `main` (blank line before it; copy it into the squash message on a squash merge), then confirm the Release PR moves to that version.

## 2. Review the Release PR

- The version follows the policy in the `release` topic; a breaking change needs migration notes in the CHANGELOG.
- `Cargo.toml`, `Cargo.lock` and `npm/package.json` carry the same version.
- Every user-visible `feat` and `fix` in `<last tag>..origin/main` is reflected in `README.md`, `skills/figlens/SKILL.md` and the wiki topics; fix stale docs on `main` first, then let the PR update.
- CI on `main` is green.

Report the version, the changelog entries and anything missing. Merge only when the user says so (`gh pr merge <n> --merge`).

## 3. Check the draft

After the merge, follow the run until the "package" job finishes (`gh run watch`). Then:

```sh
gh release view v<version> --json isDraft,assets --jq '.isDraft, (.assets[].name)'
gh release download v<version> --dir samples/release-v<version> && (cd samples/release-v<version> && shasum -a 256 -c SHA256SUMS)
```

Expect five archives (macOS arm64/x86_64, Linux musl arm64/x86_64, Windows zip) plus `SHA256SUMS`. Unpack the archive for this machine and run `figlens --version` and `figlens tree <a sample .fig> --depth 1`.

## 4. Publish

Only on the user's explicit go-ahead: `gh workflow run publish-release.yml -f tag=v<version>`. It publishes the draft, then Homebrew, crates.io and npm. Watch it to the end and report each job; a failed channel is reported, never retried blindly.
