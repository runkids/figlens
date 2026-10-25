# MCP contract and request admission

Status: implemented. Docker Rust, stdio and POSIX permission-revocation checks pass; actual Pi registration/packaged-launcher and Windows acceptance remain required before release. The evidence sections below record the pre-implementation investigation.

[Traditional Chinese translation](mcp-contract-admission.zh-TW.md)

## Decision and scope

Deepen two existing modules, in order:

1. The tool contract module owns MCP parameter interpretation.
2. The request admission module owns the decisions required before loading a design.

The user selected both opportunities and explicitly accepted incompatible MCP changes because legacy compatibility is not needed. Remove legacy translation rather than maintaining a compatibility adapter, mode flag or migration mechanism. CLI behavior remains unchanged. This document records the approved design; implementation evidence is separate from design approval.

The objective is locality and leverage, not a performance claim or a file-size cleanup. Domain terms below follow existing code: tool, .fig, design, document, Tree, export and bundle. No new domain concept or GLOSSARY entry is required.

## Evidence

Source inspected at `bb45fed`:

| Location | Current friction |
| --- | --- |
| `src/main.rs::TOOLS` and `src/contract.rs::property` | Parameter declarations and discovery metadata are distributed. |
| `src/contract.rs::arguments` | Validates and normalizes arguments, but also classifies legacy input and leaves omitted defaults to execution. |
| `src/command.rs::Command::from_tool` | Repeats MCP defaults already advertised by discovery. |
| `src/main.rs::call_tool` | Rebuilds CLI arguments for legacy calls and adds another export default. |
| `src/main.rs::refused`, `direct_file`, `is_direct`, `mcp_reply_with` | Repeatedly parse or classify the same request before authorization, preload and execution. |
| `src/main.rs::serve` | Needs admission knowledge both outside and inside the borrowed Tree reuse loop. |

These are architectural friction findings, not demonstrated incorrect outputs. Existing tests protect much of the behavior that must survive.

## 1. Deepen the tool contract module

### Responsibility and interface

Keep the module rooted in `src/contract.rs`. Its interface exposes discovery and preparation of a named tool call. It owns parameter definitions, validation, normalization and MCP defaults. Descriptions can remain static; do not build a generic schema interpreter.

Preparation returns either a structured `Failure` or an owned prepared tool call. The prepared value carries tool identity and normalized arguments. Its fields and unchecked constructor are private to the contract module; expose only immutable, purpose-specific accessors. Do not expose mutable JSON, setters, `DerefMut`, or unchecked construction through `Default` or deserialization. Callers cannot change tool identity, defaults or arguments after validation. It exposes the source/output information needed by admission without requiring callers to revalidate raw JSON.

Use the existing JSON representation internally unless implementation demonstrates a need for more. Do not introduce a separate argument struct for every tool, a registry abstraction or a configurable validation framework. The prepared value earns its place by carrying an invariant, not by forwarding arbitrary JSON.

`Command::from_tool` must require the opaque prepared-call type, not a tool name plus bare `Value`, nor a wrapper immediately unpacked into the old unchecked interface. MCP source/output work must also consume the authoritative admitted paths described below. It retains necessary Tree-dependent checks, such as target existence and supported export roots, but must not choose MCP defaults again. Keep absence meaningful for optional values; represent unlimited depth explicitly within the prepared representation rather than making multiple callers choose its meaning. Acceptance includes a visibility/interface check that external callers cannot construct or mutate a prepared call or bypass it with bare JSON.

The seam is between raw MCP arguments and prepared calls. The CLI adapter continues to use `Command::from_cli`; both adapters retain leverage from the existing command execution module. Do not force CLI parsing or CLI defaults through the MCP contract.

### Defaults and behavior to preserve

| Parameter | Omitted value |
| --- | --- |
| `tree.node`, `find.node` | `0:0` |
| `tree.depth` | Unlimited |
| `tree.json`, `inspect.images` | `false` |
| `inspect.depth` | `1` |
| `find.limit` | `20` |
| `export.scale` | `1` |
| `export.quality` | `80` |
| `export.maxKb` | No byte budget |
| `export.maxSize` | `1600`; explicit `0` disables the side cap |
| `variables.node` and other optional strings | Remain absent |

