# Related component variants

`figlens inspect <file.fig> <node> --states` (MCP `inspect` with `states: true`) adds an independent `states` object. It works with `--images` and uses the already resolved effective subtree, irrespective of summary depth. `bundle --states` includes the same inventory in a handoff. Both options default to false; ordinary inspect, REST nodes and bundles without the option keep their existing contracts. `states` is a per-call tool argument supplied by the agent, not an MCP configuration setting.

## Evidence and discovery

Resolve a recognizable frame with find/tree, then inspect it with states enabled. `instances` retains each full instance ID, effective component reference, current properties and explicit relationship status. `sets` lists each related component set once, including its component children and their stored names. A selected COMPONENT or COMPONENT_SET also lists its related variants. Follow returned node IDs with inspect/nodes on the same source file.

Membership requires the actual parent COMPONENT_SET (`FRAME` with `isStateGroup`) and component children (`SYMBOL`). Names, positions, appearance, hidden layers and prototype destinations never establish membership. Variants can encode size or theme as well as behavior; listing them does not classify them as business states.

Variant properties use `variantPropSpecs.propDefId/value` and the set's `componentPropDefs`. Entries retain the component's `sourceNodeId`, the set's `definitionNodeId` and `definitionId`; definition IDs are references within that source node, not independently selectable nodes. Reference and definition GUIDs require both fields to be unsigned integers; malformed fields never default to zero or establish a relationship. Parent membership also requires a strictly typed GUID, even for candidates found through Tree children. Invalid parent references produce `unsupported_component` with `reason: invalid_component_parent`; excluded set candidates mark `membershipComplete: false`, `membershipState: unsupported` and the same `membershipReason`. Missing specifications are `absent`; unresolved definitions, unsupported values and oversized scalar values remain `unsupported`. Names may be shortened with `nameTruncated`; IDs are never shortened. No properties are parsed from a variant's name.

Current properties come from the effective instance, including false and empty text. Expanded source identity uses `_figlens.semantics.instanceContext.sourceNodeId`, so a nested swap can identify its effective component even when the legacy REST componentId/properties are unavailable. In that case the list reports `currentPropertiesState: unsupported` rather than copying defaults from the new main. The existing document is neither replaced nor mutated. This identifies the expansion used, not a replay of alternate variants with the instance's overrides.

`missing_component`, `unsupported_component` and `no_component_set` distinguish unresolved references from a known standalone component. `triggerEvidence` collects [supported prototype field evidence](prototype.md) with exact node/override sources and per-field unknowns, scoped to the selected effective subtree and variant roots. Component-definition inheritance and swapped instance-local precedence remain unverified. It reads no external specifications or business conditions. Loading/Error names do not establish events, API outcomes or business conditions.

## Bundle handoff

```sh
figlens bundle design.fig 12:8 handoff.zip --states
figlens bundle design.fig --frames frames.txt --out-dir handoff --jobs 1 --states
```

MCP uses the existing `bundle` tool with `states: true`. Ask the agent to include related component states; it supplies the argument. No new MCP tool, configuration file or installed skill is required.

`design/states.json` contains the same object as `inspect.states` for the selected scope, preserving source, unknown, hidden-coverage and truncation markers. It is computed from the bundle's prepared effective document, without a second REST resolution or alternate-variant rendering. The selected nodes, variables, assets and reference files retain their existing contents. Other variants' structures/assets are not added; inspecting them requires the same source snapshot.

The optional sidecar is registered as metadata in `manifest.files`, with size and SHA-256, and referenced by `manifest.root.states`. Bundle format/version remain `figlens-bundle`/1. Existing consumers can continue reading their known files; consumers that require an exact entry/field list need to allow this opt-in extension. The legacy library `bundle::write` keeps the option off; CLI/MCP/batch use `write_with_states`. With the option off, neither the sidecar, root pointer nor context section is added. An older bundle without them means state information was not provided, not that there are no states.

`context.md` adds a summary of the source, current instance references/statuses, sets, variant names/IDs, supported prototype fields and evidence gaps. Design strings use the existing safe one-line quoting. This added section has its own 8 KiB cap and explicitly points back to the sidecar when shortened. Its summary limit is independent of inventory truncation. An inventory with `truncated: true` also adds a bundle warning (CLI/MCP/manifest); it does not add a missing-asset entry. Missing mains, unsupported fields and hidden coverage remain visible even when no related sets are returned. State names and prototype connections do not establish business conditions.

## Scope and limits

All IDs belong to `source.file`, the canonical local path when available. The path is a locator, not an immutable snapshot digest or a Figma file key. Keep it with IDs and re-read after a source change; never merge IDs from different source paths. No cross-file or new persistent cache is introduced.

The source is a saved snapshot. `coverage.hiddenSubtreesNotExpanded` identifies encountered hidden instances/expanded layers whose descendants may be unavailable. Existing hidden-layer expansion rules are unchanged. A complete membership listing covers only component copies present in this snapshot, not every variant in an external library.

