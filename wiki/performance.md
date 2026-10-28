# Bounded local baseline

Use `scripts/measure-baseline.py` to collect a small reproducible baseline before proposing cache changes. It accepts an explicit binary and defaults to the original MIT public fixture (`examples/first-success/design.fig`, 1,615 bytes). Explicit `--fixture`/`--node`, `--alternate`/`--alternate-node` and `--frames ID ID ID` select approved local inputs only; acquire permission and execution limits before using private samples. It creates and removes its own temporary HOME/cache/output directories. It does not flush the OS file cache or contact GCS.

```sh
python3 -B scripts/measure-baseline.py --binary /absolute/path/to/figlens --repetitions 10
```

Rust builds/runs for development stay in Docker. For a release-profile Linux test binary:

```sh
docker compose run --rm dev cargo build --release --locked --bin figlens
docker compose run --rm dev python3 -B scripts/measure-baseline.py --binary /work/target/release/figlens --repetitions 10
```

The report records HEAD and dirty state, platform, explicit binary path/version/hash, fixture size/hash, parameters, all samples, median and nearest-rank p95. Record the build command too: a supplied binary's version alone cannot prove its source. Repetitions are bounded to 3–30. These are sequential observations, not a load test.

## Workload and memory boundaries

The script runs fresh CLI nodes and MCP initialization/first call for each repetition (default 5), then 10 warm tree/inspect/nodes calls per server. With an alternate input, A/B/A calls assert A returns the original JSON. With three frame IDs, sequential CLI, batch jobs 1/2/default and MCP bundles run three times each; entry bytes and manifest hashes must match by node ID, ignoring only `createdAt`. Every child command/request has a 60-second limit. Output includes raw timings, median/p95, serialized response sizes, per-child peak RSS via Unix `wait4`, and Linux MCP live RSS/lifetime high-water observations. The container section records cgroup current/lifetime peak and CPU/memory limits when available; it includes the Python harness, page cache and all children, not just the product. CLI timings include up to 1 ms wait polling. MCP peaks belong to the server lifetime, not a particular request; missing `/proc` observations are null. These measurements do not establish retained-object lifetimes or absence of a long-term leak.

For stage evidence, the opt-in test `bundle::probe::stages` reads `FIGLENS_PROBE_FILE`/`FIGLENS_PROBE_NODE` (public fixture by default) and writes JSON to required `FIGLENS_PROBE_OUTPUT`. Run that exact ignored test under an external timeout, using a release-profile test binary. It measures open/inflate/schema, decode/preflight, Tree, REST expansion, image reads, SVG, raster/PNG, complete bundle (inclusive), and repacking its existing entry bytes. Stages retain predecessors, and high-water is cumulative. Repacking uses probe metadata; it isolates zip/hash/manifest cost, not another complete bundle. Never subtract inclusive timings to claim pure parser time. Repeat fresh processes and summarize raw stage samples before drawing conclusions.

Record Docker image ID, CPU/memory limits, source commit/diff and binary hash alongside reports. Compare the same approved workload before and after a change. Investigate timing increases over both 5% and 1 ms, or RSS increases over both 5% and 16 MiB; these are review thresholds, not noise guarantees.

## ZIP opening

All three ZIP-open paths (document, render images and preview assets) now share a buffered opener that first uses `ArchiveOffset::Known(0)`, avoiding zip 9.0.5's full-file forward scan. The 2026-10-26 Docker Linux aarch64 measurements on 41/60/100 MB designs were 26/37/58–62 ms per open with `Detect`, versus 0.1–0.3 ms with `Known(0)`; these isolate ZIP opening, not total parsing or rendering. If the fixed-offset attempt fails, the helper reopens with `Detect` so archives with prepended bytes still work.

## Observed local run

2026-10-26, HEAD `3811c4034447a5518cb8dae0c13de8ba5c167402` **plus batch-5 uncommitted changes**, Docker Linux aarch64/glibc 2.41 on OrbStack, Python 3.13.5; Rust 1.97.1 release profile, `--locked --bin figlens`, no architecture/cache changes. Binary SHA-256: `267547a42aef07600ff289c01170fabb0e0ff09b9324e3d1f9e409a8d0a4c778`. Fixture hash is recorded in its [provenance](../examples/first-success/README.md). Ten repetitions, OS cache not flushed:

