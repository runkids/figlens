import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { Plugin } from '@opencode/plugin';
import { createAdapter, indexGrant } from './adapter.mjs';
import { registerReview } from './review-rpc.mjs';
import { operationRpc, reviewRpc } from './rpc.mjs';
import { WorkflowAuthorizationError } from '../pi/workflow.mjs';
import { Targets, check, checkInput, failure, notice, reminder } from '../pi/stale.mjs';

const string = maxLength => ({ type: 'string', minLength: 1, maxLength });
const selection = { file: string(4096), frame: string(200) };
const object = (properties, required = []) => ({ type: 'object', properties, required, additionalProperties: false });
const texts = maxItems => ({ type: 'array', items: string(2000), maxItems });
const bool = { type: 'boolean' }, path = string(4096), id = string(200);
// An exact instance sublayer id has up to expand::MAX_DEPTH (256) segments of at most 23 bytes, as src/search/store.rs bounds it.
const node = string((256 + 1) * 23);
// The figlens MCP index, jobs, search, usages, diff, changes, watch and inspect tools, run as the same CLI commands.
const CLI_TOOLS = [
  ['index', 'Local workspace indexes for search, as the figlens MCP index tool. prepare registers file (or a project workspace) and with scope builds exactly that stored scope (0:0 is the whole file), returning workspaceId and the finished job; status reports sources and storage; add, remove, relocate, refresh, rebuild and clear_cache manage sources and indexes; track starts recording the changes of one scope with a baseline build and untrack stops it, keeping its history; reset deletes this workspace database only after the user asked for it. Errors carry recovery steps; recreation (replaceUnavailable) needs the user\'s confirmation.',
    { action: { enum: ['prepare', 'status', 'add', 'remove', 'relocate', 'refresh', 'rebuild', 'track', 'untrack', 'clear_cache', 'reset'] },
      file: path, workspace: path, workspaceId: id, sourceId: id, scope: node, requestId: id, storage: path, replaceUnavailable: bool }, ['action']],
  ['jobs', 'Status or cancellation of an index job by its jobId, as the figlens MCP jobs tool.',
    { action: { enum: ['status', 'cancel'] }, jobId: id, storage: path }, ['action', 'jobId']],
  ['search', 'Ranked search of layer names and effective text, as the figlens MCP search tool: give file to read a .fig now, or workspaceId to search its prepared indexes without parsing. Results carry exact ref.nodeId for figlens_inspect, matches and coverage; sources not ready list the index call that prepares them.',
    { file: path, workspaceId: id, sourceId: id, storage: path, query: string(2000), node, type: id,
      limit: { type: 'integer', minimum: 1, maximum: 200 }, cursor: string(4096), includeHidden: bool, includeInternal: bool, mode: { enum: ['text'] } }, ['query']],
  // A usages target and scope take any id an index stores, as src/search/usages.rs validates them.
  ['usages', 'Exact relations of one node or resource in one stored scope, as the figlens MCP usages tool: give file to read a .fig now, or workspaceId to read its prepared index of scope. direction in (default) lists what points at it, out what it points at. Results carry exact refs, coverage and a snapshotId; a changed source is source_stale.',
    { file: path, workspaceId: id, sourceId: id, storage: path, node, resource: node, scope: node, relation: string(200),
      direction: { enum: ['in', 'out'] }, snapshotId: id, includeHidden: bool, limit: { type: 'integer', minimum: 1, maximum: 200 }, cursor: string(4096) }],
  ['diff', 'What changed between two saved revisions of a design (old, file), as the figlens MCP diff tool: layers match by exact id and type only, unknown means a side stopped or left that layer unexpanded, not removed. Page with nextCursor for the same pair and node.',
    { old: path, file: path, node, limit: { type: 'integer', minimum: 1, maximum: 500 }, cursor: string(4096) }, ['old', 'file']],
  ['changes', 'Recorded design changes of the scopes index track follows, as the figlens MCP changes tool; never writes or builds. A query subscription\'s subscriptionId pages its stored search evaluations instead, with no revision pages: evaluation current, current_for_index (the file changed since that index: refresh, then evaluate), pending, or unavailable (also when its evaluation at this publication failed, with that reason); a stored evaluation too large or damaged to page fails with a recovery that acks past it. Pages revisions oldest first after cursor, else after subscriptionId\'s or consumer\'s checkpoint, else from the oldest kept; each changes revision\'s impact summarizes the layers its changes reach (null before it was recorded); revision pages one revision\'s changed nodes, then its impact\'s listed layers, each marked by entry; limit is 1-50 revisions, or 1-500 entries with revision. tracking says when a scope needs refresh to record a change; event_gap means retention deleted unread revisions: follow its recovery.',
    { workspaceId: id, sourceId: id, scope: node, consumer: string(128), subscriptionId: id, cursor: string(4096), revision: id,
      limit: { type: 'integer', minimum: 1, maximum: 500 }, storage: path }, ['workspaceId']],
  ['watch', 'Watching tracked scopes, change consumers and subscriptions, as the figlens MCP watch tool. start holds a lease on a tracked scope (sourceId when the workspace has several) for this session: the workspace\'s shared runner refreshes it on each save, so figlens_changes shows new revisions without an index call; it asks read on that design and edit on the workspace. A requestId replays this session\'s live or pending start and reports its ended lease (kept 1 h, newest 1024), never restarting it; the same requestId with other arguments is refused. stop ends a lease this session started (leaseId); deleting the session or unloading the plugin ends them all and logs any release it could not confirm. status lists the workspace\'s leases and runner, read-only. ack moves a consumer\'s checkpoint (same sourceId and scope), or a subscription\'s (subscriptionId), to the nextCursor of the last figlens_changes page it handled. An older cursor normally leaves it, so a retry is safe; a cursor issued before retention deleted revisions it had not passed is event_gap and writes nothing: follow its recovery. A current recovery cursor moves a checkpoint behind such a deletion back to it, so the kept revisions replay; advanced is true only for forward progress, and the returned checkpoint is the one held. subscribe follows a source, scope or exact node (never a same-name layer: target_missing or identity_uncertain) from now, with its own checkpoint; owner makes it temporary for that task. subscribe with query (and type) saves a search of one source\'s scope instead and stores its top-50 evaluation, and evaluate (no sourceId: it evaluates the subscription\'s own source) stores a newer one after a publication, and a watch lease on that scope makes its runner store it unasked (unavailable evaluation_too_large past 128 KiB), and figlens_changes only reads them; delta refs are exact sourceId/snapshotId/nodeId, rankChanged is null when rankComparable is false, and nodes compare by sourceId, which relocate keeps. list shows visible subscriptions; unsubscribe removes one (or an owner\'s temporary ones) and forget a consumer\'s raw checkpoints; neither untracks.',
    { action: { enum: ['start', 'stop', 'status', 'ack', 'subscribe', 'evaluate', 'list', 'unsubscribe', 'forget'] }, workspaceId: id, consumer: string(128), cursor: string(4096),
      sourceId: id, scope: node, subscriptionId: id, node, query: string(2000), type: id, owner: string(128), after: id, leaseId: id, requestId: id, storage: path },
    ['action', 'workspaceId']],
  ['inspect', 'Compact expanded node summary of a .fig node or exact instance sublayer, as the figlens MCP inspect tool.',
    { file: path, node, depth: { type: 'integer', minimum: 0, maximum: 10 }, images: bool, states: bool, flat: bool, compareVariant: node }, ['file', 'node']],
];
const result = data => {
  const text = JSON.stringify(data, null, 2);
  if (Buffer.byteLength(text) > 4 * 1024 * 1024) throw new Error('Result exceeds 4 MiB. Request a smaller page.');
  // OpenCode refuses a structured output from a tool that declares no output schema.
  return { content: [{ type: 'text', text }], metadata: { figlens: true } };
};