Discovery and preparation must use the same default decisions. Preserve current ranges, mathematical JSON integer normalization (`2.0` is valid), extension handling, case-insensitive search types and successful output shapes. Design tools continue to ignore unknown fields; `workflow` continues to reject extras. Strict parameter types do not imply making every string a validated node identifier before a Tree exists.

### Intentional incompatibilities

| Former input/behavior | New behavior |
| --- | --- |
| Numeric strings, such as `depth: "0"` | `invalid_arguments`; send a JSON number. |
| Scale strings, such as `scale: "2x"` | `invalid_arguments`; send a JSON number. |
| Optional `null`, or `false` for a non-boolean parameter | `invalid_arguments`; omit only when the omission default matches the intended behavior. |
| `export.maxSize: null` or `export.maxSize: false` (formerly uncapped) | Use `export.maxSize: 0`; omission means a 1600 side cap, not uncapped. |
| Flag-like node strings interpreted as CLI options | Never CLI options; follow ordinary node-string handling and target checks. |
| MCP calls routed through `Command::from_cli` or `run` | Removed. |
| `_meta.figlens.argumentMode: "legacy"` | Removed; do not replace it with another mode marker. |

Boolean `false` remains valid for boolean parameters. CLI strings and CLI flag syntax remain supported. An invalid former legacy call under `--root` now produces `invalid_arguments` before path admission, not the former legacy-specific `permission_denied` result.

### Breaking-change communication

When implementation ships, its release notes must include intent-preserving replacements: `depth: "0"` becomes `depth: 0`; `scale: "2x"` becomes `scale: 2`; uncapped `maxSize: null` or `maxSize: false` becomes `maxSize: 0`. Omit `maxSize` only to select the 1600 default. Other optional omission sentinels must be mapped according to their actual former semantics, not a blanket instruction to omit every field. Mention removal of the legacy mode metadata and the restricted-root error change. No compatibility adapter is required, but communication is.

### Deletion test

Delete the legacy branch, legacy classification and repeated MCP default decisions. Do not delete the shared command module: its behavior would otherwise reappear across the CLI and MCP adapters. Moving functions into another file without reducing caller knowledge does not increase depth.

## 2. Deepen request admission

### Responsibility and interface

Keep one MCP request admission module; `src/main.rs` can retain it initially. Extract it into `src/mcp.rs` only if that keeps the responsibility coherent, not simply because the file is large. Do not add a trait or a second transport adapter: one stdio adapter does not establish a real transport seam.

Its conceptual interface is `admit(line, roots)`, with three outcomes:

- **Ignore:** an envelope-valid message with no `id`, even if its method or tool parameters are unknown/invalid; no reply and no design/output work.
- **Reply:** a complete protocol/control reply or tool rejection; no design loading.
- **Call:** an owned prepared tool call whose source and output passed the configured root policy.

These names describe the design, not a commitment to exact Rust declarations. Calls without a design, notably `workflow`, remain valid calls and expose no preload path. The interface hides JSON parsing, envelope checks, notification handling, tool preparation and path admission from `serve`.

Admission reads no design bytes and starts no remote download. It may inspect filesystem metadata to enforce the existing canonical-path policy. The admitted-call value owns both the immutable prepared call and the authoritative source/output results, with private fields and no unchecked constructor outside admission. Source loading, local preload, cache selection, export and bundle writing must use these stored results, never independently rederive a path from raw or normalized JSON.

For a local source or existing output, preserve the resolved path used by the root decision. For a new output, preserve the approved resolved parent plus filename. The path check must return its usable result, not only a boolean while execution chooses another spelling. Apply the same interpretation without configured roots. Preserve the remote source identity for the existing per-call resolver; resolve it once per call and pass that local result onward rather than resolve it independently for preload and execution. Remote calls still bypass local parsed-Tree reuse. Diagnostic labels may retain original input but are never execution paths. Necessary filesystem rechecks operate on the stored results; this is not a promise to eliminate races.

### Ordering

```text
incoming line
  -> parse JSON and validate envelope
  -> ignore envelope-valid messages without id, before method/tool validation
  -> answer control methods / reject invalid method shape
  -> prepare named tool arguments through the contract module
  -> check configured source and output roots
  -> execute an admitted call
       local design: reuse unchanged Tree or load one
       remote design: resolve and load per call
       workflow: read embedded guidance, no design
  -> serialize the existing reply shape
```

