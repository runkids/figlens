import { mkdir, readFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { getMarkdownTheme, withFileMutationQueue, type ExtensionAPI, type ExtensionContext } from '@oh-my-pi/pi-coding-agent';
import { Container, Image, Markdown } from '@oh-my-pi/pi-tui';
import { Workflow } from '../pi/workflow.mjs';
import { Viewer } from '../pi/viewer.mjs';
import { buildRequest } from '../pi/requests.mjs';
import { reviewMappings } from './review.mjs';

const HELP = [
  "## figlens",
  "",
  "Ask OMP directly: **\"Build the List views screen from ./design.fig using this project's components.\"**",
  "OMP can find and read the design through MCP. No open command or node ID is needed.",
  "Asking OMP to find copy or a screen searches the .fig directly and stores nothing. For repeated searches, ask it to enable an index for a design and scope: it prepares a local index once (no setup command), reports the job until ready, then reuses it across restarts and hosts. Resetting that index needs your explicit request. Asking where a component, variable, style or image is used lists its exact usages the same way: from the .fig directly, or from that index once enabled. Across every design with an index, it also lists where a library component, style or variable is used, by the library reference that outgoing usages on a layer names. Asking it to follow the changes of a design tracks a scope of that index and lists what changed since the last change it handled; asking it to watch that scope picks up each save while its figlens server runs.",
  "",
  "Optional shortcuts:",
  "- `/figlens open [description]` — tell OMP what you know: a file, screen name, purpose or visible text. OMP finds it or asks a recognizable follow-up.",
  "- `/figlens preview [description]` — start the local interactive viewer for a design; OMP resolves names and returns its link.",
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
  "MCP-dependent shortcuts currently report an unverified server. Use local controls below or the shared skill in ordinary conversation.",
  "Mappings are optional. OMP can implement the screen immediately.",
].join('\n');
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
    '', 'Mappings: .figlens/component-mappings.md', 'Mappings are optional. Ask OMP to implement the screen, or /figlens review to decide pending proposals.',
  ].join('\n');
}

