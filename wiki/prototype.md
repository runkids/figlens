# Prototype field evidence

`nodes` and bundle `design/nodes.json` add `_figlens.prototype` when a prototype source is present or an instance's missing main prevents checking it. Preview uses the same projection. `inspect --states` and `bundle --states` additionally collect it under `triggerEvidence.nodes`, with each effective `nodeId` and the inventory's source file. No new tool or option is needed.

## Supported facts

The projection reads literal snapshot fields, without converting them into a business state machine:

- `prototypeInteractions[i].event.interactionType` becomes `trigger`, with its own value, support state and source field.
- Each `actions[j]` retains separate `navigationType` and `connectionType` evidence. One does not silently replace the other.
- `transitionNodeID` becomes a node `destination`. A valid stored GUID is retained even when the node is missing (`unresolved`). A present node is `resolved`, independently of whether it has a name or can be rendered. An unset GUID has no destination; malformed references are unsupported and never become `0:0`.
- Every interaction/action retains its original array index and field path. `isDeleted: true` interactions are skipped; malformed deletion markers remain unsupported. Missing event/action fields and explicitly empty lists stay distinct.

Scalar field states are `recorded`, `absent` or `unsupported`. Recorded means a literal string/enum was read, not that figlens implements that event/action. Absent means that specific field was missing from the checked source, not that the design has no behavior. Destination state is `resolved`, `unresolved`, `absent` or `unsupported`. Missing and unsupported values never default to ON_CLICK or NAVIGATE.

Interaction/action `detailsState: not-extracted` leaves other parameters explicit as a gap. Timing, keyboard details, animation, conditions, variable operations, URLs and other action fields are not interpreted. No URL is followed and no prototype graph is traversed. A non-navigation action can retain its recorded connection type without inventing a destination. Prototype connections do not establish API outcomes, validation, loading completion or other business conditions; names such as Loading/Error do not establish events.

## Exact sources and instance limits

Every envelope has `sourceNodeId`, `sourceField`, `basis`, `interpretation: stored-fields-only` and `verification: not-compared-with-figma`. Read the stored source with raw `node` on the same file to verify its field path. Effective IDs can contain library override keys and must never be resolved by splitting their suffix.

- `stored-node`: a direct stored layer/component's own list.
- `expanded-source`: the source selected by the existing instance expander.
- `instance-override`: the winning field in `symbolData.symbolOverrides[index]`, including outer/nested precedence. A later patch containing only geometry does not replace the event source. An explicitly empty override stays empty.
- `instance-local`: explicit fields on the instance record itself. A swap retains this source rather than replacing it with the new main's list. When both instance-local data and a swapped definition are involved, `swappedDefinitionId` and `precedenceState: unsupported` leave runtime precedence unverified.
- `component-definition`: a main's own list when the instance has no local list, including nested instances swapped to that main. `inheritanceState: unsupported` and its reason prohibit treating those records as proven instance behavior. A missing main produces unsupported evidence, not absent triggers.

These are source observations and the expander's field selection, not a claim that Figma's prototype inheritance, ID-based list merging or playback is fully reproduced. The selected instance structure, text/image/visibility overrides and render preparation remain in use. A different source's same node ID must not be merged with these records.

## Inventory and bounds

`triggerEvidence` covers the selected effective subtree plus related variants' root records. It does not expand variant descendants, include unrelated frames, follow destinations or prove complete library coverage. Its aggregate state reports whether recorded/unsupported sources were returned; consumers must inspect each field, inheritance/precedence, hidden coverage and truncation. `businessConditions: not-inferred` always remains explicit. Older sidecars with `prototype_not_extracted` remain unknown, not evidence of absence.

Each source checks at most 32 interaction records (deleted ones still consume the scan limit), 16 actions per interaction and 16 KiB of projected JSON. Scalar labels are limited to 128 characters; oversized values are unsupported, not silently shortened. Destination names have 256-character previews and a truncation marker. Per-source `truncated` reports omitted records. The states inventory additionally shares its existing 128 KiB output and work/result budgets, with `prototype_limit`/byte reasons and bundle warnings; its context summary remains capped at 8 KiB. Existing selected-subtree expansion happens before those inventory limits. These are not whole-request memory/time guarantees.

Each interaction has `actionsTruncated`: true when its action-count or byte limit omitted actions, independently of `actionsState` (which describes the stored field). An empty projected array with this marker is unknown, not an explicitly empty source list. CLI/MCP nodes, state sidecars and preview retain the same marker; `context.md` reports it per interaction.

Preview shows unspecified/unsupported values literally, retains unresolved node IDs without making them clickable, and offers links only for resolved local node references. It distinguishes omitted actions from a source list known to be empty; when all actions in an interaction are omitted, the destination displays `Node destination not extracted`. For older envelopes without `actionsTruncated`, a truncated source with a recorded action list is conservatively incomplete. It uses text nodes for design strings. Component-definition and ambiguous swapped-local evidence disclose their limits. The `interactions` scope payload holds the same per-node evidence envelopes as the REST nodes. Previously generated offline HTML retains its embedded script and snapshot.

## Verification boundary

Original synthetic unit/local-copy fixtures verify literal event/action fields, deletion and malformed fields, unset/unresolved destinations, library override keys, nested field precedence, empty overrides, swaps and ambiguous instance-local records. Integration checks cover CLI/MCP, sidecars/context, source-field round trips, offline snapshot payloads and unchanged assets/reference bytes. Frontend unit tests exercise the renderer's text/link decisions without opening a port.

No private designs, external specifications, live Figma session or independent same-version Figma capture is used. This is source/contract evidence, not calibrated Figma prototype parity or proof of business behavior. No rendering architecture or persistent cache changes are included.
