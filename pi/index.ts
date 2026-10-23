import { mkdir, readFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { Type } from 'typebox';
import { getMarkdownTheme, withFileMutationQueue, type ExtensionAPI, type ExtensionContext } from '@earendil-works/pi-coding-agent';
import { Container, Image, Input, Markdown, SelectList, fuzzyFilter, truncateToWidth } from '@earendil-works/pi-tui';
import { Workflow } from './workflow.mjs';

const HELP = [
  "## figlens",
  "",
  "Ask Pi directly: **\"Build the List views screen from ./design.fig using this project's components.\"**",
  "Pi can find and read the design through MCP. No open command or node ID is needed.",
  "",
  "Optional manual controls:",
  "- `/figlens open ./design.fig` — search for a screen by name and save a selection.",
  "- `/figlens open \"./design files/mobile.fig\" --frame 12:345` — use a known node.",
  "- `/figlens frame` — search within the saved file; add an ID to select it directly.",
  "- `/figlens status` — inspect the saved selection.",
  "- `/figlens map` — request mapping proposals for the saved selection, without application edits.",
  "- `/figlens review` — review pending proposals across this project; no open required.",
  "- `/figlens close` — clear the saved selection, keeping reviewed mappings.",
  "",
  "Mappings are optional. Pi can implement the screen immediately.",
].join('\n');
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
    '', 'Mappings: .figlens/component-mappings.md', 'Mappings are optional. Ask Pi to implement the screen, or /figlens review to decide pending proposals.',
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
  pi.registerMessageRenderer('figlens-preview', message => {
    const container = new Container();
    for (const part of typeof message.content === 'string' ? [{ type: 'text' as const, text: message.content }] : message.content) {
      if (part.type === 'text') container.addChild(new Markdown(part.text, 1, 0, getMarkdownTheme()));
      else if (part.type === 'image') container.addChild(new Image(part.data, part.mimeType,
        { fallbackColor: text => text }, { maxWidthCells: 80, maxHeightCells: 24 }));
    }
    return container;
  });
  let workflow: Workflow | undefined;
  function project(ctx: ExtensionContext) {
    if (!workflow || workflow.root !== path.resolve(ctx.cwd)) {
      workflow = new Workflow(ctx.cwd, async (args: string[], signal?: AbortSignal) => {
        const execution = await pi.exec(binary, [...prefix, ...args], { cwd: ctx.cwd, signal, timeout: 120000 });
        if (execution.code !== 0 || execution.killed) {
          throw new Error(`Could not ${args[0]} the design: ${execution.stderr.trim().slice(0, 2000) || 'cancelled or timed out'}`);
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
    const directory = path.join(project(ctx).directory, 'previews');
    await mkdir(directory, { recursive: true });
    const output = path.join(directory, `${randomUUID()}.png`);
    const execution = await pi.exec(binary, [...prefix, 'export', design.absolute, design.frame, output, '--scale', '0.5'], { cwd: ctx.cwd, signal: ctx.signal, timeout: 120000 });
    if (execution.code !== 0 || execution.killed) throw new Error(execution.stderr || 'Preview render cancelled or timed out.');
    const bytes = await readFile(output);
    if (bytes.length > 5 * 1024 * 1024) throw new Error('Preview exceeds 5 MiB. Use MCP export with a smaller scale.');
    pi.sendMessage({
      customType: 'figlens-preview', display: true,
      content: [
        { type: 'text', text: `**${design.name}** · ${design.file}\n[Open reference image](${pathToFileURL(output).href})\n${design.instances.length ? `${design.instances.length} component instances available for optional mapping.` : 'This scope has no component instances to map. Pi can still read its layout and build it.'}` },
        { type: 'image', mimeType: 'image/png', data: bytes.toString('base64') },
      ],
    });
  }

  async function pickFrame(frames: { id: string; name: string; size?: { width: number; height: number } }[], ctx: ExtensionContext) {
    const items = frames.map(frame => ({ value: frame.id, label: frame.name,
      description: `${frame.size ? `${Math.round(frame.size.width)} × ${Math.round(frame.size.height)} · ` : ''}${frame.id}` }));
    if (ctx.mode !== 'tui') {
      let offset = 0;
      while (true) {
        const page = items.slice(offset, offset + 6);
        const labels = page.map(item => `${item.label} · ${item.description}`);
        const choice = await ctx.ui.select(`Matching screens (${offset + 1}–${offset + page.length} of ${items.length})`,
          [...labels, ...(offset ? ['Previous page'] : []), ...(offset + 6 < items.length ? ['Next page'] : [])]);
        if (choice === 'Next page') offset += 6;
        else if (choice === 'Previous page') offset -= 6;
        else return page[labels.indexOf(choice ?? '')]?.value;
      }
    }
    return ctx.ui.custom<string | undefined>((tui, theme, kb, done) => {
      const input = new Input();
      input.focused = true;
      let filtered = items;
      let list: SelectList;
      let rows = 0;
      const rebuild = () => {
        const selected = list?.getSelectedItem()?.value;
        rows = Math.max(1, Math.min(10, tui.terminal.rows - 9));
        list = new SelectList(filtered, rows, {
          selectedPrefix: text => theme.fg('accent', text), selectedText: text => theme.fg('accent', text),
          description: text => theme.fg('muted', text), scrollInfo: text => theme.fg('dim', text), noMatch: text => theme.fg('warning', text),
        });
        list.setSelectedIndex(Math.max(0, filtered.findIndex(item => item.value === selected)));
        list.onSelect = item => done(item.value);
        list.onCancel = () => done(undefined);
      };
      rebuild();
      return {
        render(width) {
          if (rows !== Math.max(1, Math.min(10, tui.terminal.rows - 9))) rebuild();
          return [truncateToWidth(theme.fg('accent', 'Select a screen · type to filter'), width), ...input.render(width),
            ...list.render(width), truncateToWidth('↑↓ / wheel: navigate · Enter: select · Esc: cancel', width)];
        },
        handleInput(data) {
          if (['tui.select.up', 'tui.select.down', 'tui.select.confirm', 'tui.select.cancel'].some(key => kb.matches(data, key as Parameters<typeof kb.matches>[1]))) list.handleInput(data);
          else {
            input.handleInput(data);
            filtered = fuzzyFilter(items, input.getValue(), item => `${item.label} ${item.description}`);
            rebuild();
          }
          tui.requestRender();
        },
        handleMouse(event) { return list.handleMouse({ ...event, y: event.y - 2 }); },
        invalidate() { input.invalidate(); list.invalidate(); },
      };
    });
  }

  async function chooseFrame(file: string, node: string | undefined, ctx: ExtensionContext) {
    const flow = project(ctx);
    if (!node) {
      if (!ctx.hasUI) { show('Ask Pi to find the screen by name using the figlens MCP tools, or provide a known node with /figlens open <file.fig> --frame <node-id>.', ctx); return; }
      const query = await ctx.ui.input('Find a screen by name', 'For example: List views. Or ask Pi to find the screen for you.');
      if (query === undefined) return;
      if (!query.trim()) { show('Enter a screen name to search, or ask Pi to discover the design from your request.', ctx); return; }
      const { frames, truncated } = await flow.frames(file, ctx.signal, query.trim());
      if (!frames.length) { show(`No frames match **${query}**. Try another name, or ask Pi to search visible text and inspect the surrounding frame.`, ctx); return; }
      if (truncated) ctx.ui.notify('Showing the first 200 matches. Search a more specific name to reach other screens.', 'warning');
      node = await pickFrame(frames, ctx);
      if (!node) return;
    }
    const design = await flow.select(file, node, ctx.signal);
    show(`Saved selection: **${design.name}** (${design.frame})\n${design.file}\nAsk Pi to implement it. Mapping is optional; a new request can name a different file or screen.`, ctx);
    if (ctx.mode === 'tui') ctx.ui.setStatus('figlens', `figlens · ${design.frame}`);
    if (ctx.hasUI) {
      try { await preview(design, ctx); }
      catch (error) { ctx.ui.notify(`Frame selected; preview unavailable: ${(error as Error).message}`, 'warning'); }
    }
  }

  async function review(ctx: ExtensionContext) {
    const flow = project(ctx);
    const pending = await flow.pending();
    if (!pending.length) { show('No pending component mappings in this project. Ask Pi to implement your screen, or to propose mappings if you want reusable decisions.', ctx); return; }
    if (!ctx.hasUI) { show(`${pending.length} pending mapping(s). Review requires interactive Pi or an RPC client supporting dialogs. Proposals remain pending.`, ctx); return; }
    for (const entry of pending) {
      const summary = [
        `${entry.source.name} (${entry.source.instance}) → ${entry.target.path}`,
        `Design: ${entry.source.file} · frame ${entry.source.frame}`,
        `Export: ${entry.target.export}`, entry.target.import, entry.target.example,
        `Rationale: ${entry.rationale}`,
        `Agent-reported checks (not independently verified): ${entry.checks.join('; ') || 'none'}`,
        ...entry.issues.map((issue: string) => `Needs review: ${issue}`),
      ].join('\n\n');
      show(summary, ctx);
      const action = await ctx.ui.select('Review this component mapping', ['Accept', 'Edit', 'Skip', 'Stop']);
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
      if (action === 'Edit') show(`Edited target:\n\n${JSON.stringify(target, null, 2)}`, ctx);
      if (resolution) show(`Conflict resolution:\n\n${resolution}`, ctx);
      if (!await ctx.ui.confirm('Approve the mapping shown above?', `${entry.source.name} → ${target.path}\n\nApproval records your mapping decision; compilation and visual checks remain separate.`)) continue;
      await flow.review(entry.id, 'approve', { ...(action === 'Edit' ? { target } : {}), resolution }, ctx.signal);
    }
    show(`Review finished. ${(await flow.pending()).length} proposal(s) remain pending. Approved mappings are available to Pi for their matching file and frame.`, ctx);
  }

  pi.on('session_start', async (_event, ctx) => {
    pi.registerMcpServer('figlens', { command: binary, args: [...prefix, 'mcp'], cwd: ctx.cwd, exposure: 'direct' });
    const active = (await project(ctx).read()).state.active;
    if (active && ctx.mode === 'tui') ctx.ui.setStatus('figlens', `figlens · ${active.frame}`);
  });

  pi.registerCommand('figlens', {
    description: 'Help and optional manual design selection or component mapping review; ask Pi directly to build a .fig screen',
    handler: async (args, ctx) => {
      const match = args.trim().match(/^(\S+)(?:\s+([\s\S]*))?$/);
      const command = match?.[1];
      try {
        if (!command || command === 'help' || command === '--help') { show(HELP, ctx); return; }
        if (!['open', 'frame', 'map', 'review', 'status', 'close'].includes(command)) {
          show(`Unknown figlens command: **${command}**.\n\n${HELP}`, ctx); return;
        }
        let rest = match?.[2]?.trim() || '';
        const frameOption = command === 'open' ? rest.match(/^([\s\S]+)\s+--frame\s+(\S+)$/) : undefined;
        if (frameOption) rest = frameOption[1].trim();
        if ((rest.startsWith('"') && rest.endsWith('"')) || (rest.startsWith("'") && rest.endsWith("'"))) rest = rest.slice(1, -1);
        await ctx.waitForIdle();
        const flow = project(ctx);
        if (rest && !['open', 'frame'].includes(command)) throw new Error(`/figlens ${command} takes no arguments. To implement a design, send your request as a normal message to Pi.`);
        if (command === 'open') {
          if (!rest) throw new Error('Provide a saved .fig file, for example: /figlens open ./design.fig. You can also ask Pi to build a screen directly.');
          const { file } = await flow.source(rest);
          await chooseFrame(file, frameOption?.[2], ctx);
        } else if (command === 'frame') {
          const active = (await flow.read()).state.active;
          if (!active) throw new Error('No manually saved file. Ask Pi to find the screen from your .fig, or use /figlens open ./design.fig to save a selection.');
          await chooseFrame(active.file, rest || undefined, ctx);
        } else if (command === 'map') {
          if (!(await flow.read()).state.active) {
            show('No manually saved screen. Ask Pi to propose component matches for a named screen in your .fig, or use /figlens open ./design.fig as a manual shortcut.', ctx);
            return;
          }
          const context = await flow.bindings({}, ctx.signal);
          pi.sendUserMessage(`Propose reusable component mappings for this selected design.\n${JSON.stringify(context)}\nUse figlens_bindings pagination to examine the current frame, then the figlens MCP tools for design details. Read actual project source, imports, props, slots and usage examples. Submit candidates with figlens_propose_bindings; report conflicts explicitly. Include implementation and supporting files in dependencies. Report checks exactly as executed; run only checks allowed by the project's instructions. Keep proposals pending for /figlens review. Work only on this frame; this request does not authorize application edits.`);
        } else if (command === 'review') await review(ctx);
        else if (command === 'status') {
          if ((await flow.read()).state.active) show(describe(await flow.bindings({}, ctx.signal)), ctx);
          else show('No manually saved selection. Pi can read a .fig directly from your request. Use /figlens review for pending mappings.', ctx);
        }
        else if (command === 'close') {
          await flow.close();
          if (ctx.mode === 'tui') ctx.ui.setStatus('figlens', undefined);
          show('Saved selection cleared. Component mappings are kept.', ctx);
        }
      } catch (error) {
        const detail = (error as Error).message;
        const hint = (error as NodeJS.ErrnoException).code === 'ENOENT'
          ? 'Check that the file exists and the path is relative to this project, or use an absolute path.'
          : 'Ask Pi to inspect the problem with the figlens MCP tools. /figlens help shows the optional manual controls.';
        show(`**Could not complete /figlens ${command}.**\n\n${detail}\n\n${hint}`, ctx);
      }
    },
  });

  pi.registerTool({
    name: 'figlens_bindings', label: 'Design component mappings',
    description: 'Optional project component reuse: pass file+frame from the current user request to read instances and approved/pending/stale mappings without open or saving a selection. Discover frames with figlens MCP find/inspect. Mapping is not required to build UI; zero instances is not a design-read failure. Omitted file+frame use the manual selection only. Results paginate with nextOffset; current text supersedes example literals. Hashes cover recorded files, not correctness.',
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
