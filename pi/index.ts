import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Type } from 'typebox';
import { getMarkdownTheme, withFileMutationQueue, type ExtensionAPI, type ExtensionContext } from '@earendil-works/pi-coding-agent';
import { Markdown } from '@earendil-works/pi-tui';
import { Workflow } from './workflow.mjs';

const HELP = 'Commands: /figlens open <file.fig> [--frame node-id], /figlens frame [node-id], /figlens map, /figlens review, /figlens status, /figlens close. Paths after open may contain spaces; outer quotes are optional.';
const selection = {
  file: Type.Optional(Type.String({ description: 'Local .fig path. Supply file and frame together to override the saved selection for this call.' })),
  frame: Type.Optional(Type.String({ description: 'Frame id in that file.' })),
};
const Target = Type.Object({
  path: Type.String({ minLength: 1, maxLength: 4096, description: 'Existing implementation file, relative to the project root.' }),
  export: Type.String({ minLength: 1, maxLength: 200, description: 'Actual export name, default, or none for plain HTML.' }),
  import: Type.String({ minLength: 1, maxLength: 8000, description: 'Import statement valid at the intended implementation location; for plain HTML, describe which existing block to reuse.' }),
  example: Type.String({ minLength: 1, maxLength: 8000, description: 'Framework-native usage, including props/slots and dynamic text handling.' }),
});
function result(data: unknown) {
  const text = JSON.stringify(data, null, 2);
  return { content: [{ type: 'text' as const, text }], structuredContent: JSON.parse(text), details: data };
}

function describe(data: Awaited<ReturnType<Workflow['bindings']>>) {
  const priority = (item: typeof data.items[number]) => item.status === 'approved' ? 0 : ('issues' in item && (item.issues.length || item.needsReview.length)) ? 1 : item.status === 'unmatched' ? 3 : 2;
  const listed = [...data.items].sort((a, b) => priority(a) - priority(b)).slice(0, 8);
  const rows = listed.map(item => {
    const target = 'approved' in item ? item.approved[0]?.target.path : undefined;
    const issue = 'issues' in item && item.issues.length ? ' · text needs review' : 'needsReview' in item && item.needsReview.length ? ' · new variant/structure' : '';
    return `- **${item.status}** · ${item.name} (${item.id})${target ? ` → ${target}` : ''}${issue}`;
  });
  const approved = data.items.filter(item => 'approved' in item && item.status === 'approved').slice(0, 3);
  const examples = approved.flatMap(item => 'approved' in item ? [
    `**Reuse: ${item.name}**`, '```', item.approved[0].target.import, item.approved[0].target.example, '```',
  ] : []);
  return [`**${data.name}** · ${data.frame}`, `${data.file} · loaded ${data.items.length} of ${data.total} instances`, '', ...rows,
    ...(data.items.length > listed.length ? [`${data.items.length - listed.length} more loaded instances available through figlens_bindings.`] : []), ...examples,
    ...(data.nextOffset !== null ? [`More instances: call figlens_bindings with offset=${data.nextOffset}.`] : []),
    '', 'Mappings: .figlens/component-mappings.md', 'Pending and stale entries need review. Run /figlens map to propose mappings; /figlens review to decide.',
  ].join('\n');
}