For messages with an `id`, a missing/non-string `tools/call` name remains a protocol error. Unknown tools and invalid arguments remain tool failures. Preparation precedes root checks; root checks precede design loading and output creation. Do not load a design to discover that a parameter or output path is invalid.

Preserve the existing protocol codes (`-32700`, `-32600`, `-32601`, `-32602`) and structured tool failure categories. Continue accepting envelopes without `jsonrpc`; removing argument compatibility does not authorize unrelated envelope changes. An absent `id` identifies a notification; an explicit null `id` retains current request handling.

### Notification regression matrix

Current ordering is confirmed by `src/main.rs::valid_request` and `mcp_reply_with`: envelope validation precedes `req.get("id")?`, which precedes method dispatch and tool preparation. Here, "valid notification" means envelope-valid, not a recognized method or valid tool call. Preserve the following cases through `serve`, both with and without configured roots:

| Input | Expected observable result |
| --- | --- |
| Unparseable JSON | `-32700`, response `id: null`; no design/output work. |
| Invalid envelope, including missing/non-string method, invalid `jsonrpc` or boolean `id` | `-32600`, response `id: null`, even if `id` is absent; no design/output work. |
| Envelope-valid, absent `id`, unknown method | Ignore; no reply, tool preparation, path lookup, source read or output. |
| Envelope-valid, absent `id`, `tools/call` with missing/non-string name or unknown tool | Ignore with the same absence of work. |
| Envelope-valid, absent `id`, `tools/call` with invalid or valid arguments | Ignore with the same absence of work; do not execute even a valid tool. |
| `id: null`, unknown method | Reply `-32601` with `id: null`. |
| `id: null`, `tools/call` missing/non-string name | Reply `-32602` with `id: null`. |
| `id: null`, unknown tool or invalid tool arguments | Tool result with `invalid_arguments`, `id: null`; no design/output work. |
| `id: null`, valid `ping` or valid tool call | Execute normal request handling and reply with `id: null`; normal source/root rules apply to tools. |

Use message sequences ending in an identified ping to prove ignored messages produced no stray replies and processing continues. Observe no filesystem/cloud/output work, not merely the absence of a response. Include omitted `jsonrpc` cases to preserve existing envelope acceptance.

### Ownership and cache constraints

Keep the current lexical ownership of `Document`, its decoded message and the borrowed `Tree`. An admitted call is owned and can be carried across the cache loop when it needs another local design. Carry that value, not the raw line to be parsed and admitted again.

Preserve local Tree reuse by path, modification time and length. Control calls, rejected calls, workflow calls and remote calls must not unnecessarily evict a valid local Tree. A changed local source must be reloaded; an unreadable source must yield a recoverable call failure rather than terminate later requests.

"Admit once" limits interpretation of a request, not observations of mutable filesystem state. Recheck modification metadata and current source readability before cached use, and retain necessary path/security checks before I/O using the stored admitted paths. Admission is not a permanent authorization or readability lease. Test warm-cache hits followed by changed/deleted/unreadable sources, metadata/read failures, and relevant symlink/output-parent changes; assert refusal or safe failure rather than stale success, followed by recovery on a valid request. Readability checks need observable evidence under the test process's actual privileges, not assumptions about permission bits.

Do not introduce self-referential ownership, global caches, remote parsed-Tree caching, generation pinning or asynchronous execution. Preserve panic recovery and finite parsing budgets. Fewer parses are a consequence of locality, not an unmeasured speed guarantee.

Root checks retain canonical-path and dangling-symlink handling. Remote `gs://` sources under `--root` remain refused before cloud access. This design does not remove filesystem races or promise race-free authorization.

### Deletion test

Replace the overlapping roles of `refused`, `direct_file` and `is_direct`; do not retain them as a second admission path beneath a new wrapper. Execution must not run argument preparation a second time. Preserve necessary authorization checks rather than deleting them to simplify the flow.

## Implementation sequence

1. **Tool contract:** concentrate parameter/default knowledge, introduce the prepared-call invariant, remove legacy translation and update its consumers/tests. Keep Tree-dependent checks and command execution shared. At this checkpoint, the existing server may still prepare the same input more than once.
2. **Request admission:** make protocol handling and root admission consume the prepared call once, carry it through `serve`, and delete redundant raw-request classification. Keep borrowed Tree ownership intact.

