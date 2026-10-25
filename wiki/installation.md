# Installation acceptance

The public [first-success fixture](../examples/first-success/README.md) is original, MIT-licensed synthetic input. It is available in the source checkout, not the npm tarball. It establishes a repeatable parser/bundle path, not Figma editability, model quality or whole-design visual accuracy.

## Three separate evidence layers

- **A: installer logic.** `npm/cache.test.js` uses the existing VM injection with controlled tiny responses and extraction results. It checks cold/warm caches, HTTP/checksum/extraction/missing-tar failures, successful retry, preservation of an older valid cache and deterministic concurrent first use. These are branch tests, not real release-download evidence.
- **B: candidate artifacts.** `scripts/build-dist.sh` builds release-format archives in `Dockerfile.dist`'s digest-pinned toolchain. `scripts/pack-candidate.py` stages maintained resources into a new temporary package and runs real `npm pack`, not dry-run. `candidate.json` binds the commit, dirty state, source digest, tarball hash, archive hashes and native binary hashes. The installation check rejects another commit or different bytes.
- **C: published artifacts.** Not exercised by the candidate job. It never creates a tag, publishes, or downloads an older release just because its version string matches. A published-asset check requires a separately selected existing release/version; candidate passes are not evidence for that layer.

The test-only `scripts/candidate-fetch.cjs` intercepts the installed wrapper's two exact release-asset URLs using Node's preload mechanism. It serves the candidate archive and its real SHA256SUMS from disk, rejects every other URL, and counts requests. The wrapper remains unmodified: checksums, actual system `tar`, staging, atomic publication and subprocess startup all run. The preload is not packaged, changes no product configuration and provides no checksum bypass.

## Run candidate acceptance

Build into a new output directory, using targets matching the machines that will consume the artifacts. For local macOS arm64, for example:

```sh
docker compose run --rm \
  -e FIGLENS_TARGETS=aarch64-apple-darwin \
  -e FIGLENS_DIST_DIR=dist/candidate-local-new \
  dist scripts/build-dist.sh
python3 -B scripts/pack-candidate.py dist/candidate-local-new
python3 -B scripts/check-installation.py \
  --artifacts dist/candidate-local-new --commit "$(git rev-parse HEAD)" --pi-version 1.4.0
```

The output directory must be run-owned; the packer refuses an existing candidate manifest. Default release scripts still write `dist/`. `prepare-npm.cjs [new-stage-directory]` can populate an isolated stage; omitting the argument keeps the existing `npm/` staging behavior. No global tools are installed.

The consumer check requires Python 3.9+ and the selected Node distribution with its adjacent npm CLI. It installs the real tarball offline into a fresh consumer with separate HOME, npm configuration/cache, native cache, Pi agent directory and temporary outputs. No repo node_modules, user credentials, global figlens or existing binary cache is used. An optional pinned Pi SDK is installed from npm into that consumer, with lifecycle scripts disabled; the loader/resource checks do not require those scripts or a paid model.

It verifies the exact package file allowlist, cold installation, warm reuse with no additional fetch, real concurrent first starts, and cached native SHA-256. It then reuses `first-success-smoke.py` for version, frame and fully checked bundle contents. `mcp-smoke.py` starts real native and installed-wrapper children (including a cold wrapper cache), sends initialize/initialized/tools-list/tool-calls over stdio, checks response IDs and content, rejects stdout logs and verifies recovery after an ordinary missing node. The same stdio harness checks numeric preflight/error codes and retained legacy calls. `check-gcs-snapshot.py` adds isolated mock generation/refresh, changed-content, CLI/MCP diagnostic redaction and restricted-root checks; it never contacts GCS. CLI smoke commands own a POSIX session and kill its process group on timeout; Windows uses `taskkill /T /F`. Output pipes are drained and the direct child is reaped before the timeout propagates. MCP stdin closes at shutdown, with owned-process cleanup on timeout. Only run-created temporary resources are removed.

`check-pi-integration.mjs` loads the installed extension through the installed Pi SDK's actual loader and runtime registry. It checks commands, tools, session-start MCP registration, public-frame reads through the adjacent wrapper, and skill/workflow discovery. The candidate check also connects to the exact registered MCP command. Scripted message sinks capture help/plan text; no model or interactive Pi session is claimed. Viewer sockets and browser reconstruction are outside this acceptance path.

## CI matrix and boundaries

`.github/workflows/test.yaml` retains existing Rust/Docker/Windows and viewer checks. A single candidate build produces Linux x64 musl, macOS arm64 and Windows x64 GNU archives plus one npm tarball. All installation jobs download that same artifact set and compare its commit with their checkout. Job timeouts and read-only repository permissions apply; no release secrets or `pull_request_target` are involved.

| Runner | Node | Pi | Purpose |
|---|---|---|---|
| Ubuntu x64 | 18.0.0 | Not run: runtime unsupported | Exact CLI/npm engine floor |
| Ubuntu x64 | 22.19.0 | 1.3.0 | Exact Pi Node floor and minimum supported SDK |
| macOS 15 arm64 | 24 | 1.4.0 | Representative Node/SDK on macOS |
| Windows x64 | 24 | 1.4.0 | Real release-format extraction/startup and Pi loader |

The CLI package declares Node >=18; Pi 1.3.0 and 1.4.0 each declare >=22.19.0. The private workspace still declares >=22; clarify that to >=22.19.0 if it is intended to describe Pi SDK development. This batch does not raise either manifest's engines to avoid failures. macOS Intel, Linux arm64, Windows arm64 emulation, and the full OS × Node cross-product are not covered by this small installation matrix. See GitHub's [runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) for the architecture of each label. CI runs are separate evidence: local successes do not establish a hosted job pass.

Run focused logic checks with `(cd npm && node --test)` and `python3 -B -m unittest discover -s scripts -p '*_test.py'`. Keep a local `listen EPERM` viewer restriction reported as blocked; do not relax security or rerun that operation elsewhere to manufacture a pass. The installation path uses stdio and test-only fetch injection, not an HTTP test server.