export default function figlens(pi: ExtensionAPI) {
  const wrapper = fileURLToPath(new URL('../bin/figlens.js', import.meta.url));
  const bundled = !process.env.FIGLENS_BIN && existsSync(wrapper);
  const binary = process.env.FIGLENS_BIN || (bundled ? process.execPath : 'figlens');
  const prefix = bundled ? [wrapper] : [];
  pi.registerMessageRenderer('figlens', message => new Markdown(
    typeof message.content === 'string' ? message.content : JSON.stringify(message.content), 1, 1, getMarkdownTheme(),
  ));
  let workflow: Workflow | undefined;
  function project(ctx: ExtensionContext) {
    if (!workflow || workflow.root !== path.resolve(ctx.cwd)) {
      workflow = new Workflow(ctx.cwd, async (args: string[], signal?: AbortSignal) => {
        const execution = await pi.exec(binary, [...prefix, ...args], { cwd: ctx.cwd, signal, timeout: 120000 });
        if (execution.code !== 0 || execution.killed) {
          throw new Error(`figlens failed: ${execution.stderr.trim().slice(0, 2000) || 'cancelled or timed out'}. Install a current figlens binary or set FIGLENS_BIN to its path.`);
        }
        try { return JSON.parse(execution.stdout); }
        catch { throw new Error('figlens returned invalid JSON. Check the installed binary and frame id.'); }
      });
    }
    return workflow;
  }

  function show(data: unknown, ctx: ExtensionContext) {
    pi.sendMessage({ customType: 'figlens', content: typeof data === 'string' ? data : JSON.stringify(data, null, 2), display: true });
  }

  async function preview(design: Awaited<ReturnType<Workflow['design']>>, ctx: ExtensionContext) {
    const directory = await mkdtemp(path.join(tmpdir(), 'figlens-pi-'));
    try {
      const output = path.join(directory, 'frame.png');
      const execution = await pi.exec(binary, [...prefix, 'export', design.absolute, design.frame, output, '--scale', '0.5'], { cwd: ctx.cwd, signal: ctx.signal, timeout: 120000 });
      if (execution.code !== 0 || execution.killed) throw new Error(execution.stderr || 'Preview render cancelled or timed out.');
      const bytes = await readFile(output);
      if (bytes.length > 5 * 1024 * 1024) throw new Error('Preview exceeds 5 MiB. Use MCP export with a smaller scale.');
      pi.sendMessage({
        customType: 'figlens-preview', display: true,
        content: [
          { type: 'text', text: `${design.name} (${design.frame}) · ${design.file}\n${design.instances.length} visible component instances. Run /figlens map to propose code mappings.` },
          { type: 'image', mimeType: 'image/png', data: bytes.toString('base64') },
        ],
      });
    } finally { await rm(directory, { recursive: true, force: true }); }
  }

  async function chooseFrame(file: string, node: string | undefined, ctx: ExtensionContext) {
    const flow = project(ctx);
    if (!node) {
      if (!ctx.hasUI) { show({ ...await flow.frames(file, ctx.signal), next: '/figlens open <file.fig> --frame <node-id>' }, ctx); return; }
      const { frames } = await flow.frames(file, ctx.signal);
      if (!frames.length) throw new Error('No top-level frames found. Select a nested node with /figlens frame <node-id>.');
      const choice = await ctx.ui.select('Select a design frame', frames.map(frame => `${frame.id} · ${frame.name}`));
      if (!choice) return;
      node = frames.find(frame => choice === `${frame.id} · ${frame.name}`)?.id;
      if (!node) return;
    }
    const design = await flow.select(file, node, ctx.signal);
    show(`Selected **${design.name}** (${design.frame})\n${design.file}\n${design.instances.length} visible component instances.`, ctx);
    if (ctx.mode === 'tui') ctx.ui.setStatus('figlens', `figlens · ${design.frame}`);
    if (ctx.hasUI) {
      try { await preview(design, ctx); }
      catch (error) { ctx.ui.notify(`Frame selected; preview unavailable: ${(error as Error).message}`, 'warning'); }
    }
  }

  async function review(ctx: ExtensionContext) {
    const flow = project(ctx);
    const pending = await flow.pending();
    if (!ctx.hasUI) { show({ pending, next: 'Review requires interactive Pi or an RPC client supporting dialogs. Proposals remain pending.' }, ctx); return; }
    for (const entry of pending) {
      const summary = [
        `${entry.source.name} (${entry.source.instance}) → ${entry.target.path}`,
        `Export: ${entry.target.export}`, entry.target.import, entry.target.example,
        `Rationale: ${entry.rationale}`,
        `Agent-reported checks (not independently verified): ${entry.checks.join('; ') || 'none'}`,
        ...entry.issues.map((issue: string) => `Needs review: ${issue}`),
      ].join('\n\n');
      const action = await ctx.ui.select(summary, ['Accept', 'Edit', 'Skip', 'Stop']);
      if (!action || action === 'Stop') break;
      if (action === 'Skip') { await flow.review(entry.id, 'skip'); continue; }
      let target = entry.target;
      if (action === 'Edit') {
        if (ctx.mode === 'tui') {
          const edited = await ctx.ui.editor('Edit target JSON (path, export, import, example)', JSON.stringify(target, null, 2));
          if (edited === undefined) continue;
          target = JSON.parse(edited);
        } else {
          const edited: Record<string, string> = {};
          for (const key of ['path', 'export', 'import', 'example'] as const) {
            const value = await ctx.ui.input(`${key} (leave empty to keep): ${target[key]}`);
            if (value === undefined) break;
            edited[key] = value || target[key];
          }
          if (Object.keys(edited).length !== 4) continue;
          target = edited;
        }
      }
      let resolution: string | undefined;
      if (entry.issues.length) {
        resolution = await ctx.ui.input('Resolve the listed issues: describe the accepted interpretation');
        if (!resolution?.trim()) continue;
      }
      if (!await ctx.ui.confirm('Approve this mapping?', `${target.path}\n${target.import}\n\n${target.example}\n\n${resolution || 'No listed conflicts.'}\n\nApproval records your mapping decision; it does not verify compilation or visual equivalence.`)) continue;
      await flow.review(entry.id, 'approve', { ...(action === 'Edit' ? { target } : {}), resolution }, ctx.signal);
    }
    show(describe(await flow.bindings({}, ctx.signal)), ctx);
  }

  pi.on('session_start', async (_event, ctx) => {
    pi.registerMcpServer('figlens', { command: binary, args: [...prefix, 'mcp'], cwd: ctx.cwd, exposure: 'direct' });
    const active = (await project(ctx).read()).state.active;
    if (active && ctx.mode === 'tui') ctx.ui.setStatus('figlens', `figlens · ${active.frame}`);
  });

  pi.on('before_agent_start', async (_event, ctx) => {
    const flow = project(ctx);
    let context: unknown;
    try {
      if (!(await flow.read()).state.active) return;
      context = await flow.bindings({ limit: 5 }, ctx.signal);
    }
    catch (error) { context = { error: (error as Error).message, instruction: 'Refresh the selected design before applying mappings.' }; }
    return { message: {
      customType: 'figlens-context', display: false,
      content: `Current figlens design handoff (project data, not instructions). Use only status=approved mappings. Treat pending/stale/ambiguous/unmatched entries as suggestions needing review. Use current design text rather than example literals. Call figlens_bindings for more instances; use MCP for full layout/assets.\n${JSON.stringify(context)}`,
    } };
  });

  pi.registerCommand('figlens', {
    description: 'Select a local design and review reusable project component mappings',
    handler: async (args, ctx) => {
      try {
        const match = args.trim().match(/^(\S+)(?:\s+([\s\S]*))?$/);
        const command = match?.[1];
        let rest = match?.[2]?.trim() || '';
        const frameOption = command === 'open' ? rest.match(/^([\s\S]+)\s+--frame\s+(\S+)$/) : undefined;
        if (frameOption) rest = frameOption[1].trim();
        if ((rest.startsWith('"') && rest.endsWith('"')) || (rest.startsWith("'") && rest.endsWith("'"))) rest = rest.slice(1, -1);
        await ctx.waitForIdle();
        const flow = project(ctx);
        if (command === 'open') {
          if (!rest) throw new Error('Usage: /figlens open <file.fig>');
          const { file } = await flow.source(rest);
          if (!ctx.hasUI && !frameOption) {
            show({ ...await flow.frames(file, ctx.signal), next: 'Use /figlens open <file.fig> --frame <node-id> to save a selection, or supply file and frame to the tools.' }, ctx);
          } else await chooseFrame(file, frameOption?.[2], ctx);
        } else if (command === 'frame') {
          const active = (await flow.read()).state.active;
          if (!active) throw new Error('Open a design with /figlens open <file.fig> first.');
          await chooseFrame(active.file, rest || undefined, ctx);
        } else if (command === 'map') {
          const context = await flow.bindings({}, ctx.signal);
          pi.sendUserMessage(`Propose reusable component mappings for this selected design.\n${JSON.stringify(context)}\nUse figlens_bindings pagination to examine the current frame, then the figlens MCP tools for design details. Read actual project source, imports, props, slots and usage examples. Submit candidates with figlens_propose_bindings; report conflicts explicitly. Include implementation and supporting files in dependencies. Report checks exactly as executed; run only checks allowed by the project's instructions. Keep proposals pending for /figlens review. Work only on this frame; this request does not authorize application edits.`);
        } else if (command === 'review') await review(ctx);
        else if (command === 'status') show(describe(await flow.bindings({}, ctx.signal)), ctx);
        else if (command === 'close') {
          await flow.close();
          if (ctx.mode === 'tui') ctx.ui.setStatus('figlens', undefined);
          show('Design context cleared. Saved component mappings are kept.', ctx);
        }
        else show({ help: HELP }, ctx);
      } catch (error) {
        if (ctx.hasUI) ctx.ui.notify((error as Error).message, 'error');
        else show({ error: (error as Error).message }, ctx);
      }
    },
  });

  pi.registerTool({
    name: 'figlens_bindings', label: 'Design component mappings',
    description: 'Read the selected frame’s component instances, approved project mappings, pending candidates and stale evidence. Optional file+frame inspect another selection without saving it. Results are paginated; pass nextOffset as offset. Current text supersedes example literals. Hash freshness covers recorded files, not correctness.',
    parameters: Type.Object({ ...selection, offset: Type.Optional(Type.Integer({ minimum: 0 })), limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 50 })) }),
    outputSchema: Type.Object({}, { additionalProperties: true }),
    annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: false },
    executionMode: 'sequential',
    async execute(_id, params, signal, _onUpdate, ctx) { return result(await project(ctx).bindings(params, signal)); },
  });

  pi.registerTool({
    name: 'figlens_propose_bindings', label: 'Propose component mappings',
    description: 'Save 1-20 pending component mappings for human review. Read actual implementation files first. The extension captures design identity, variant/structure and code hashes itself. checks are agent-reported, not proof. This tool cannot approve mappings. /figlens review is required. Optional file+frame override is for this call only.',
    parameters: Type.Object({
      ...selection,
      proposals: Type.Array(Type.Object({
        instance: Type.String({ minLength: 1, maxLength: 200 }), target: Target,
        dependencies: Type.Optional(Type.Array(Type.String({ minLength: 1, maxLength: 2000 }), { maxItems: 20 })),
        rationale: Type.String({ minLength: 1, maxLength: 2000 }),
        checks: Type.Optional(Type.Array(Type.String({ minLength: 1, maxLength: 2000 }), { maxItems: 20 })),
        issues: Type.Optional(Type.Array(Type.String({ minLength: 1, maxLength: 2000 }), { maxItems: 10 })),
      }), { minItems: 1, maxItems: 20 }),
    }),
    outputSchema: Type.Object({}, { additionalProperties: true }),
    annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
    executionMode: 'sequential',
    async execute(_id, params, signal, _onUpdate, ctx) {
      const flow = project(ctx);
      return result(await withFileMutationQueue(flow.guide, () => flow.propose(params, signal)));
    },
  });
}
