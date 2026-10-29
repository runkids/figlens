import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { Plugin } from '@opencode/plugin';
import { createAdapter, indexGrant } from './adapter.mjs';
import { registerReview } from './review-rpc.mjs';
import { operationRpc, reviewRpc } from './rpc.mjs';
import { WorkflowAuthorizationError } from '../pi/workflow.mjs';

const string = maxLength => ({ type: 'string', minLength: 1, maxLength });
const selection = { file: string(4096), frame: string(200) };
const object = (properties, required = []) => ({ type: 'object', properties, required, additionalProperties: false });
const texts = maxItems => ({ type: 'array', items: string(2000), maxItems });
const bool = { type: 'boolean' }, path = string(4096), id = string(200);
// The figlens MCP index, jobs, search and inspect tools, run as the same CLI commands.
const CLI_TOOLS = [
  ['index', 'Local workspace indexes for search, as the figlens MCP index tool. prepare registers file (or a project workspace) and with scope builds exactly that stored scope (0:0 is the whole file), returning workspaceId and the finished job; status reports sources and storage; add, remove, relocate, refresh, rebuild and clear_cache manage sources and indexes; reset deletes this workspace database only after the user asked for it. Errors carry recovery steps; recreation (replaceUnavailable) needs the user\'s confirmation.',
    { action: { enum: ['prepare', 'status', 'add', 'remove', 'relocate', 'refresh', 'rebuild', 'clear_cache', 'reset'] },
      file: path, workspace: path, workspaceId: id, sourceId: id, scope: id, requestId: id, storage: path, replaceUnavailable: bool }, ['action']],
  ['jobs', 'Status or cancellation of an index job by its jobId, as the figlens MCP jobs tool.',
    { action: { enum: ['status', 'cancel'] }, jobId: id, storage: path }, ['action', 'jobId']],
  ['search', 'Ranked search of layer names and effective text, as the figlens MCP search tool: give file to read a .fig now, or workspaceId to search its prepared indexes without parsing. Results carry exact ref.nodeId for figlens_inspect, matches and coverage; sources not ready list the index call that prepares them.',
    { file: path, workspaceId: id, sourceId: id, storage: path, query: string(2000), node: id, type: id,
      limit: { type: 'integer', minimum: 1, maximum: 200 }, cursor: string(4096), includeHidden: bool, includeInternal: bool, mode: { enum: ['text'] } }, ['query']],
  ['inspect', 'Compact expanded node summary of a .fig node or exact instance sublayer, as the figlens MCP inspect tool.',
    { file: path, node: id, depth: { type: 'integer', minimum: 0, maximum: 10 }, images: bool, states: bool, flat: bool }, ['file', 'node']],
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
  const events = (async () => {
    try {
      for await (const event of ctx.event.subscribe({ types: ['session.deleted'], signal: adapter.signal })) {
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
