# Bounded local baseline

Use `scripts/measure-baseline.py` to collect a small reproducible baseline before proposing cache changes. It accepts an explicit binary and uses only the original MIT public fixture (`examples/first-success/design.fig`, 1,615 bytes). It creates and removes its own temporary HOME/cache/output directories. It does not flush the OS file cache or contact GCS.

```sh
python3 -B scripts/measure-baseline.py --binary /absolute/path/to/figlens --repetitions 10
```

Rust builds/runs for development stay in Docker. For a release-profile Linux test binary:

```sh
docker compose run --rm dev cargo build --release --locked --bin figlens
docker compose run --rm dev python3 -B scripts/measure-baseline.py --binary /work/target/release/figlens --repetitions 10
```

The report records HEAD and dirty state, platform, explicit binary path/version/hash, fixture size/hash, parameters, all samples, median and nearest-rank p95. Record the build command too: a supplied binary's version alone cannot prove its source. Repetitions are bounded to 3–30. These are sequential observations, not a load test.

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

There is no representative licensed large design here, and no real metadata/download timing, image-heavy render profile, cross-platform benchmark or external-client measurement. Keep the current cache architecture for this batch. Gather those measurements before deciding whether remote parsed-tree caching is worth its invalidation/ownership costs. Generation-pinned download/provenance is a separate correctness follow-up, not a measured performance fix.


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
