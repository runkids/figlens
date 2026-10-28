import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { Plugin } from '@opencode/plugin';
import { createAdapter } from './adapter.mjs';
import { registerReview } from './review-rpc.mjs';
import { operationRpc, reviewRpc } from './rpc.mjs';
import { WorkflowAuthorizationError } from '../pi/workflow.mjs';

const string = maxLength => ({ type: 'string', minLength: 1, maxLength });
const selection = { file: string(4096), frame: string(200) };
const object = (properties, required = []) => ({ type: 'object', properties, required, additionalProperties: false });
const texts = maxItems => ({ type: 'array', items: string(2000), maxItems });
const result = data => {
  const text = JSON.stringify(data, null, 2);
  if (Buffer.byteLength(text) > 4 * 1024 * 1024) throw new Error('Result exceeds 4 MiB. Request a smaller page.');
  return { output: data, content: [{ type: 'text', text }], metadata: { figlens: true } };
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
    description: 'Open or preview a design, plan, implement, or review mappings', execute: async input => {
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
