import { mkdir, readFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { Type } from 'typebox';
import { getMarkdownTheme, withFileMutationQueue, type ExtensionAPI, type ExtensionContext } from '@earendil-works/pi-coding-agent';
import { Container, Image, Input, Markdown, SelectList, fuzzyFilter, truncateToWidth } from '@earendil-works/pi-tui';
import { Workflow } from './workflow.mjs';
import { Viewer } from './viewer.mjs';
import { buildRequest } from './requests.mjs';
import { ENTRY, Targets, bounded, changesArgs, check, failure, notice, reader, restore as restoreTargets } from './stale.mjs';

const HELP = [
  "## figlens",
  "",
  "Ask Pi directly: **\"Build the List views screen from ./design.fig using this project's components.\"**",
  "Pi can find and read the design through MCP. No open command or node ID is needed.",
  "Asking Pi to find copy or a screen searches the .fig directly and stores nothing. For repeated searches, ask it to enable an index for a design and scope: it prepares a local index once (no setup command), reports the job until ready, then reuses it across restarts and hosts. Resetting that index needs your explicit request. Asking where a component, variable, style or image is used lists its exact usages the same way: from the .fig directly, or from that index once enabled. Across every design with an index, it also lists where a library component, style or variable is used, by the library reference that outgoing usages on a layer names. Asking it to follow the changes of a design tracks a scope of that index and lists what changed since the last change it handled; asking it to watch that scope picks up each save while its figlens server runs. Giving it a screenshot, or pointing at a found screen, and asking for screens that look like it searches that index by image once its visual layers are built (experimental; the image is not stored).",
  "",
  "Optional shortcuts:",
  "- `/figlens open [description]` — tell Pi what you know: a file, screen name, purpose or visible text. Pi finds it or asks a recognizable follow-up.",
  "- `/figlens preview [description]` — start the local interactive viewer for a design; Pi resolves names and returns its link.",
  "- `/figlens plan [request]` — plan only the requested region or pages before implementing.",
  "- `/figlens yolo [request]` — plan, implement and verify the requested scope, then remove only this run's temporary plan after success.",
  "",
  "Advanced: saved selections and reusable component matches (not required to build):",
  "- `/figlens open \"./design files/mobile.fig\" --frame 12:345` — use a known node.",
  "- `/figlens frame` — search within the saved file; add an ID to select it directly.",
  "- `/figlens status` — inspect the saved selection.",
  "- `/figlens map` — propose reusable component matches for the design in this conversation.",
  "- `/figlens review` — review pending proposals across this project; no open required.",
  "- `/figlens close` — stop the viewer and clear the saved selection, keeping reviewed mappings.",
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
  const viewer = new Viewer(binary, prefix);
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

  async function requestScope(request: string) {
    pi.sendUserMessage(await buildRequest('open', request, import.meta.url));
  }

  async function preview(design: Awaited<ReturnType<Workflow['design']>>, ctx: ExtensionContext) {
    const directory = await project(ctx).referenceDirectory();
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
      if (!ctx.hasUI) { await requestScope(file); return; }
      const query = await ctx.ui.input('Find a screen by name', 'For example: List views. Or ask Pi to find the screen for you.');
      if (query === undefined) return;
      if (!query.trim()) { await requestScope(file); return; }
      const { frames, truncated } = await flow.frames(file, ctx.signal, query.trim());
      if (!frames.length) { await requestScope(`${file}\nThe user described the screen as: ${query}`); return; }
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
  // Watch targets this session subscribed to or acknowledged, kept in the session branch so a resumed session
  // checks them again. The check runs the same binary as the figlens MCP server registered above, which has no
  // --root, so it reads with that tool's authority; it never acks, builds or starts a watch.
  let targets = new Targets();
  let note: string | undefined;
  let checks = new AbortController();
  // Readers still running; a shutdown or session change waits for them to exit (each is bounded).
  const reading = new Set<Promise<unknown>>();
  const stopChecks = async () => { checks.abort(); await Promise.allSettled([...reading]); };
  pi.on('session_shutdown', async () => { await stopChecks(); await viewer.close(); });
  const restore = async (_event: unknown, ctx: ExtensionContext) => {
    await stopChecks();
    checks = new AbortController();
    targets = restoreTargets(ctx.sessionManager.getBranch());
    note = undefined;
    if (ctx.mode === 'tui') ctx.ui.setStatus('figlens-changes', undefined);
  };
  pi.on('session_start', restore);
  pi.on('session_tree', restore);
  pi.on('tool_result', async event => {
    if (targets.observe(event.toolName, event.input, event.content, event.isError)) pi.appendEntry(ENTRY, [...targets.list]);
  });
  // Each user prompt checks again; programmatic runs keep the last note.
  pi.on('before_agent_start', async (event, ctx) => {
    if (event.messageType === 'custom') return;
    note = targets.list.length ? notice(await check(targets.turn(), async (target, signal) => {
      // pi.exec buffers a whole reply, so the check reads through a byte-bounded child of the native binary instead.
      const [command, ...first] = reader();
      const run = bounded(command, [...first, ...changesArgs(target)], { cwd: ctx.cwd, signal });
      reading.add(run);
      run.then(() => reading.delete(run), () => reading.delete(run));
      const execution = await run;
      if (execution.code !== 0) throw failure(execution.stderr);
      return JSON.parse(execution.stdout);
    }, AbortSignal.any([checks.signal, ...(ctx.signal ? [ctx.signal] : [])])), targets.dropped) : undefined;
    if (ctx.mode === 'tui') ctx.ui.setStatus('figlens-changes', note ? 'figlens · design context may be stale' : undefined);
  });
  // The note joins each model call of this run without entering the session, so a processed or revoked
  // target leaves no copy behind, compaction cannot drop it, and the next prompt replaces it.
  pi.on('context', async event => note
    ? { messages: [...event.messages, { role: 'custom', customType: 'figlens', content: note, display: false, timestamp: Date.now() }] }
    : undefined);

  const COMMANDS = {
    open: 'Find a design from a file, screen name, purpose or visible text',
    preview: 'Start the local viewer and return its link',
    plan: 'Inspect the design and relevant code, write a plan; no edits',
    yolo: 'Plan, implement, verify and clean up the temporary plan',
    frame: 'Search the saved file, or select a frame id directly',
    map: 'Propose reusable component matches for the current design',
    review: 'Accept, edit or skip pending component mappings',
    status: 'Show the manual selection and mapping states',
    close: 'Stop the viewer and clear the manual selection',
    help: 'List these commands',
  };
  pi.registerCommand('figlens', {
    description: 'Plan or implement a design, select a scope or review component mappings',
    getArgumentCompletions: prefix => Object.entries(COMMANDS)
      .filter(([value]) => value.startsWith(prefix)).map(([value, description]) => ({ value, label: value, description })),
    handler: async (args, ctx) => {
      const match = args.trimStart().match(/^(\S+)(?:\s([\s\S]*))?$/);
      const command = match?.[1];
      try {
        if (!command || command === 'help' || command === '--help') { show(HELP, ctx); return; }
        if (!Object.hasOwn(COMMANDS, command)) {
          show(`Unknown figlens command: **${command}**.\n\n${HELP}`, ctx); return;
        }
        let rest = match?.[2]?.trim() || '';
        if (command === 'preview') {
          await ctx.waitForIdle();
          const file = ((rest.startsWith('"') && rest.endsWith('"')) || (rest.startsWith("'") && rest.endsWith("'"))) ? rest.slice(1, -1) : rest;
          const absolute = path.resolve(ctx.cwd, file);
          if (file && path.extname(file).toLowerCase() === '.fig' && existsSync(absolute)) {
            const started = await viewer.start(absolute, undefined, { cwd: ctx.cwd, signal: ctx.signal });
            show(`[Open design viewer](${started.url})`, ctx);
            return;
          }
          pi.sendUserMessage(await buildRequest('preview', rest, import.meta.url));
          return;
        }
        if (command === 'plan') {
          await ctx.waitForIdle();
          pi.sendUserMessage(await buildRequest('plan', rest, import.meta.url));
          return;
        }
        if (command === 'yolo') {
          await ctx.waitForIdle();
          pi.sendUserMessage(await buildRequest('yolo', match?.[2] || '', import.meta.url));
          return;
        }
        const frameOption = command === 'open' ? rest.match(/^([\s\S]+)\s+--frame\s+(\S+)$/) : undefined;
        if (frameOption) rest = frameOption[1].trim();
        if ((rest.startsWith('"') && rest.endsWith('"')) || (rest.startsWith("'") && rest.endsWith("'"))) rest = rest.slice(1, -1);
        await ctx.waitForIdle();
        if (command === 'open' && !frameOption) { await requestScope(rest); return; }
        const flow = project(ctx);
        if (rest && !['open', 'frame'].includes(command)) throw new Error(`/figlens ${command} takes no arguments. To implement a design, send your request as a normal message to Pi.`);
        if (command === 'open') {
          if (!rest) throw new Error('Provide a saved .fig file, for example: /figlens open ./design.fig. You can also ask Pi to build a screen directly.');
          const { file } = await flow.source(rest);
          await chooseFrame(file, frameOption?.[2], ctx);
        } else if (command === 'frame') {
          const active = (await flow.read()).state.active;
          if (!active) throw new Error('No manually saved file. Use /figlens open and describe the screen, or ask Pi directly. Known IDs are optional advanced controls in /figlens help.');
          await chooseFrame(active.file, rest || undefined, ctx);
        } else if (command === 'map') {
          pi.sendUserMessage(await buildRequest('map', '', import.meta.url));
        } else if (command === 'review') await review(ctx);
        else if (command === 'status') {
          if ((await flow.read()).state.active) show(describe(await flow.bindings({}, ctx.signal)), ctx);
          else show('No manually saved selection. Pi can read a .fig directly from your request. Use /figlens review for pending mappings.', ctx);
        }
        else if (command === 'close') {
          await viewer.close();
          await flow.close();
          if (ctx.mode === 'tui') ctx.ui.setStatus('figlens', undefined);
          show('Local viewer stopped and saved selection cleared. Component mappings are kept.', ctx);
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
    name: 'figlens_preview', label: 'Open design viewer',
    description: 'Start the local interactive viewer for a local .fig file, optionally focused on an exact node. Returns its loopback URL; does not launch a browser or modify the application, manual selection or mappings. May write render cache. Replaces this session’s previous viewer after successful startup; viewers stop when the Pi session shuts down.',
    parameters: Type.Object({
      file: Type.String({ minLength: 1, description: 'Local .fig path resolved from the current request.' }),
      node: Type.Optional(Type.String({ minLength: 1, description: 'Exact node ID to focus; omit to browse the whole file.' })),
    }),
    outputSchema: Type.Object({}, { additionalProperties: true }),
    annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
    executionMode: 'sequential',
    execute: async (_id, params, signal, _update, ctx) => result(await viewer.start(path.resolve(ctx.cwd, params.file), params.node, { cwd: ctx.cwd, signal })),
  });

  pi.registerTool({
    name: 'figlens_bindings', label: 'Design component mappings',
    description: 'Read a frame’s component instances and this project’s recorded mappings (approved, pending, stale or unmatched). Supply file and frame together to read that scope without changing the manual selection; omit both to use the saved selection. Returns current variant properties/text, mapped targets with import/usage examples, and nextOffset when more instances remain. Zero instances returns an empty list, not a read failure. Read-only; never approves a mapping. Recorded hashes cover listed evidence files, not correctness.',
    parameters: Type.Object({
      ...selection,
      offset: Type.Optional(Type.Integer({ minimum: 0, description: 'First instance index, default 0; pass nextOffset from the previous result.' })),
      limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 50, description: 'Instances per page, 1-50, default 20.' })),
    }),
    outputSchema: Type.Object({}, { additionalProperties: true }),
    annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: false },
    executionMode: 'sequential',
    async execute(_id, params, signal, _onUpdate, ctx) { return result(await project(ctx).bindings(params, signal)); },
  });

  pi.registerTool({
    name: 'figlens_propose_bindings', label: 'Propose component mappings',
    description: 'Record 1-20 candidate mappings from design instances to existing project implementations as pending for human review with /figlens review. Stores design identity, variant/structure and file hashes; checks are agent-reported results, not independent proof. This call cannot approve mappings. Supply file and frame together to override the saved scope for this call only.',
    parameters: Type.Object({
      ...selection,
      proposals: Type.Array(Type.Object({
        instance: Type.String({ minLength: 1, maxLength: 200, description: 'Instance ID in this frame, as returned by figlens_bindings.' }), target: Target,
        dependencies: Type.Optional(Type.Array(Type.String({ minLength: 1, maxLength: 2000 }), { maxItems: 20, description: 'Supporting project files, relative to the project root; their hashes are recorded with the target file.' })),
        rationale: Type.String({ minLength: 1, maxLength: 2000, description: 'Why the actual implementation and its usage match this instance.' }),
        checks: Type.Optional(Type.Array(Type.String({ minLength: 1, maxLength: 2000 }), { maxItems: 20, description: 'Checks actually executed for this mapping, each with its result; omit checks not run.' })),
        issues: Type.Optional(Type.Array(Type.String({ minLength: 1, maxLength: 2000 }), { maxItems: 10, description: 'Unresolved conflicts for review, such as design text the implementation does not expose.' })),
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
