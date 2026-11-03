---
name: figlens-release
description: Prepare, review and publish a figlens release through the Release Please PR, the draft release and the manual Publish Release workflow. Use when the user says "release", "prepare a release", "cut a release", "please release", or asks to publish a new figlens version.
argument-hint: "[version]"
metadata:
  targets: [claude, universal, pi]
---

Prepare or review a figlens release. A version argument is the requested version, not permission to commit, merge, push or publish.

Before acting, run `python3 scripts/ai-context.py release`. That topic is the source of truth for the automation; this skill is only the order of steps.

Identify the authorized scope: preparation or publication. An explicit request to publish includes the necessary Release PR merge and Publish Release dispatch; continue within that scope without asking again. Bypassing required reviewer approval needs explicit authorization for that bypass.

## 1. Find the pending release

Inspect Git status and preserve unrelated work. From the repository root:

```sh
gh release list --limit 3
gh pr list --label 'autorelease: pending' --state open
```

The open Release PR holds the next version and its CHANGELOG entry. No PR: check the latest "Release Please" run (`gh run list --workflow release-please.yml --limit 3`) and the repository setting that lets Actions create pull requests. Dispatch the workflow only when the user authorizes it.

Requested version: with the user's authorization for a direct push, push an empty `chore(release): request X.Y.Z` commit with a `Release-As: X.Y.Z` footer to `main` (see the `release` topic), then confirm the Release PR moves to that version. The footer covers only that release; nothing is removed afterwards.

## 2. Review the Release PR

- The version follows the policy in the `release` topic; a breaking change needs migration notes in the CHANGELOG.
- `Cargo.toml`, `Cargo.lock`, `npm/package.json` and the host plugin manifests carry the same version (`python3 -B -m unittest scripts/host_plugins_test.py`).
- The tracking manifest agrees with that version.
- Every user-visible change in `<last tag>..origin/main` has a CHANGELOG entry. Release Please skips a squash commit whose title is not Conventional (`clipboard: ...` rather than `feat(clipboard): ...`). List the commits it skips:

  ```sh
  git log --format='%h %s' v<last>..origin/main | grep -Ev '^[0-9a-f]+ (feat|fix|perf|docs|chore|ci|test|refactor|build|style|revert)(\(.+\))?!?: '
  ```

  For each user-visible one, append the Conventional message to the merged PR's body, one entry per change, separated by blank lines:

  ```text
  BEGIN_COMMIT_OVERRIDE
  feat(<scope>): <summary> (#<pr>)
  END_COMMIT_OVERRIDE
  ```

  Then run `gh workflow run release-please.yml --ref main`, wait for it, and reread the Release PR's version and CHANGELOG.
- Every user-visible `feat` and `fix` is described wherever users read about figlens. A quick check is `rg -il '<feature term>'` over these places:
  - the lead of `README.md` and every root `README-*.md`: the feature bullets, and the setup line for a new way to get a `.fig`
  - the reference in `docs/README.md` and every `docs/README-*.md`
  - `npm/README.md` and `pi/README.md`
  - the site landing page (`site/src/data.ts`, `site/src/pages/index.astro`)
  - `skills/figlens/SKILL.md` and the wiki topics

  Fix gaps in a docs PR on `main` before merging the Release PR. The site deploys only at Publish Release, so a fix merged before the release ships with it.
- CI on `main` and the exact Release PR head is green. Bot-created PR runs wait as `action_required`. Find the run with `gh run list --branch <release branch> --limit 1`, approve it with `gh api -X POST repos/runkids/figlens/actions/runs/<id>/approve`, then watch it.

Report the version, the changelog entries and anything missing. Within the authorized scope, merge the reviewed head (`gh pr merge <n> --squash --match-head-commit <sha>`). If a required review blocks it, obtain a reviewer approval or explicit permission for an admin merge; keep all CI gates intact.

## 3. Check the draft

After the merge, follow the run until the "package" job finishes (`gh run watch`). Then:

```sh
gh release view v<version> --json isDraft,assets --jq '.isDraft, (.assets[].name)'
gh release download v<version> --dir samples/release-v<version> && (cd samples/release-v<version> && shasum -a 256 -c SHA256SUMS)
```

Confirm the tag points to the exact tested merge SHA and its Cargo/npm versions agree. Expect five archives (macOS arm64/x86_64, Linux musl arm64/x86_64, Windows zip) plus `SHA256SUMS`. Check each archive contains its executable and the tag's complete `skills/figlens/` directory, including `SKILL.md`, `references/audit.md`, `references/planning.md`, `references/planning-details.md`, `references/yolo.md` and `references/search.md`. Unpack the archive for this machine and run `figlens --version` and `figlens tree <a sample .fig> --depth 1`; for a bug fix, exercise the reported behavior with the packaged binary. Private designs and outputs stay in ignored `samples/`.

## 4. Write the release notes

Follow the release notes rules in the `release` topic. Write English user-facing copy to `samples/release-v<version>/notes.md`: the concrete problem, resulting behavior, a useful example or update command, and relevant verification or limitations. Scale the detail to the change. Review claims against the pending commits and actual checks; the generated CHANGELOG is input, not the finished copy.

Apply it to the completed draft before publication:

```sh
gh release edit v<version> --title v<version> --notes-file samples/release-v<version>/notes.md
gh release view v<version> --json name,body,isDraft
```

Verify the title is exactly `v<version>` and the body matches the notes file. Recheck both after a draft rebuild and restore the reviewed copy if needed. Keep the notes and private artifacts ignored; do not force-add them. Draft social announcements only when requested.

## 5. Publish and verify distribution

With publication authorized, run `gh workflow run publish-release.yml -f tag=v<version>`. Use this workflow so GitHub, Homebrew and npm all run. Watch it to completion and inspect each job. Investigate a failed channel before retrying; preserve the tag and version.

Verify the public result:

- GitHub marks the intended tag as latest, with the version-only title and reviewed body.
- Homebrew's published formula has the version and archive hashes from `SHA256SUMS`.
- npm exposes the requested version as `latest`, with both Pi extension and skill metadata. If npm reports processing and the registry still returns 404, wait and recheck availability; do not republish.
- Run `npx --yes @runkids/figlens@<version> --version` using isolated npm and binary caches under `samples/` (`npm_config_cache` and `XDG_CACHE_HOME`) so the check exercises the public package and download path.

Report the release link, shipped behavior, actual checks and each distribution result. Distinguish unpublished drafts, successful publication and pending or failed channels. Finish only after the authorized steps are complete or a concrete blocker is reported.