Each step must pass its applicable tests before the next. Do not combine the work with decoding, rendering or cache redesign.

## Caller audit

Static inspection at `dd33bd2` found that `pi/index.ts` registers MCP directly with the host via `registerMcpServer`; it does not build design-tool MCP arguments. Its mapping workflow invokes CLI commands through `pi.exec`. `pi/workflow.mjs` passes CLI strings such as `--limit 200`, and `pi/viewer.mjs` launches the CLI `preview` command; these are not legacy MCP coercions and must not be rewritten as JSON numbers. The corresponding `npm/pi/` files were byte-identical at inspection. `npm/bin/figlens.js` forwards argv and inherited stdio, not rewritten MCP parameters. No dependency on the removed MCP formats was found in these inspected paths.

This is not host/model compatibility evidence. Before release, exercise a strict-call/rejection/recovery sequence through Pi's registered MCP path and the packaged launcher, and audit shared skill/example instructions for old argument recipes. `scripts/mcp-smoke.py` now checks strict rejection and recovery instead of legacy success. Do not report Pi compatibility as verified solely from source inspection.

## Consumers and verification

The interface is the test surface. Prefer observable tool replies and ordered `serve` message sequences over tests of private classification predicates. Replace obsolete compatibility assertions; retain unrelated regression coverage. Use existing atomic/scoped fixture helpers for generated test files.

| Area | Required checks |
| --- | --- |
| `src/command/tests/contract.rs`, `src/command/tests.rs` | Discovery/default agreement; missing versus explicit values; strict rejection of former coercions; boolean false and `2.0` acceptance; unchanged command results and CLI behavior. |
| `src/mcp.rs` and `src/mcp/admission_tests.rs` | Notifications without replies or design work; workflow without a design; protocol versus tool errors; source/output refusals before work; symlinks; restricted remote refusal; recovery after invalid/unreadable calls. |
| `serve` message sequences | Repeated local calls, changed files, switching local files, interleaved control/workflow/remote calls and failed calls without poisoning subsequent requests. Preserve reuse semantics through an existing observation point or a narrow internal test hook, not a public cache interface. |
| `scripts/mcp-smoke.py` | Replace legacy success and `legacyCompatibility` reporting with strict-rejection/recovery assertions; retain successful calls and JSON-RPC-only stdout. Exercise through the existing candidate installation matrix. |
| `scripts/check-gcs-snapshot.py` | Retain remote metadata/download behavior and restricted-root refusal; no new remote-cache claim. |
| `npm/`, Pi extension and existing installation checks | Audit generated/passed arguments for rejected coercions; change only callers that actually rely on them. Run packaging checks when those files change. |
| `wiki/mcp.md`, `wiki/gcs.md`, user-facing help/examples, release notes | Update removed legacy behavior, metadata, error distinctions and renamed execution paths in the implementation change. Include intent-preserving replacement examples, especially uncapped `maxSize: 0`. Do not describe the proposed behavior as shipped beforehand. |

Implementation verification commands:

```sh
docker compose run --rm dev cargo test
docker compose run --rm dev sh -c 'cargo fmt --check && cargo clippy --all-targets -- -D warnings'
python3 scripts/ai-context.py check
```

Use the documented Docker/native-candidate smoke workflow for subprocess checks; do not introduce a host Rust toolchain. This design requires no new dependency, public fixture or emitted design-data change. Rendering accuracy measurements are not a substitute for request-contract tests and are not needed unless implementation changes rendering.

## Acceptance

- MCP has one owner for parameter meaning and defaults, with no CLI fallback.
- Prepared and admitted calls cannot be forged or mutated by consumers; `Command::from_tool` does not accept bare JSON.
- Each actionable request is parsed/prepared/admitted once before design loading; a carried call is not readmitted. Ignored notifications never prepare a tool call.
- Checked source/output results are the execution paths; there is no second interpretation of raw parameters.
- `serve` uses admission results without duplicating permission or argument policy, while retaining required filesystem freshness, readability and security rechecks.
- The notification matrix, warm-cache failure/recovery cases, Pi caller verification and release-note replacements are covered before release.
- CLI behavior, valid MCP results, workflow discovery, recoverable failures and local Tree reuse remain intact except for the explicit incompatibilities above.
- Tests and current-behavior documentation change with implementation; this proposal alone changes no runtime behavior.
