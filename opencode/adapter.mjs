import { execFile, spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, readFile, readlink, realpath, rm } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import path from 'node:path';
import { homedir, tmpdir } from 'node:os';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { promisify } from 'node:util';
import { Workflow } from '../pi/workflow.mjs';
import { stopProcess, Viewer } from '../pi/viewer.mjs';
import { buildRequest } from '../pi/requests.mjs';

const execute = promisify(execFile);
import { HELP } from './rpc.mjs';
import { createHolder } from './watch-holder.mjs';

// Under Bun-built hosts such as OpenCode, process.execPath is the host binary, which cannot run the Node launcher.
export function launcherRuntime(versions = process.versions, execPath = process.execPath) {
  return versions.bun ? 'node' : execPath;
}

const MANAGED_MCP = 'figlensmcp';

const flag = (name, value) => value === undefined || value === false ? [] : value === true ? [name] : [name, String(value)];
const required = (input, name) => {
  if (typeof input[name] !== 'string' || !input[name]) throw new Error(`${input.action ?? 'This call'} needs ${name}.`);
  return input[name];
};

// The MCP index, jobs, search, usages, changes, watch and inspect arguments as the same CLI command, for the plugin's CLI path.
export function cliArgs(tool, input) {
  const storage = flag('--storage', input.storage);
  if (tool === 'jobs') return ['jobs', input.action, required(input, 'jobId'), ...storage];
  if (tool === 'inspect') return ['inspect', required(input, 'file'), required(input, 'node'), ...flag('--depth', input.depth),
    ...flag('--images', input.images), ...flag('--states', input.states), ...flag('--flat', input.flat), ...flag('--compare-variant', input.compareVariant)];
  // As the MCP search and usages tools: one corpus, and source narrowing never silently disappears.
  const corpus = () => {
    if (!input.file === !input.workspaceId || (input.file && (input.sourceId || input.storage)))
      throw new Error(`${tool} needs file or workspaceId, not both; sourceId and storage apply to workspaceId`);
    return input.file ? [input.file] : ['--id', input.workspaceId, ...flag('--source', input.sourceId), ...storage];
  };
  if (tool === 'search') {
    const target = corpus();
    const flags = [...flag('--node', input.node), ...flag('--type', input.type), ...flag('--limit', input.limit),
      ...flag('--cursor', input.cursor), ...flag('--include-hidden', input.includeHidden),
      ...flag('--include-internal', input.includeInternal), ...flag('--mode', input.mode)];
    // Workspace search reads everything after -- as the query, so a query may start with '-'.
    return input.file ? ['search', ...target, required(input, 'query'), ...flags] : ['search', ...target, ...flags, '--', required(input, 'query')];
  }
  if (tool === 'usages') {
    const target = corpus();
    if (!input.node === !input.resource) throw new Error('usages needs node or resource, not both');
    // The CLI tells a resource from a layer by its prefix, so a mislabelled target would silently switch kinds; same messages as the core.
    const resource = /^(VariableID:|StyleKey:|image:)/.test(input.node ?? input.resource);
    if (input.resource && !resource) throw new Error('resource takes a VariableID:…, StyleKey:… or image:… id; pass a layer id as node');
    if (input.node && resource) throw new Error('node takes a layer id; pass VariableID:…, StyleKey:… or image:… as resource');
    return ['usages', ...target, input.node ?? input.resource, ...flag('--scope', input.scope), ...flag('--relation', input.relation),
      ...flag('--out', input.direction === 'out'), ...flag('--snapshot', input.snapshotId), ...flag('--include-hidden', input.includeHidden),
      ...flag('--limit', input.limit), ...flag('--cursor', input.cursor)];
  }
  // As the MCP changes and watch tools, which read one workspace by its id.
  const filters = () => ['--id', required(input, 'workspaceId'), ...flag('--source', input.sourceId), ...flag('--scope', input.scope), ...storage];
  if (tool === 'changes') return ['changes', ...filters(), ...flag('--consumer', input.consumer), ...flag('--subscription', input.subscriptionId),
    ...flag('--cursor', input.cursor), ...flag('--revision', input.revision), ...flag('--limit', input.limit)];
  // A start is a held `watch run` (see createHolder), which takes the scope and requestId.
  if (tool === 'watch' && input.action === 'start') return ['watch', 'run', '--id', required(input, 'workspaceId'), ...flag('--source', input.sourceId),
    '--scope', required(input, 'scope'), ...storage, ...flag('--request-id', input.requestId)];
  // The CLI refuses a flag the action does not take.
  if (tool === 'watch') return ['watch', input.action, ...filters(), ...flag('--consumer', input.consumer),
    ...flag('--subscription', input.subscriptionId), ...(input.action === 'ack' ? ['--cursor', required(input, 'cursor')] : flag('--cursor', input.cursor)),
    ...flag('--node', input.node), ...flag('--query', input.query), ...flag('--type', input.type), ...flag('--owner', input.owner),
    ...flag('--after', input.after)];
  // Every given selector and scope field reaches the CLI, which refuses conflicting selectors and fields an
  // action does not take, so none is dropped into a broader operation. A file is the operand of prepare, add
  // and relocate, else it selects the workspace; a sourceId is the operand of remove and relocate.
  const { action } = input;
  const operands = ['prepare', 'add'].includes(action) ? [required(input, 'file')]
    : action === 'remove' ? [required(input, 'sourceId')]
    : action === 'relocate' ? [required(input, 'sourceId'), required(input, 'file')] : [];
  return ['index', action, ...operands, ...flag('--workspace', input.workspace), ...flag('--id', input.workspaceId),
    ...(['prepare', 'add', 'relocate'].includes(action) ? [] : flag('--file', input.file)),
    ...(['remove', 'relocate'].includes(action) ? [] : flag('--source', input.sourceId)), ...storage,
    ...flag('--scope', input.scope), ...flag('--request-id', input.requestId), ...flag('--replace-unavailable', input.replaceUnavailable)];
}