Fixed limits apply before additional scanning/copying: 10,000 work items shared across subtree nodes, set candidates, definitions and properties; 200 instances; 32 sets; 64 variants per set and 256 overall; 32 properties per variant/instance; names and scalar text values up to 256 characters; complete IDs/source locators up to 4,096 bytes. Oversized values are omitted rather than silently shortened. The states object is at most 128 KiB, with space reserved for provenance/coverage. The selected component is listed first when discovering its set.

`states.truncated` and `reasons` are independent of the ordinary summary's `truncated`. Each set has `membershipComplete`; property support and hidden coverage must also be checked. A work/result/byte limit is not evidence of no more states. No unmeasured total or default state is invented.

The implementation reuses Tree node/child lookups, deduplicates sets within a request and never resolves or renders sibling variants to populate this list. Normal inspect still builds/expands its selected subtree before applying the new analysis limits. These are not parser, process-memory or whole-request time limits; selecting a whole set can itself include all its children in the ordinary inspection.

## Compare two variants

```sh
figlens inspect design.fig 12:8 --compare-variant 12:9
```

MCP uses `inspect` with `node: "12:8"` and `compareVariant: "12:9"`. The optional `variantComparison` is independent of `states`, `images`, summary depth and summary truncation. Both IDs must select different stored COMPONENT definitions under the same explicit COMPONENT_SET in this source file. Instances, expanded instance paths, standalone components and unrelated same-name frames are rejected. Ordinary inspect, `compare` against Figma captures, bundle contents and preview controls keep their existing contracts; no bundle comparison sidecar is added.

`rootChanges` compares the two roots by their explicit variant role. Each before/after value keeps its `sourceNodeId`, raw `sourceField` and `recorded`/`absent`/`unsupported` state. The `fields` array declares the checked paths: size, visibility, opacity, fills/strokes (including stored image hashes), effects, blend, stroke width (including per-side `border*Weight`), corner radii, basic stack layout, text, font fields and the transform's linear part. Left/top padding use the stored `stackHorizontalPadding`/`stackVerticalPadding` fields. Canvas translation is excluded. Missing fields remain absent, never inferred defaults. Values use the existing four-decimal comparison normalization. This compares stored fields, not resolved paints, computed layout, inherited visibility or final pixels.

`pairedNodes` pairs a descendant with its counterpart only when, under already paired parents (the roots first), each side has exactly one scanned child of that type and name, and neither name was cut short; `basis` records that rule, and `changes` lists the pair's differing fields like `rootChanges`. Duplicate names stay unpaired (#163). A variable alias in a recorded value gets `variableId` and `variableName`, resolved like `variables <node>` including a library's cached copies; `null` when the file holds neither. Names are cut like layer names (`variableNameTruncated`) and charged to the shared input budget; a field whose aliases could not all be named is `unsupported` (`variable_name_limit`), never a difference.

`collectionChanges` compares each field's value multiset among the remaining unpaired descendants, grouped by node type, with source IDs and occurrence counts. A multiset difference identifies changed stored values/counts; it does not pair two layers or prove that a layer was added/removed. `unpairedNodes` lists the other descendants with their variant ID and `state: not-paired`. Child order and override keys do not establish correspondence. Unsupported fields appear in `unknownFields`; collection comparisons affected by unsupported values or incomplete scanning are `unknown`, not confirmed differences. `state: no-differences-in-compared-fields` never claims visual equality; unpaired descendants, missing coverage or nested instances prevent that state when no confirmed field changes were found.

The comparison walks only the two stored definition subtrees, including stored hidden layers. It never expands nested instances, follows their mains, loads images or renders either variant. `coverage.nestedInstancesNotExpanded` and `notCompared` expose those limits. Text ranges, vector-region paints, resolved variable values, geometry, prototype playback and business conditions remain outside this comparison.

Limits precede copying: at most 200 stored nodes per side, 10,000 shared node/value work items, depth 16 and 4 KiB per readable field value, 64 KiB of shared input projections and 128 KiB of output. Node names retain the inventory's 256-character preview; IDs are not shortened. `truncated` and `reasons` disclose scan/output limits; `unknownFields` discloses malformed or oversized values. Normal inspect still resolves the selected primary subtree after this bounded analysis, so these are comparison limits rather than whole-request memory/time guarantees. No speed or Figma accuracy improvement is claimed.

## Verification boundary

Original synthetic tests cover explicit relations, unrelated same-name frames, multidimensional properties, standalone/missing components, hidden coverage, nested property swaps, source-scoped IDs, false/empty properties, independent limits, and CLI/MCP parity with images and states enabled together. Existing instance tests retain text/image overrides and bundle structure/reference checks. No private designs, external specifications or prototype graph are read by these checks.

Bundle tests additionally compare existing file bytes, manifest integrity, scope/source identity, single/MCP/batch results in both job modes, missing/hidden/truncated cases, safe quoting and context byte limits. Variant comparison tests cover literal field and multiset changes, unpaired same-name nodes, unknown/limited values, same-set admission, CLI/MCP parity and unchanged ordinary inspection. This is synthetic contract evidence, not comparison against independent same-version Figma exports. No rendering-fidelity or speed improvement is claimed. Broader prototype semantics and pixel differences remain separate follow-ups.
