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