// Index actions that delete a database or start a new one elsewhere; each needs its own host permission.
export function indexGrant(input) {
  return input.action === 'reset' ? 'reset' : input.replaceUnavailable === true ? 'replace_unavailable' : null;
}

export function createAdapter(ctx, entry = import.meta.url) {
  const wrapper = fileURLToPath(new URL('../bin/figlens.js', entry));
  const bundled = !process.env.FIGLENS_BIN && existsSync(wrapper);
  const command = process.env.FIGLENS_BIN || (bundled ? launcherRuntime() : 'figlens');
  const prefix = bundled ? [wrapper] : [];
  const cacheRoot = path.join(process.env.XDG_CACHE_HOME || (process.platform === 'win32' && process.env.LOCALAPPDATA) || path.join(homedir(), '.cache'), 'figlens');
  const commandLine = (args, argv = [command, ...prefix]) => [...argv, ...args].map(value => `'${value.replaceAll("'", "'\\''")}'`).join(' ');
  // A lease holder runs the launcher's cached native binary itself, so stopping the child stops the holder
  // on every platform (the launcher would leave it running); a prior `index status` installed and checked it.
  const holding = () => {
    if (!bundled) return [command, ...prefix];
    const require = createRequire(entry);
    const native = require('../target.js').cachedBinary(cacheRoot, process.platform, process.arch, require('../package.json').version);
    if (!existsSync(native)) throw new Error(`figlens's cached binary is missing at ${native}; run figlens_index status to install it, then start again`);
    return [native];
  };
  const owners = new Map();
  const lifetime = new AbortController();
  let managedMcp = false, cliPath = false;
  // OpenCode drops an MCP server whose name matches this plugin's own figlens_* tool namespace, so
  // the managed default uses another key; a user server under either key stays the one in use.
  let mcpName = MANAGED_MCP;
  const mcpConfig = { type: 'local', command: [command, ...prefix, 'mcp'], cwd: ctx.location.directory,
    ...(process.env.FIGLENS_BIN ? { environment: { FIGLENS_BIN: process.env.FIGLENS_BIN } } : {}) };

  async function close(sessionID) {
    const current = owners.get(sessionID);
    if (!current) return;
    current.lifetime.abort();
    try { await Promise.all([...current.commands].map(stop => stop())); }
    finally {
      // This session's leases only; the shared runner and other holders' leases are left alone.
      for (const end of await current.holder.stopAll())
        if (end.state !== 'ended') console.error('figlens watch lease not released:', JSON.stringify(end));
    }
    await current.viewer.close();
    owners.delete(sessionID);
  }
  async function scope(sessionID, signal) {
    lifetime.signal.throwIfAborted();
    const session = await ctx.session.get({ sessionID });
    const root = await realpath(ctx.location.directory);
    if (!session.location?.directory || await realpath(session.location.directory) !== root) {
      await close(sessionID);
      throw new Error('Session belongs to a different project directory.');
    }
    let current = owners.get(sessionID);
    if (!current) {
      current = { viewer: new Viewer(command, prefix), lifetime: new AbortController(), commands: new Set(),
        holder: createHolder(args => {
          const [run, ...first] = holding();
          return spawn(run, [...first, ...args], { cwd: root, stdio: 'pipe', windowsHide: true, detached: process.platform !== 'win32' });
        },
          { group: process.platform !== 'win32' }) };
      owners.set(sessionID, current);
    }
    const combined = AbortSignal.any([lifetime.signal, current.lifetime.signal, ...(signal ? [signal] : [])]);
    const admitted = new Set();
    async function authorize(action, resources) {
      combined.throwIfAborted();
      const remaining = resources.filter(resource => !admitted.has(JSON.stringify([action, resource])));
      if (remaining.length) await ctx.permission.assert({ sessionID, action, resources: remaining });
      combined.throwIfAborted();
      for (const resource of remaining) admitted.add(JSON.stringify([action, resource]));
    }
    // A foreground index build runs until its jobs finish, so it gets no timeout of its own.
    async function run(args, timeout = 120000) {
      const argv = [...prefix, ...args];
      if (bundled) await authorize('edit', [cacheRoot]);
      await authorize('bash', [commandLine(args)]);
      const signal = timeout ? AbortSignal.any([combined, AbortSignal.timeout(timeout)]) : combined;
      const execution = execute(command, argv, { cwd: root, detached: process.platform !== 'win32', maxBuffer: 64 * 1024 * 1024 });
      let stopping;
      const abort = () => {
        stopping ??= stopProcess(execution.child);
        stopping.catch(() => {});
        return stopping;
      };
      signal.addEventListener('abort', abort, { once: true });
      if (signal.aborted) abort();
      const finished = execution.finally(async () => {
        signal.removeEventListener('abort', abort);
        await stopping;
      });
      current.commands.add(abort);
      try {
        const result = await finished;
        signal.throwIfAborted();
        return result;
      } catch (error) { signal.throwIfAborted(); throw error; }
      finally { current.commands.delete(abort); }
    }
    return { root, ...current, signal: combined, authorize, run,
      flow: new Workflow(root, async args => JSON.parse((await run(args)).stdout), authorize) };
  }
  function configureMcp(editor) {
    mcpName = [MANAGED_MCP, 'figlens'].find(name => editor.get(name)) ?? MANAGED_MCP;
    const config = editor.get(mcpName);
    managedMcp = !config;
    if (managedMcp) editor.set(mcpName, mcpConfig);
    // Only a disabled user server, or one OpenCode drops for its figlens name, leaves the search flow without MCP tools.
    cliPath = !managedMcp && (config.enabled === false || config.disabled === true || mcpName === 'figlens');
  }
  async function mcpStatus() {
    const { data } = await ctx.mcp.list();
    const server = data.find(server => server.name === mcpName);
    const tools = await ctx.tool.list();
    return { server: mcpName, managed: managedMcp, status: server?.status ?? { status: 'pending' },
      tools: tools.filter(tool => tool.name.startsWith('figlens_')).map(tool => tool.name),
      catalog: tools.filter(tool => tool.options?.namespace === mcpName).map(tool => tool.id),
      reason: managedMcp ? 'Package launcher; local project paths.' : `Existing configuration preserved; server version and snapshot are unverified. MCP shortcuts are blocked.${mcpName === 'figlens' ? ` OpenCode drops a server named figlens because it collides with this plugin's figlens_* tools; rename it to ${MANAGED_MCP}.` : ''}` };
  }
  // OpenCode reports MCP connected before its tool registry reloads the server's tools; a turn started
  // in between gets a Code Mode catalog without them, so wait for the registry, not the connection.
  async function requireMcp(current) {
    let status = await mcpStatus();
    for (let attempt = 0; attempt < 40 && status.managed && status.status.status === 'connected' && !status.catalog.length; attempt++) {
      await delay(250, undefined, { signal: current.signal });
      status = await mcpStatus();
    }
    if (!status.managed || status.status.status !== 'connected') throw new Error(`figlens MCP is unavailable for shortcuts: ${JSON.stringify(status)}. Check the host MCP configuration; local preview, frame and review remain available.`);
    if (!status.catalog.length) throw new Error(`figlens MCP is connected, but OpenCode has not catalogued its tools after 10 s: ${JSON.stringify(status)}. Retry the command; local preview, frame and review remain available.`);
    const pkg = JSON.parse(await readFile(new URL('../package.json', entry), 'utf8'));
    const version = (await current.run(['--version'])).stdout.trim();
    if (version !== `figlens ${pkg.version}`) throw new Error(`Launcher version differs: ${version}; package ${pkg.version}.`);
  }
  async function preview(current, file, node) {
    if (!file) throw new Error('Provide a local .fig file.');
    const { absolute } = await current.flow.source(file);
    await current.authorize('read', [cacheRoot]);
    await current.authorize('edit', [cacheRoot, path.join(tmpdir(), 'figlens-preview-*')]);
    await current.authorize('bash', [commandLine(['preview', absolute, ...(node ? [node] : [])])]);
    return current.viewer.start(absolute, node, { cwd: current.root, signal: current.signal });
  }
  async function select(current, file, node) {
    if (!file || !node) throw new Error('Provide a file and frame together.');
    const directory = await current.flow.referenceDirectory();
    const design = await current.flow.design({ file, frame: node }, current.signal);
    const output = path.join(directory, `${randomUUID()}.png`);
    const args = ['export', design.absolute, design.frame, output, '--scale', '0.5'];
    await current.authorize('edit', [current.flow.directory, current.flow.guide, directory]);
    await current.authorize('bash', [commandLine(args)]);
    try {
      await mkdir(directory, { recursive: true });
      await current.run(args);
      const bytes = await readFile(output);
      if (bytes.length > 5 * 1024 * 1024) throw new Error('Reference exceeds 5 MiB. Export at a smaller scale.');
      await current.flow.select(file, node, current.signal);
      return { design, referenceUrl: pathToFileURL(output).href, reference: { uri: `data:image/png;base64,${bytes.toString('base64')}`, mime: 'image/png', name: `${design.name}.png` } };
    } catch (error) { await rm(output, { force: true }); throw error; }
  }
  async function operation(input, signal) {
    const current = await scope(input.sessionID, signal);
    if (input.action === 'status') return { active: (await current.flow.read()).state.active, mcp: await mcpStatus() };
    if (input.action === 'close') { await current.viewer.close(); await current.flow.close(); return { closed: true }; }
    if (input.action === 'preview') return preview(current, input.file, input.node);
    if (input.action === 'select') return select(current, input.file, input.node);
    if (input.action === 'frames') {
      const file = input.file || (await current.flow.read()).state.active?.file;
      if (!file) throw new Error('No saved file. Open a known file/frame, or describe the screen in a normal message.');
      return { file, ...await current.flow.frames(file, current.signal, input.query || '') };
    }
    throw new Error('Unknown figlens operation.');
  }
  async function commandHandler({ sessionID, prompt, delivery }) {
    const match = prompt.text.trimStart().match(/^(\S+)(?:\s([\s\S]*))?$/);
    const kind = match?.[1] || 'help', rest = match?.[2] || '';
    if (rest.trim() && ['status', 'close', 'review', 'map'].includes(kind)) throw new Error(`/figlens ${kind} takes no arguments.`);
    const current = await scope(sessionID);
    await ctx.session.wait({ sessionID });
    if (['plan', 'yolo', 'map'].includes(kind) || (kind === 'open' && !/\s--frame\s/.test(rest))) {
      await requireMcp(current);
      await ctx.session.prompt({ sessionID, text: await buildRequest(kind, rest, entry), files: prompt.files, agents: prompt.agents, skills: prompt.skills, delivery });
      return;
    }
    let data;
    if (kind === 'help' || kind === '--help') data = HELP;
    else if (kind === 'review') data = 'Use /figlens review in the native TUI to review mappings. Noninteractive clients leave proposals pending; authenticated RPC clients are trusted.';
    else if (kind === 'status' || kind === 'close') data = await operation({ sessionID, action: kind });
    else if (kind === 'frame') {
      const active = (await current.flow.read()).state.active;
      if (!active || !rest.trim()) throw new Error('No saved file or frame ID. Use /figlens frame in the native TUI, or /figlens open "file.fig" --frame ID.');
      const selected = await select(current, active.file, rest.trim());
      await ctx.session.prompt({ sessionID, text: `figlens reference: ${selected.design.name}\n[Open reference PNG on the server machine](${selected.referenceUrl})`, files: [selected.reference], resume: false });
      data = selected.design;
    }
    else if (kind === 'open') {
      const known = rest.trim().match(/^([\s\S]+)\s+--frame\s+(\S+)$/);
      if (!known) throw new Error('Provide a file and --frame ID.');
      const file = known[1].trim().replace(/^(["'])([\s\S]*)\1$/, '$2');
      const selected = await select(current, file, known[2]);
      await ctx.session.prompt({ sessionID, text: `figlens reference: ${selected.design.name}\n[Open reference PNG on the server machine](${selected.referenceUrl})`, files: [selected.reference], resume: false });
      data = selected.design;
    } else if (kind === 'preview') {
      const file = rest.trim().replace(/^(["'])([\s\S]*)\1$/, '$2');
      if (file && path.extname(file).toLowerCase() === '.fig' && existsSync(path.resolve(current.root, file))) data = await preview(current, file);
      else {
        await requireMcp(current);
        await ctx.session.prompt({ sessionID, text: await buildRequest(kind, rest, entry), files: prompt.files, agents: prompt.agents, skills: prompt.skills, delivery });
        return;
      }
    } else throw new Error(`Unknown figlens command: ${kind}. ${HELP}`);
    await ctx.session.synthetic({ sessionID, text: typeof data === 'string' ? data : JSON.stringify(data, null, 2), resume: false });
  }
  // The JSON report a failed CLI command prints after its message, if any.
  const report = error => {
    for (const line of (error.cause?.stderr ?? '').trim().split('\n').reverse()) {
      try { return JSON.parse(line); } catch { /* the message line */ }
    }
    return null;
  };
  async function cli(current, tool, input) {
    // Only a lease this session's holder started can be stopped here; it releases it through its own child.
    if (tool === 'watch' && input.action === 'stop') {
      const leaseId = required(input, 'leaseId');
      const end = await current.holder.stop(leaseId);
      if (!end) throw new Error(`${leaseId} is not a lease this session holds, or it already ended; only its own live leases can be stopped here. figlens_watch status lists the workspace's leases.`);
      return end;
    }
    // Invalid or conflicting arguments fail before any permission prompt.
    const args = cliArgs(tool, input);
    const invoke = async (args, text = false) => {
      const build = args[0] === 'index' && ['prepare', 'refresh', 'rebuild', 'track'].includes(args[1]);
      try { const { stdout } = await current.run(args, build ? 0 : undefined); return text ? { flat: stdout } : JSON.parse(stdout); }
      catch (error) {
        // The CLI explains a failure on stderr; a partial reset or failed job also prints its report.
        const text = [error.stderr, error.stdout].map(value => value?.trim()).filter(Boolean).join('\n');
        if (current.signal.aborted || !text) throw error;
        throw new Error(text, { cause: error });
      }
    };
    // Status takes only the selectors, so conflicting ones fail here, before any permission or write. An
    // operand file is not a selector, except that a prepare without a project directory is found by it.
    const status = ({ action, file, workspace, workspaceId, storage }) => invoke(cliArgs('index', { action: 'status', workspace, workspaceId, storage,
      file: ['add', 'relocate'].includes(action) || (action === 'prepare' && workspace) ? undefined : file }));
    const missing = what => new Error(`This figlens binary does not report ${what}; update figlens to use these tools.`);
    // Where the binary places this workspace: its directory and the sibling lock any exclusive open may recreate,
    // or the state root of the storage it chose while none exists.
    const place = ({ paths, roots }) => {
      if (paths && !paths.lock) throw missing("its workspace's lock file");
      return paths ? [paths.workspace, paths.lock] : [roots.state.path];
    };
    // The binary canonicalizes a root before writing under it, so an edit also covers the target of any link on the
    // way; a path that does not exist yet resolves through its nearest existing parent. The walk starts from the
    // unnormalized path, so a `..` after a link resolves as the binary resolves it, not textually.
    const destination = async file => {
      try { return await realpath(file); }
      catch (error) {
        // Only a missing entry climbs; a file in the way (ENOTDIR), a loop or a denied directory fails closed.
        if (error.code !== 'ENOENT' || path.dirname(file) === file) throw error;
        // A dangling link is written through, as output::destination follows it.
        const link = await readlink(file).catch(() => null);
        if (link) return destination(path.isAbsolute(link) ? link : `${path.dirname(file)}${path.sep}${link}`);
        return path.join(await destination(path.dirname(file)), path.basename(file));
      }
    };
    // Each path as requested and as its canonical destination.
    const both = async files => [...new Set((await Promise.all(files.map(async file =>
      [path.resolve(current.flow.root, file), await destination(path.isAbsolute(file) ? file : `${current.flow.root}${path.sep}${file}`)]))).flat())];
    const edit = async files => current.authorize('edit', await both(files));
    // The CLI downloads a gs:// design into its own cache, which this path cannot authorize, so only MCP serves it.
    if (input.file?.startsWith('gs://')) throw new Error('The OpenCode CLI path reads local .fig files only. Enable the figlens MCP server for a gs:// design, or pass a local copy.');
    // A design the CLI reads honours read rules on its requested and canonical paths, as Workflow.source does; the
    // CLI gets the path unnormalized, so a `..` after a link reads where the link leads.
    if (input.file && (tool !== 'index' || ['prepare', 'add', 'relocate'].includes(input.action))) await current.authorize('read', await both([input.file]));
    // Workspace search, usages and changes, refresh, rebuild and track read the registered designs' content, which this session may deny.
    const readSources = async ({ state, sources }) => {
      // A workspace that does not exist yet reads no design; the command then fails as the MCP tool does.
      if (['absent', 'uninitialized'].includes(state)) return;
      if (!Array.isArray(sources)) throw missing("a workspace's sources");
      const paths = sources.filter(source => !input.sourceId || source.sourceId === input.sourceId).map(source => source.path);
      await current.authorize('read', [...new Set((await Promise.all(paths.map(async file => [file, await realpath(file).catch(() => file)]))).flat())]);
    };
    if (['search', 'usages', 'changes'].includes(tool) && !input.file) await readSources(await status(input));
    // Every other watch action writes checkpoints, subscriptions or leases into the workspace database, as a cancel writes its job.
    if (tool === 'watch' && !['list', 'status', 'start'].includes(input.action)) await edit(place(await status(input)));
    // A lease makes the shared runner read its source on every save, so start also asks read on it.
    if (tool === 'watch' && input.action === 'start') {
      const found = await status(input);
      await readSources(found);
      await edit(place(found));
      await current.authorize('bash', [commandLine(args, holding())]);
      return current.holder.start(args.slice(2), current.signal);
    }
    if (tool === 'jobs' && input.action === 'cancel') {
      const { workspaceId } = await invoke(cliArgs('jobs', { ...input, action: 'status' }));
      if (!workspaceId) throw missing("a job's workspace");
      await edit(place(await status({ workspaceId, storage: input.storage })));
    }
    if (tool === 'index' && input.action !== 'status') {
      const grant = indexGrant(input);
      if (grant === 'replace_unavailable') {
        // The old place is unreachable, so status fails; its recovery still names the config root holding the location record.
        const roots = await status(input).then(found => found.roots,
          error => report(error)?.recovery?.find(action => action.roots)?.roots ?? Promise.reject(error));
        if (!roots?.config) throw missing('its config root');
        await current.authorize('figlens_index', [grant]);
        // The binary writes under DIR/state, which may itself be a link.
        // The state child joins the raw storage, so a `..` after a link resolves as the binary resolves it.
        await edit([required(input, 'storage'), `${input.storage}${path.sep}state`, roots.config.path]);
      } else {
        // A lost lock or newer schema fails status; the binary's own prepare or reset recovery names the place.
        const found = await status(input).catch(error => {
          const fix = report(error)?.recovery?.find(action => action.action === input.action && action.path && action.roots?.config);
          if (!fix) throw error;
          return { paths: { workspace: fix.path, lock: fix.lock }, roots: fix.roots };
        });
        if (!found.roots?.config) throw missing('its config root');
        if (['refresh', 'rebuild', 'track'].includes(input.action)) await readSources(found);
        if (grant) await current.authorize('figlens_index', [grant]);
        // Every write also takes a location lock or record under the config root (a build's publish, relocation's alias).
        await edit([...place(found), found.roots.config.path]);
      }
    }
    // inspect --flat prints lines, as the MCP tool's flat field.
    return invoke(args, tool === 'inspect' && input.flat === true);
  }
  async function dispose() {
    lifetime.abort();
    await Promise.all([...owners.keys()].map(close));
  }
  return { command, prefix, scope, preview, operation, commandHandler, configureMcp, mcpStatus, cli, close, dispose, signal: lifetime.signal,
    get cliPath() { return cliPath; }, get mcpName() { return mcpName; } };
}
