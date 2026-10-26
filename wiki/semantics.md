# Partial semantic fields

`nodes` and bundle `design/nodes.json` add the fields below from the effective expanded node. Existing sizing fields and property assignments are unchanged. These mappings are **not compared with same-version Figma exports**; every `_figlens.semantics` object carries `verification: "not-compared-with-figma"`. This is not a claim of REST API parity.

## Layout

| Output | Snapshot source | Supported recorded values |
|---|---|---|
| `constraints.horizontal` | `horizontalConstraint` | MIN → LEFT, MAX → RIGHT, CENTER, SCALE, STRETCH → LEFT_RIGHT |
| `constraints.vertical` | `verticalConstraint` | MIN → TOP, MAX → BOTTOM, CENTER, SCALE, STRETCH → TOP_BOTTOM |
| `layoutGrow` | `stackChildPrimaryGrow` | 0 or 1 on normal-flow children of HORIZONTAL/VERTICAL auto-layout |
| `layoutAlign` | `stackChildAlignSelf` | INHERIT, STRETCH, MIN, CENTER, MAX in the same context |

Missing axes stay absent, so `constraints` can contain only one axis. Swaps retain the original child constraints, grow, align and positioning (including absence) for these new fields; explicit overrides take precedence. Legacy sizing remains unchanged. Unknown enums and growth values are not coerced. Absolute-positioned, grid and non-auto-layout children do not receive grow/align; their recorded but unsupported values are identified in metadata. Existing `layoutSizingHorizontal`/`layoutSizingVertical` HUG/FILL/FIXED rules remain unchanged. A directly selected stored node still uses its actual parent for the new fields.

Each field's metadata contains `source` and `state`: `recorded` means a supported stored value, `absent` means no source field, and `unsupported` means a present value/context not emitted. Recorded does not mean calibrated against Figma. Expanded nodes additionally identify `instanceContext.instanceId` and `sourceNodeId` with `state: "effective"`; this describes the expansion used, not an override history.

## Component definitions

`componentPropertyDefinitions` appears on selected COMPONENT/COMPONENT_SET nodes and on referenced `components`/`componentSets` metadata. Definitions resolve `parentPropDefId` through the same parent lookup used by existing instance property assignments. Provenance entries retain own/resolved definition IDs and source node IDs.

- BOOL/BOOLEAN, TEXT and INSTANCE_SWAP emit their known type. Non-variant names use `name#definitionId`.
- Defaults use the existing selection: `varValue.value`, then the `value` object (or `initialValue` when `value` is not an object). Boolean false and empty text are preserved. A default with the wrong type is omitted and marked unsupported. Swap defaults require a real stored GUID; arbitrary strings and the unset sentinel are omitted.
- Swap `preferredValues.instanceSwapValues` emits only lists entirely composed of COMPONENT/COMPONENT_SET entries with nonempty keys. Unsupported lists are omitted and marked; an explicitly empty supported list remains empty.
- VARIANT emits its name/type only. Default and complete options are not established and are marked unsupported. SLOT and unknown types remain provenance-only. Unresolved parent references do not become guessed definitions.
- Missing definitions are absent, explicitly empty arrays are recorded, and malformed definition lists are unsupported. Metadata distinguishes these from unsupported entries within a recorded list.

`inspect` shows layout fields, type-only definitions and provenance for the nodes it already includes. Definition lists, provenance entries and render diagnostics are each limited to 20 per shown node; counts/truncation remain explicit. Full values stay in `nodes`/bundles. `compare` counts constraints, grow, align and definitions separately, including definitions on referenced component metadata; it does not compare `_figlens` provenance. Asset planning also ignores `_figlens` metadata. Pi mapping signatures and human approval rules are unchanged.

## Evidence and limits

On 2026-10-26, authorized Aurora and held-out DashArt local snapshots supplied recorded constraint/align values; Aurora additionally supplied growth, inherited text definitions and incomplete variant records. DashArt supplied no component definitions. Positive Boolean and swap/default/preferred-value cases are original synthetic tests, not independent real-file confirmation. Tests cover absence/unknown values, parent resolution, effective instance overrides/swaps, bounded inspection and compare categories.

No paired Figma JSON, PNG or SVG was available. Same-version independent captures are still needed to calibrate the mappings and establish any pixel-fidelity improvement. Snapshot omissions must not be interpreted as Figma defaults. No Dev Mode annotations, full grid semantics, SLOT inference, font reflow or new rendering algorithm is included.

The target shapes are described by the official [REST node types](https://developers.figma.com/docs/rest-api/file-node-types/), [layout constraints](https://developers.figma.com/docs/rest-api/file-property-types/#layoutconstraint) and [component property definitions](https://developers.figma.com/docs/plugins/api/ComponentPropertyDefinitions/). Those documents specify the public target shape; they do not prove the private snapshot mappings.