export default function figlens(pi: ExtensionAPI) {
  const wrapper = fileURLToPath(new URL('../bin/figlens.js', import.meta.url));
  const bundled = !process.env.FIGLENS_BIN && existsSync(wrapper);
  const binary = process.env.FIGLENS_BIN || (bundled ? process.execPath : 'figlens');
  const prefix = bundled ? [wrapper] : [];
  const z = pi.zod;
  const selection = { file: z.string().min(1).max(4096).optional(), frame: z.string().min(1).max(200).optional() };
  const Target = z.object({ path: z.string().min(1).max(4096), export: z.string().min(1).max(200),
    import: z.string().min(1).max(8000), example: z.string().min(1).max(8000) });
  const owners = new Map<string, { root: string; viewer: Viewer; lifetime: AbortController }>();
  function owner(ctx: ExtensionContext) {
    const id = ctx.sessionManager.getSessionId();
    let current = owners.get(id);
    if (current && current.root !== path.resolve(ctx.cwd)) throw new Error('Session project changed; reload the extension before continuing.');
    if (!current) {
      const lifetime = new AbortController();
      const root = path.resolve(ctx.cwd);
      current = { root, viewer: new Viewer(binary, prefix), lifetime };
      owners.set(id, current);
    }
    return current;
  }
  function project(ctx: ExtensionContext, signal?: AbortSignal) {
    const current = owner(ctx);
    const combined = signal ? AbortSignal.any([current.lifetime.signal, signal]) : current.lifetime.signal;
    return new Workflow(current.root, async (args: string[]) => {
      combined.throwIfAborted();
      const execution = await pi.exec(binary, [...prefix, ...args], { cwd: current.root, signal: combined, timeout: 120000 });
      if (execution.code !== 0 || execution.killed) throw new Error(execution.stderr.trim().slice(0, 2000) || 'CLI cancelled or timed out.');
      return JSON.parse(execution.stdout);
    }, async () => { combined.throwIfAborted(); });
  }
  async function closeOwners() {
    for (const current of owners.values()) current.lifetime.abort();
    await Promise.all([...owners.values()].map(current => current.viewer.close()));
    owners.clear();
  }
  function requireMcp() {
    const names = pi.getAllTools().filter(tool => tool.sourceInfo.source === 'mcp').map(tool => tool.name);
    throw new Error(`MCP shortcut unavailable: OMP does not expose the effective server version or snapshot to extensions. Available MCP tools: ${names.join(', ') || 'none'}. Use the figlens skill directly, or use local preview, frame, status and review commands.`);
  }
  async function sendRequest(kind: string, request: string) {
    requireMcp();
    pi.sendUserMessage(await buildRequest(kind, request, import.meta.url));
  }
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
  function show(data: unknown, ctx: ExtensionContext) {
    pi.sendMessage({ customType: 'figlens', content: typeof data === 'string' ? data : JSON.stringify(data, null, 2), display: true });
  }

  async function requestScope(request: string) {
    await sendRequest('open', request);
  }

  async function preview(design: Awaited<ReturnType<Workflow['design']>>, ctx: ExtensionContext) {
    const directory = await project(ctx).referenceDirectory();
    await mkdir(directory, { recursive: true });
    const output = path.join(directory, `${randomUUID()}.png`);
    const execution = await pi.exec(binary, [...prefix, 'export', design.absolute, design.frame, output, '--scale', '0.5'], { cwd: ctx.cwd, signal: owner(ctx).lifetime.signal, timeout: 120000 });
    if (execution.code !== 0 || execution.killed) throw new Error(execution.stderr || 'Preview render cancelled or timed out.');
    const bytes = await readFile(output);
    if (bytes.length > 5 * 1024 * 1024) throw new Error('Preview exceeds 5 MiB. Use MCP export with a smaller scale.');
    pi.sendMessage({
      customType: 'figlens-preview', display: true,
      content: [
        { type: 'text', text: `**${design.name}** · ${design.file}\n[Open reference image](${pathToFileURL(output).href})\n${design.instances.length ? `${design.instances.length} component instances available for optional mapping.` : 'This scope has no component instances to map. OMP can still read its layout and build it.'}` },
        { type: 'image', mimeType: 'image/png', data: bytes.toString('base64') },
      ],
    });
  }

  async function pickFrame(frames: { id: string; name: string; size?: { width: number; height: number } }[], ctx: ExtensionContext) {
    const items = frames.map(frame => ({ value: frame.id, label: frame.name,
      description: `${frame.size ? `${Math.round(frame.size.width)} × ${Math.round(frame.size.height)} · ` : ''}${frame.id}` }));
    {
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
  }

  async function chooseFrame(file: string, node: string | undefined, ctx: ExtensionContext) {
    const flow = project(ctx);
    if (!node) {
      if (!ctx.hasUI) { await requestScope(file); return; }
      const query = await ctx.ui.input('Find a screen by name', 'For example: List views. Or ask OMP to find the screen for you.');
      if (query === undefined) return;
      if (!query.trim()) { await requestScope(file); return; }
      const { frames, truncated } = await flow.frames(file, owner(ctx).lifetime.signal, query.trim());
      if (!frames.length) { await requestScope(`${file}\nThe user described the screen as: ${query}`); return; }
      if (truncated) ctx.ui.notify('Showing the first 200 matches. Search a more specific name to reach other screens.', 'warning');
      node = await pickFrame(frames, ctx);
      if (!node) return;
    }
    const design = await flow.select(file, node, owner(ctx).lifetime.signal);
    show(`Saved selection: **${design.name}** (${design.frame})\n${design.file}\nAsk OMP to implement it. Mapping is optional; a new request can name a different file or screen.`, ctx);
    if (ctx.mode === 'tui') ctx.ui.setStatus('figlens', `figlens · ${design.frame}`);
    if (ctx.hasUI) {
      try { await preview(design, ctx); }
      catch (error) { ctx.ui.notify(`Frame selected; preview unavailable: ${(error as Error).message}`, 'warning'); }
    }
  }

  pi.on('session_switch', closeOwners);
  pi.on('session_shutdown', closeOwners);

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
            const started = await owner(ctx).viewer.start(absolute, undefined, { cwd: ctx.cwd, signal: owner(ctx).lifetime.signal });
            show(`[Open design viewer](${started.url})`, ctx);
            return;
          }
          await sendRequest('preview', rest);
          return;
        }
        if (command === 'plan') {
          await ctx.waitForIdle();
          await sendRequest('plan', rest);
          return;
        }
        if (command === 'yolo') {
          await ctx.waitForIdle();
          await sendRequest('yolo', match?.[2] || '');
          return;
        }
        const frameOption = command === 'open' ? rest.match(/^([\s\S]+)\s+--frame\s+(\S+)$/) : undefined;
        if (frameOption) rest = frameOption[1].trim();
        if ((rest.startsWith('"') && rest.endsWith('"')) || (rest.startsWith("'") && rest.endsWith("'"))) rest = rest.slice(1, -1);
        await ctx.waitForIdle();
        if (command === 'open' && !frameOption) { await requestScope(rest); return; }
        const flow = project(ctx);
        if (rest && !['open', 'frame'].includes(command)) throw new Error(`/figlens ${command} takes no arguments. To implement a design, send your request as a normal message to OMP.`);
        if (command === 'open') {
          if (!rest) throw new Error('Provide a saved .fig file, for example: /figlens open ./design.fig. You can also ask OMP to build a screen directly.');
          const { file } = await flow.source(rest);
          await chooseFrame(file, frameOption?.[2], ctx);
        } else if (command === 'frame') {
          const active = (await flow.read()).state.active;
          if (!active) throw new Error('No manually saved file. Use /figlens open and describe the screen, or ask OMP directly. Known IDs are optional advanced controls in /figlens help.');
          await chooseFrame(active.file, rest || undefined, ctx);
        } else if (command === 'map') {
          await sendRequest('map', '');
        } else if (command === 'review') await reviewMappings(flow, ctx, message => show(message, ctx), owner(ctx).lifetime.signal);
        else if (command === 'status') {
          if ((await flow.read()).state.active) show(describe(await flow.bindings({}, owner(ctx).lifetime.signal)), ctx);
          else show('No manually saved selection. OMP can read a .fig directly from your request. Use /figlens review for pending mappings.', ctx);
        }
        else if (command === 'close') {
          await owner(ctx).viewer.close();
          await flow.close();
          if (ctx.mode === 'tui') ctx.ui.setStatus('figlens', undefined);
          show('Local viewer stopped and saved selection cleared. Component mappings are kept.', ctx);
        }
      } catch (error) {
        const detail = (error as Error).message;
        const hint = (error as NodeJS.ErrnoException).code === 'ENOENT'
          ? 'Check that the file exists and the path is relative to this project, or use an absolute path.'
          : 'Ask OMP to inspect the problem with the figlens MCP tools. /figlens help shows the optional manual controls.';
        show(`**Could not complete /figlens ${command}.**\n\n${detail}\n\n${hint}`, ctx);
      }
    },
  });

  pi.registerTool({
    name: 'figlens_preview', label: 'Open design viewer',
    description: 'Start the local interactive viewer for a local .fig file, optionally focused on an exact node. Returns its loopback URL; does not launch a browser or modify the application, manual selection or mappings. May write render cache. Replaces this session’s previous viewer after successful startup; viewers stop when the OMP session shuts down.',
    parameters: z.object({ file: z.string().min(1).max(4096), node: z.string().min(1).max(200).optional() }),
    approval: 'exec',
    execute: async (_id, params, signal, _update, ctx) => result(await owner(ctx).viewer.start(path.resolve(ctx.cwd, params.file), params.node, { cwd: ctx.cwd, signal: signal ? AbortSignal.any([owner(ctx).lifetime.signal, signal]) : owner(ctx).lifetime.signal })),
  });

  pi.registerTool({
    name: 'figlens_bindings', label: 'Design component mappings',
    description: 'Read a frame’s component instances and this project’s recorded mappings (approved, pending, stale or unmatched). Supply file and frame together to read that scope without changing the manual selection; omit both to use the saved selection. Returns current variant properties/text, mapped targets with import/usage examples, and nextOffset when more instances remain. Zero instances returns an empty list, not a read failure. Read-only; never approves a mapping. Recorded hashes cover listed evidence files, not correctness.',
    parameters: z.object({ ...selection, offset: z.number().int().min(0).optional(), limit: z.number().int().min(1).max(50).optional() }),
    approval: 'read',
    async execute(_id, params, signal, _onUpdate, ctx) { return result(await project(ctx, signal).bindings(params, signal)); },
  });

  pi.registerTool({
    name: 'figlens_propose_bindings', label: 'Propose component mappings',
    description: 'Record 1-20 candidate mappings from design instances to existing project implementations as pending for human review with /figlens review. Stores design identity, variant/structure and file hashes; checks are agent-reported results, not independent proof. This call cannot approve mappings. Supply file and frame together to override the saved scope for this call only.',
    parameters: z.object({ ...selection, proposals: z.array(z.object({
      instance: z.string().min(1).max(200), target: Target,
      dependencies: z.array(z.string().min(1).max(2000)).max(20).optional(),
      rationale: z.string().min(1).max(2000), checks: z.array(z.string().min(1).max(2000)).max(20).optional(),
      issues: z.array(z.string().min(1).max(2000)).max(10).optional(),
    })).min(1).max(20) }),
    approval: 'write',
    async execute(_id, params, signal, _onUpdate, ctx) {
      const flow = project(ctx, signal);
      return result(await withFileMutationQueue(flow.guide, () => flow.propose(params, signal)));
    },
  });
}