| Observed boundary | Median ms | p95 ms |
|---|---:|---:|
| Fresh `--version` process (includes spawn/version handling) | 0.906 | 3.662 |
| Fresh `nodes design.fig 0:1` (spawn + parse + tool/output) | 1.187 | 3.088 |
| MCP spawn + initialize | 0.826 | 0.975 |
| First MCP `nodes`, fresh server parsed state | 0.355 | 0.465 |
| Next MCP `nodes`, same local file/server | 0.157 | 0.192 |

The raw local report is retained in the ignored run directory `samples/batch5-review-l8khg07w/baseline-linux-arm64-final.json`; a fresh checkout can rerun the script. These boundaries do **not** isolate pure parser time, tool time or IPC by subtraction. Scheduling, process setup and OS cache state affect them. No speed claim follows from this tiny example.

Source inspection confirms local MCP parsed-tree reuse; GCS calls reuse same-generation **downloads** but bypass that parsed-tree cache. The [GCS mock](gcs.md#evidence-and-limitations) confirms six metadata checks and two downloads across repeated/new/failed/recovered calls; these are counts, not cloud latency measurements.

That batch had no representative licensed large design, real metadata/download timing, image-heavy render profile, cross-platform benchmark or external-client measurement. Keep the current cache architecture for this batch. Gather those measurements before deciding whether remote parsed-tree caching is worth its invalidation/ownership costs. Generation-pinned download/provenance is a separate correctness follow-up, not a measured performance fix.


## Compact Kiwi byte arrays (2026-10-26)

Compared main `a1610d69ea964a164ac4c8a2e32c43edcfa184e3` with compact byte arrays, using the same release-profile stage probe, Docker Linux aarch64/glibc and Rust 1.97.1. Build: `docker compose run --rm dev cargo build --release --locked --bin figlens --example stages`. Five fresh processes per binary, no frame arguments, OS page cache not flushed. The ignored probe was copied temporarily into `examples/stages.rs` and removed after measurement.

The community-design sample has 17,995,555 decompressed message bytes, 26,880 nodes and 5,009 blobs. Medians:

| Boundary | Before | After |
|---|---:|---:|
| `treeNew` VmHWM (KiB) | 1,022,772 | 574,304 |
| VmHWM / decompressed message bytes | 58.20x | 32.68x |
| `kiwiDecodeOnly` (ms) | 211.55 | 165.95 |
| `documentMessageTotal`, including preflight (ms) | 245.45 | 197.29 |

Byte arrays now use `Vec<u8>` rather than one 72-byte `Value` per byte. Decoded high-water RSS fell 43.8%; raw decode time fell 21.6%. This is a local observation, not a cross-platform guarantee. The unchanged probe's later `countValues` walk does not recognize `Bytes`; only its earlier decode/tree stages are used here.

Compatibility was measured on the public first-success fixture and two different held-out community designs (2,250 and 4,719 nodes), not the performance sample. `nodes` JSON was byte-identical. Bundle entry names and contents were identical (9, 39 and 8 entries), except `manifest.json`'s `createdAt`. No exports or design files are distributed. These checks establish output preservation, not improved accuracy against new Figma exports.

## Baseline after #23/#24, 2026-10-26

Re-ran the ignored `samples/perf-2026-10-26/perf.py` harness on clean main `4ec0f368825a19a6d2191744b4897cf11df0b3df` (only the temporary stage example was untracked), against the supplied `results.json` baseline (metadata records `a1610d6`, not the requested `7847b9d`). Same five local designs and frame IDs, Docker Linux aarch64/OrbStack, release build, 18 CPUs, OS page cache not flushed. Five fresh-process loads, ten calls per warm MCP method; ZIP/RSS probes and three-frame bundle routes each ran once. Build: `docker compose run --rm dev cargo build --release --locked --bin figlens --example stages`. The probe mirrors the current ZIP opener and counts compact `Bytes`. Old → new:

| File | CLI first median ms | MCP first median ms | ZIP open ms | `treeNew` VmHWM KiB | Three-frame batch wall ms |
|---|---:|---:|---:|---:|---:|
| small | 40.89 → 35.56 | 34.11 → 29.00 | 0.414 → 0.153 | 98,516 → 75,196 | 882 → 937 |
| medium | 8.63 → 2.59 | 7.33 → 1.39 | 6.047 → 0.105 | 6,400 → 5,488 | 838 → 867 |
| large | 154.61 → 93.15 | 134.62 → 76.64 | 25.860 → 0.328 | 445,432 → 200,832 | 479 → 393 |
| xl | 397.35 → 306.00 | 344.70 → 250.69 | 35.058 → 0.154 | 1,022,744 → 574,256 | 543 → 439 |
| xxl | 185.57 → 104.70 | 165.59 → 85.88 | 58.254 → 0.282 | 360,868 → 193,664 | 319 → 234 |

First load is `tree --depth 1`; batch uses default jobs. VmHWM is the probe's process high water through `treeNew`, including an earlier separate decode, not Tree allocation alone. Warm medians (tree/inspect/find/nodes, ms): small 0.062/7.428/1.127/8.585; medium 0.035/0.271/0.056/0.320; large 0.049/1.779/1.172/1.854; xl 0.085/7.145/4.617/8.156; xxl 0.109/8.021/1.840/8.972. Flagged >1.3×: xxl warm tree 0.069 → 0.109 ms (1.59×). Single-probe stage timings also include regressions, notably xxl's third-frame SVG 17.02 → 39.87 ms and PNG 36.33 → 62.55 ms; n=1 cannot establish a causal regression.

All six scenarios completed. Overwrite and mtime-only changes reloaded; unreadable/deleted copies returned `source_unreadable`, stayed responsive and recovered. All 15 frame bundles matched across sequential CLI, both batch modes and MCP after timestamp normalization. Raw results, full old/new tables, all >1.3× stage observations, binary hash and reproduction command are retained locally in `samples/perf-2026-10-26/results-4ec0f36.json` and `remeasure-4ec0f36.md`. No new Figma accuracy or cross-platform claim follows.

## Pure-blur filter intermediates (2026-10-26)

Compared main `d7275d6` (including #27) with raster-only removal of unused hard-alpha and single-input merge primitives. Docker Linux aarch64, Rust 1.97.1 release, resvg 0.48.1; actual CLI batch commands, five private files / fifteen frames, three fresh processes per configuration, alternating before/after, OS cache not flushed. One frame per command; walls include startup/decode. `--jobs 1` limits frames, not internal rendering; `RAYON_NUM_THREADS` is unset in both modes.

| Frame | `--jobs 1` before → after (ms) | Default before → after (ms) | Default VmHWM median before → after (MiB) |
|---|---:|---:|---:|
| small `20:2` | 906.1 → 823.9 (−9.1%) | 894.1 → 821.7 | 187.9 → 154.4 |
| medium `7:346` | 830.4 → 823.9 | 820.4 → 825.6 | 510.9 → 510.1 |

Earlier primitive profiling on `4ec0f36` attributes medium's cost mainly to GaussianBlur (586.16 ms across reference and two 2x composites), ColorMatrix (102.43) and Merge (93.18); its shadow graphs have no removable intermediates. Small's corresponding costs were 488.05 / 61.06 / 60.99 ms; removing unused intermediates saved 74.35 ms in the isolated SVG experiment. Tightening shadow regions changed pixels and was rejected. Small's 535 ms target was not reached; medium already meets 1450 ms on current main because #27 retains rendering parallelism, not because of this change. Medium default after VmHWM was 510.1 / 511.3 / 510.1 MiB; do not attribute its noisy memory to this optimization. Larger decoded-tree samples still exceed 510 MiB.

No other frame regressed over 10%. All 90 before/after archive pairs had identical entry names and bytes except the manifest, including SVG assets and PNG references/composites. This establishes preservation, not improved Figma accuracy. The earlier single-thread stage baseline is retained separately: `RAYON_NUM_THREADS=1` must not be described as current `--jobs 1`. Raw profiles and paired CLI observations remain in gitignored `samples/perf-2026-10-26/filter-work/`; no private fixtures are distributed.

## Early MCP nodes budget (2026-10-29)

`scripts/measure-expansion.py --binary B --fixture F NODE...` records CLI `nodes` elapsed time, child peak RSS, output bytes, node/expanded-node counts and instance nesting per scope; run it inside `docker compose run --rm dev` like the baseline. Across every canvas of four private designs, the largest canvas was 188 MiB pretty JSON (52 MB compact), 34,966 nodes, 18,629 expanded, nesting 3, 3.0 s and 2.4 GiB peak (loading alone: 0.5 s, 807 MiB). A temporary stage probe split the build: expansion 223 ms / +130 MiB, REST builder about 1.9 s / +1.15 GiB, compact serialization 50 ms.

Release build, Docker Linux aarch64, one run each, OS cache not flushed. On a warm MCP server, rejecting `nodes` on that canvas went from 2,383 ms and a lifetime high-water of 895 → 2,465 MiB to 545 ms and 895 → 1,041 MiB; the 414-byte `response_budget_exceeded` reply is unchanged. `inspect` of the same scope still succeeds (4,119 bytes) and builds the full selection. Eighteen fitting scopes from two held-out designs returned MCP JSON equal to CLI `nodes`. Raw observations are in ignored `samples/expansion-61/`. Expansion under a stored root is bounded too: a synthetic six-level, ten-way fan-out (1,111,110 copies) stops after 191 ms and 112 MiB high-water instead of 1,751 ms and 886 MiB for expansion alone; real canvases are unaffected.

## Historical batch timings

Historical runs: 24 frames: 28.6 s as separate processes, 8.1 s with `--jobs 1`, 2.9 s with 4; 96 frames: 32.3 s / 12.2 s / 8.5 s with 1 / 4 / 12 jobs, peak memory 1.6 / 1.9 / 2.6 GiB (the decoded `.fig` is about 1.2 GiB of it).

## Partial semantic metadata (2026-10-26)

Compared clean `eb01fe3` with additive effective layout/property semantics and render diagnostics using fixed release binaries. Authorized Aurora plus held-out DashArt and the public fixture; Docker aarch64, 4 CPU/4 GiB, five first calls, ten warm calls per tool/server, three runs per three-frame bundle mode, 60-second child/request limit, no OS cache eviction. A reversed-order repeat checked initial threshold flags. Warm Aurora nodes consistently grew: 7.44→8.96 ms initially, 7.46→8.77 ms on repeat (+17.7%, +1.32 ms); the MCP envelope grew 183,370→286,574 bytes (+56.3%) with the new fields and provenance. This records an additive cost, not an isolated serialization attribution or speedup. Initial bundle/first-call flags did not persist; no peak rose by both 5% and 16 MiB. Public and held-out medians stayed below the >5% AND >1 ms review threshold.

Nine selected frame/page/instance/component/set cases preserved existing fields and asset/reference bytes. CLI sequential, batch jobs 1/2/default and MCP bundles matched within each binary. Same-version Figma calibration is still absent. Fixed binary hashes, raw first/repeat observations, source-field evidence and limits are retained in ignored `samples/semantic-work/BATCH12.md`.

### Post-review verification (2026-10-26)

The reviewed candidate preserves swapped-child placement absence, includes drawn stroke/mask diagnostics, and excludes provenance from asset planning. Nine-case existing-field and asset/reference comparisons still pass. Two bounded Aurora runs (final→baseline, then baseline→final) observed CLI nodes medians +6.9–11.2%, first MCP nodes +10.5–16.7%, and warm nodes +0.75–2.40 ms; the nodes envelope remains +56.3%. Bundle timings were inconsistent: first-run medians ranged from −8.6% to +4.6%, while the repeat ranged from +3.2% to +27.9%. These observations do not isolate the cost of the fixes or establish a stable bundle regression; they do rule out claiming a general speedup. No peak crossed both the 5% and 16 MiB thresholds. Final binary `a70c93993d52fb8acadac042e70c3d55674c90a7dab0383360b83dd9c83ed8ef`, raw runs and comparisons are retained under ignored `samples/semantic-review/`; unrelated concurrent changes are excluded.