export default Plugin.define({ id: 'figlens', async setup(ctx) {
  const adapter = createAdapter(ctx, import.meta.url);
  const registrations = [];
  registrations.push(await ctx.mcp.transform(adapter.configureMcp));
  const skillPath = fileURLToPath(new URL('../skills/figlens/SKILL.md', import.meta.url));
  const skill = await readFile(skillPath, 'utf8');
  registrations.push(await ctx.skill.transform(editor => {
    if (!editor.list().some(item => item.name === 'figlens')) editor.add({
      id: 'figlens', name: 'figlens', description: 'Read local Figma designs, plan and implement screens, and review reusable mappings.',
      path: skillPath, content: skill,
    });
  }));
  registrations.push(await registerReview(ctx, adapter));
  const operations = await ctx.rpc.register(operationRpc, { run: async (input, call) => {
    try { return await adapter.operation(input, call.signal); }
    catch (failure) {
      const error = failure instanceof WorkflowAuthorizationError ? failure.cause : failure;
      if (call.signal.aborted || error?.name === 'AbortError' || error?._tag === 'Permission.DeclinedError') return call.error('cancelled', 'Operation cancelled; reload the stored state before retrying.', null);
      if (error?._tag === 'Tool.Error') return call.error('permission_denied', error.message, null);
      return call.error('storage_error', error?.message ?? String(error), null);
    }
  } });
  registrations.push(operations);
  registrations.push(await ctx.command.transform(editor => editor.add({ name: 'figlens',
    description: 'Plan or implement a design, select a scope or review component mappings: open, preview, plan, yolo, frame, map, review, status, close, help', execute: async input => {
      const text = input.prompt.text.trim();
      if (text === 'review' || text === 'frame') {
        await adapter.scope(input.sessionID);
        await ctx.session.wait({ sessionID: input.sessionID });
        await operations.events.emit('manual', { sessionID: input.sessionID, action: text });
        await ctx.session.synthetic({ sessionID: input.sessionID, resume: false,
          text: `figlens ${text}: use the active native TUI dialog. Noninteractive clients leave mappings and selection unchanged.` });
      } else await adapter.commandHandler(input);
    },
  })));
  registrations.push(await ctx.tool.transform(editor => {
    editor.add({ name: 'figlens_preview', description: 'Start a local interactive viewer for a .fig file and optional node. Returns the server loopback URL; may write render cache. Preserves the old viewer on failure. Never changes selection or mappings.',
      input: object({ file: string(4096), node: string(200) }, ['file']),
      options: { codemode: false },
      execute: async ({ file, node }, call) => result(await adapter.preview(await adapter.scope(call.sessionID, call.signal), file, node)),
    });
    editor.add({ name: 'figlens_bindings', description: 'Read component mappings, evidence and current variants. Supply file and frame together for a temporary override; omit both for the manual selection. Read-only, never approves. Paginate using nextOffset.',
      input: object({ ...selection, offset: { type: 'integer', minimum: 0 }, limit: { type: 'integer', minimum: 1, maximum: 50 } }),
      options: { codemode: false },
      execute: async (input, call) => {
        const current = await adapter.scope(call.sessionID, call.signal);
        return result(await current.flow.bindings(input, current.signal));
      },
    });
    editor.add({ name: 'figlens_propose_bindings', description: 'Record 1–20 reusable component mappings as pending for /figlens review. No approval tool exists. Supply file and frame together for this call only. Checks are agent-reported; human approval and compilation remain separate.',
      input: object({ ...selection, proposals: { type: 'array', minItems: 1, maxItems: 20, items: object({
        instance: string(200), target: reviewRpc.methods.review.input.properties.target,
        dependencies: texts(20), rationale: string(2000), checks: texts(20), issues: texts(10),
      }, ['instance', 'target', 'rationale']) } }, ['proposals']),
      options: { codemode: false },
      execute: async (input, call) => {
        const current = await adapter.scope(call.sessionID, call.signal);
        return result(await current.flow.propose(input, current.signal));
      },
    });
    // An MCP server already serves these tools unless the user's figlens server is disabled or dropped.
    if (!adapter.cliPath) return;
    for (const [name, description, properties, need] of CLI_TOOLS) editor.add({ name: `figlens_${name}`, description,
      input: object(properties, need), options: { codemode: false },
      execute: async (input, call) => result(await adapter.cli(await adapter.scope(call.sessionID, call.signal), name, input)),
    });
  }));
  // Reset and recreation through the MCP index tool ask the same host permission as the CLI path.
  registrations.push(await ctx.tool.hook('execute.before', async call => {
    // On the CLI path figlens_index asks inside adapter.cli; a server named figlens would match it here too.
    const grant = !adapter.cliPath && call.tool === `${adapter.mcpName}_index` && indexGrant(call.input ?? {});
    if (grant) await (await adapter.scope(call.sessionID)).authorize('figlens_index', [grant]);
  }));
  // Watch targets each session subscribed to or acknowledged, held in memory: after an OpenCode restart a session
  // is checked again from its next subscribe or ack. On the CLI path the check is adapter.cli's changes, asking the
  // same read and bash permissions as the agent's own call; an MCP server's --root folders are not visible here,
  // so through MCP the session is only reminded to call changes itself.
  const watched = new Map(), notes = new Map();
  registrations.push(await ctx.tool.hook('execute.after', call => {
    const targets = watched.get(call.sessionID) ?? new Targets();
    if (targets.observe(call.tool, call.input, call.result?.content, call.status !== 'completed')) watched.set(call.sessionID, targets);
  }));
  registrations.push(await ctx.session.hook('prompt', async input => {
    notes.delete(input.sessionID);
    const targets = watched.get(input.sessionID);
    if (!targets?.list.length) return;
    let text = reminder(targets.list);
    if (adapter.cliPath) {
      // One check at a time, so at most one read permission question is open while the deadline runs; each
      // scope carries the batch signal, so the deadline also stops its CLI child.
      text = notice(await check(targets.turn(), async (target, signal) => {
        try { return await adapter.cli(await adapter.scope(input.sessionID, signal), 'changes', checkInput(target)); }
        catch (error) { throw error.cause?.stderr ? failure(error.cause.stderr) : error; }
      }, adapter.signal, { parallel: 1 }), targets.dropped);
    }
    if (text) notes.set(input.sessionID, text);
  }));
  // The note stays in each model call of that prompt's turn; the next prompt checks again.
  registrations.push(await ctx.session.hook('context', input => {
    const text = notes.get(input.sessionID);
    if (text) input.system.push({ type: 'text', text });
  }));
  const events = (async () => {
    try {
      for await (const event of ctx.event.subscribe({ types: ['session.deleted'], signal: adapter.signal })) {
        watched.delete(event.data.sessionID);
        notes.delete(event.data.sessionID);
        await adapter.close(event.data.sessionID);
      }
    } catch (error) {
      if (!adapter.signal.aborted) { console.error('figlens session cleanup stream failed:', error); await adapter.dispose(); }
    }
  })();
  return async () => {
    try { await adapter.dispose(); }
    finally {
      await events;
      for (const registration of registrations.reverse()) await registration.dispose();
    }
  };
} });
