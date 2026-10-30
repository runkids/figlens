// Real OMP loader, runner and approval wrapper; scripted dialog answers, no model.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { copyFile, mkdir, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { Workflow } from '../pi/workflow.mjs';

const [sdkInput, outputInput, binaryInput, packageInput] = process.argv.slice(2);
assert.ok(sdkInput && outputInput && binaryInput && packageInput, 'Usage: bun scripts/check-omp-integration.mjs <omp-root> <fresh-output> <binary> <installed-package> [--tui]');
const sdk = path.resolve(sdkInput), output = path.resolve(outputInput), binary = path.resolve(binaryInput);
await mkdir(output);
const project = path.join(output, 'project 空白');
await mkdir(path.join(project, 'src'), { recursive: true });
process.env.PI_CODING_AGENT_DIR = path.join(output, 'agent');
process.env.FIGLENS_BIN = binary;
await copyFile(new URL('../samples/plugin-research/fixtures/mapping-library.fig', import.meta.url), path.join(project, 'mapping.fig'));
await copyFile(path.join(project, 'mapping.fig'), path.join(project, 'mapping-old.fig'));
await writeFile(path.join(project, 'src/Button.mjs'), 'export const Button = () => "button";\n');
const execute = promisify(execFile);
const flow = new Workflow(project, async args => JSON.parse((await execute(binary, args, { cwd: project })).stdout));
await flow.select('mapping.fig', '0:1');
await writeFile(flow.guide, (await readFile(flow.guide, 'utf8')) + '\nHuman note: preserve this line.\n');
const load = file => import(pathToFileURL(path.join(sdk, 'src', file)));
const { loadExtensions } = await load('extensibility/extensions/loader.ts');
const { ExtensionRunner } = await load('extensibility/extensions/runner.ts');
const { ExtensionToolWrapper, wrapRegisteredTool } = await load('extensibility/extensions/wrapper.ts');
const { SessionManager } = await load('session/session-manager.ts');
const { Settings } = await load('config/settings.ts');
const loaded = await loadExtensions([path.resolve(packageInput, 'omp/index.ts')], project);
assert.deepEqual(loaded.errors, []);
const session = SessionManager.inMemory(project);
const runner = new ExtensionRunner(loaded.extensions, loaded.runtime, project, session, {});
const shown = [], prompts = [], answers = [], notices = [];
const ui = {
  select: async (title, options) => { prompts.push({ title, options }); return answers.shift(); },
  input: async title => { prompts.push({ title }); return answers.shift(); },
  confirm: async (title, message) => { prompts.push({ title, message }); return answers.shift(); },
  approve: async prompt => { prompts.push(prompt); return answers.shift(); },
  notify: message => notices.push(message), setStatus() {},
};
runner.initialize({ sendMessage: message => shown.push(message), sendUserMessage: () => { throw new Error('No model dispatch in this probe'); },
  appendEntry() {}, getActiveTools: () => [], getAllTools: () => [], setActiveTools: async () => {}, getCommands: () => [],
}, { getModel: () => undefined, isIdle: () => true, abort() {}, hasPendingMessages: () => false,
  shutdown() {}, getContextUsage: () => undefined, compact: async () => {}, getSystemPrompt: () => [] },
{ waitForIdle: async () => {} }, ui, 'tui');
const result = { host: JSON.parse(await readFile(path.join(sdk, 'package.json'))).version, package: path.resolve(packageInput), modelCalled: false, nativeTui: false, tests: [] };
const tools = Object.fromEntries(runner.getAllRegisteredTools().map(tool => [tool.definition.name, new ExtensionToolWrapper(wrapRegisteredTool(tool, runner), runner)]));
const settings = policy => Settings.isolated({ 'tools.approvalMode': 'always-ask', 'tools.approval': policy });
const invoke = (name, policy, signal, params = {}) => tools[name].execute('installed', params, signal, undefined, { settings: settings(policy) });
const proposal = { instance: '0:2', target: { path: 'src/Button.mjs', export: 'Button', import: 'import { Button } from \"./src/Button.mjs\";', example: 'Button()' }, rationale: 'Original synthetic component fixture' };
async function check(name, run) {
  try { result.tests.push({ name, status: 'passed', evidence: await run() }); }
  catch (error) { result.tests.push({ name, status: 'failed', error: String(error.stack || error) }); }
}
await runner.emit({ type: 'session_start' });
await check('real OMP loader registers native schema, command and tools', async () => {
  assert.ok(JSON.parse(await readFile(path.join(sdk, 'package.json'))).version);
  assert.equal(loaded.extensions[0].commands.has('figlens'), true);
  assert.ok((await loaded.extensions[0].commands.get('figlens').getArgumentCompletions('')).every(item => item.description), 'every subcommand completion has a description');
  assert.deepEqual(Object.keys(tools), ['figlens_preview', 'figlens_bindings', 'figlens_propose_bindings']);
  return { tools: Object.keys(tools), approvalTiers: Object.values(tools).map(tool => tool.approval) };
});
await check('packaged descriptor, bridge handshake and absolute design path', async () => {
  await load('discovery/omp-plugins.ts');
  const { loadCapability } = await load('capability/index.ts');
  const options = { cwd: project, agentDir: process.env.PI_CODING_AGENT_DIR, providers: ['omp-plugins'],
    extensionRoots: { explicit: [path.resolve(packageInput)], configured: [], configuredLevel: 'project', mode: 'explicit-only' } };
  const discovered = await loadCapability('mcps', options);
  const config = discovered.items.find(item => item.name === 'figlens');
  assert.ok(config);
  assert.equal(config.args[0], path.resolve(packageInput, 'omp/mcp.cjs'));
  assert.equal(config.cwd, path.resolve(packageInput));
  const { connectToServer, listTools, callTool, disconnectServer } = await load('mcp/client.ts');
  const connection = await connectToServer('figlens', config);
  try {
    assert.ok((await listTools(connection)).some(tool => tool.name === 'find'));
    const relative = await callTool(connection, 'find', { file: 'mapping.fig', query: 'Mapping review' });
    assert.equal(relative.isError, true);
    const read = await callTool(connection, 'find', { file: path.join(project, 'mapping.fig'), query: 'Mapping review' });
    assert.equal(read.isError, false);
    assert.equal(read.structuredContent.results[0].id, '0:1');
    const searched = await callTool(connection, 'search', { file: path.join(project, 'mapping.fig'), query: 'Mapping review' });
    assert.equal(searched.isError, false);
    assert.equal(searched.structuredContent.results[0].ref.nodeId, '0:1');
    assert.equal(searched.structuredContent.coverage.complete, true);
    const inspected = await callTool(connection, 'inspect', { file: path.join(project, 'mapping.fig'), node: searched.structuredContent.results[0].ref.nodeId });
    assert.equal(inspected.structuredContent.node.id, '0:1');
    // Wiring only: two copies of one file are two sources with no changes.
    const diffed = await callTool(connection, 'diff', { old: path.join(project, 'mapping-old.fig'), file: path.join(project, 'mapping.fig') });
    assert.equal(diffed.isError, false);
    assert.equal(diffed.structuredContent.coverage.complete, true);
    assert.equal(diffed.structuredContent.coverage.changes, 0);
    assert.notEqual(diffed.structuredContent.source.old.sourceId, diffed.structuredContent.source.new.sourceId);
    const prepared = await callTool(connection, 'index', { action: 'prepare', file: path.join(project, 'mapping.fig'), scope: '0:0' });
    assert.equal(prepared.isError, false);
    let job;
    for (const end = Date.now() + 30000; Date.now() < end;) {
      job = await callTool(connection, 'jobs', { action: 'status', jobId: prepared.structuredContent.job.jobId });
      if (job.isError || !['queued', 'running'].includes(job.structuredContent.state)) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    assert.equal(job.structuredContent?.state, 'succeeded', JSON.stringify(job));
    // The library reference comes from an instance's own outgoing usages, not from the fixture.
    const workspaceId = prepared.structuredContent.workspaceId;
    const out = await callTool(connection, 'usages', { workspaceId, node: '0:2', direction: 'out' });
    const named = out.structuredContent?.results?.find(item => item.library)?.library;
    assert.deepEqual(named, { kind: 'component', namespace: 'synthetic-library', key: 'synthetic-button', basis: 'cached_copy' }, JSON.stringify(out));
    const library = { kind: named.kind, namespace: named.namespace, key: named.key };
    const used = await callTool(connection, 'usages', { workspaceId, library });
    assert.equal(used.isError, false);
    assert.deepEqual(used.structuredContent.library, library);
    assert.deepEqual(used.structuredContent.results.map(item => item.ref.nodeId), ['0:2', '0:3']);
    assert.deepEqual(used.structuredContent.definition, { state: 'unresolved', reason: 'library_source_unproven' });
    assert.equal(used.structuredContent.coverage.complete, true);
    assert.deepEqual(used.structuredContent.coverage.sources.map(source => source.state), ['ready']);
    return { descriptorCwd: config.cwd, bridge: config.args[0], absoluteFrame: '0:1', relativePathRejected: true,
      searchRef: searched.structuredContent.results[0].ref, diffedTwoCopies: diffed.structuredContent.source,
      libraryUsages: used.structuredContent.definition };
  } finally { await disconnectServer(connection); }
});
await check('first use through the packaged bridge: prepare, poll, search the published index, inspect, with native write approval', async () => {
  await load('discovery/omp-plugins.ts');
  const { loadCapability } = await load('capability/index.ts');
  const discovered = await loadCapability('mcps', { cwd: project, agentDir: process.env.PI_CODING_AGENT_DIR, providers: ['omp-plugins'],
    extensionRoots: { explicit: [path.resolve(packageInput)], configured: [], configuredLevel: 'project', mode: 'explicit-only' } });
  const { connectToServer, listTools, callTool, disconnectServer } = await load('mcp/client.ts');
  const { MCPTool } = await load('mcp/tool-bridge.ts');
  const { resolveApproval } = await load('tools/approval.ts');
  const state = path.join(output, 'state');
  const saved = { XDG_STATE_HOME: process.env.XDG_STATE_HOME, XDG_CONFIG_HOME: process.env.XDG_CONFIG_HOME };
  process.env.XDG_STATE_HOME = state;
  process.env.XDG_CONFIG_HOME = path.join(output, 'config');
  const connection = await connectToServer('figlens', discovered.items.find(item => item.name === 'figlens'));
  let connected = true;
  try {
    const listed = await listTools(connection);
    const file = path.join(project, 'mapping.fig');
    const content = async (name, args) => {
      const reply = await callTool(connection, name, args);
      assert.equal(reply.isError, false, JSON.stringify(reply).slice(0, 2000));
      return reply.structuredContent;
    };
    await content('search', { file, query: 'Mapping review' });
    await assert.rejects(readdir(path.join(state, 'figlens')), { code: 'ENOENT' }, 'connecting, listing and stateless search created figlens state');
    const prepared = await content('index', { action: 'prepare', file, scope: '0:0' });
    let job = prepared.job, polls = 0;
    for (const deadline = Date.now() + 30000; !['succeeded', 'failed', 'cancelled', 'interrupted'].includes(job.state) && Date.now() < deadline; polls++) {
      await new Promise(resolve => setTimeout(resolve, 50));
      job = await content('jobs', { action: 'status', jobId: job.jobId });
    }
    assert.equal(job.state, 'succeeded', JSON.stringify(job));
    const found = await content('search', { workspaceId: prepared.workspaceId, query: 'Mapping review' });
    assert.deepEqual(found.sources.map(source => source.state), ['ready']);
    const inspected = await content('inspect', { file, node: found.results[0].ref.nodeId });
    assert.equal(inspected.node.id, '0:1');
    const used = await content('usages', { workspaceId: prepared.workspaceId, node: '0:10' });
    assert.deepEqual(used.results.map(item => `${item.relation} ${item.ref.nodeId}`), ['instance_of 0:2', 'instance_of 0:3']);
    // Following changes: track builds a baseline, and a consumer that acks it reads nothing new.
    const settle = async job => {
      for (const deadline = Date.now() + 30000; !['succeeded', 'failed', 'cancelled', 'interrupted'].includes(job.state) && Date.now() < deadline;) {
        await new Promise(resolve => setTimeout(resolve, 50));
        job = await content('jobs', { action: 'status', jobId: job.jobId });
      }
      assert.equal(job.state, 'succeeded', JSON.stringify(job));
    };
    for (const tracked of (await content('index', { action: 'track', workspaceId: prepared.workspaceId, scope: '0:0' })).jobs) await settle(tracked);
    const polled = await content('changes', { workspaceId: prepared.workspaceId, consumer: 'omp' });
    assert.deepEqual(polled.revisions.map(revision => revision.kind), ['baseline'], JSON.stringify(polled).slice(0, 2000));
    assert.equal((await content('watch', { action: 'ack', workspaceId: prepared.workspaceId, consumer: 'omp', cursor: polled.nextCursor })).advanced, true);
    assert.deepEqual((await content('changes', { workspaceId: prepared.workspaceId, consumer: 'omp' })).revisions, []);
    // OMP approves every MCP tool at the write tier and shows the call's arguments; it has no per-action rule.
    const index = MCPTool.fromTools(connection, listed).find(tool => tool.mcpToolName === 'index');
    const reset = { action: 'reset', file };
    const decide = (mode, policy) => resolveApproval(index, reset, mode, policy).policy;
    const decisions = { alwaysAsk: decide('always-ask'), userDeny: decide('yolo', { [index.name]: 'deny' }), yolo: decide('yolo') };
    assert.equal(index.approval, 'write');
    const usages = MCPTool.fromTools(connection, listed).find(tool => tool.mcpToolName === 'usages');
    assert.equal(usages.approval, 'write');
    for (const name of ['changes', 'watch']) assert.equal(MCPTool.fromTools(connection, listed).find(tool => tool.mcpToolName === name).approval, 'write', name);
    assert.equal(decisions.alwaysAsk, 'prompt');
    assert.equal(decisions.userDeny, 'deny');
    // The README states this: OMP's default yolo mode runs reset without asking.
    assert.equal(decisions.yolo, 'allow');
    assert.match(index.formatApprovalDetails(reset).join('\n'), /reset/);
    // A lease the bridge's server holds ends released when OMP disconnects it, read by the CLI afterwards.
    const lease = await content('watch', { action: 'start', workspaceId: prepared.workspaceId, scope: '0:0' });
    assert.equal(lease.state, 'active', JSON.stringify(lease));
    await disconnectServer(connection);
    connected = false;
    const ended = async () => JSON.parse((await execute(binary, ['watch', 'status', '--id', prepared.workspaceId], { cwd: project })).stdout)
      .leases.filter(item => item.leaseId === lease.leaseId).map(item => [item.state, item.reason]);
    let released = await ended();
    for (const deadline = Date.now() + 10000; released[0]?.[0] !== 'ended' && Date.now() < deadline; released = await ended())
      await new Promise(resolve => setTimeout(resolve, 100));
    assert.deepEqual(released, [['ended', 'released']]);
    return { workspaceId: prepared.workspaceId, dispatched: prepared.job.state, job: job.state, polls, ref: found.results[0].ref,
      inspected: inspected.node.id, usedBy: used.results.map(item => item.ref.nodeId), approvalTier: index.approval, usagesTier: usages.approval, trackedBaselineAcked: true, watchLeaseReleasedOnDisconnect: true, resetDecisions: decisions };
  } finally {
    if (connected) await disconnectServer(connection);
    for (const [name, value] of Object.entries(saved)) value === undefined ? delete process.env[name] : process.env[name] = value;
  }
});
await check('native disabled and remote overrides retain priority over the package descriptor', async () => {
  await load('discovery/builtin.ts');
  const { loadCapability, invalidate } = await load('capability/index.ts');
  await mkdir(process.env.PI_CODING_AGENT_DIR, { recursive: true });
  const file = path.join(process.env.PI_CODING_AGENT_DIR, 'mcp.json');
  for (const override of [{ command: 'never-launch-disabled', enabled: false }, { url: 'https://example.invalid/mcp' }]) {
    await writeFile(file, JSON.stringify({ mcpServers: { figlens: override } }));
    invalidate(file);
    const discovered = await loadCapability('mcps', { cwd: project, agentDir: process.env.PI_CODING_AGENT_DIR,
      providers: ['native', 'omp-plugins'], extensionRoots: { explicit: [path.resolve(packageInput)], configured: [], configuredLevel: 'project', mode: 'explicit-only' } });
    const config = discovered.items.find(item => item.name === 'figlens');
    assert.ok(config);
    for (const [key, value] of Object.entries(override)) assert.equal(config[key], value);
    assert.equal(await readFile(file, 'utf8'), JSON.stringify({ mcpServers: { figlens: override } }));
  }
  return { disabledPreserved: true, remotePreserved: true, overrideNeverConnected: true };
});
await check('real approval wrapper denies before the shared writer', async () => {
  const before = await readFile(flow.guide);
  await assert.rejects(invoke('figlens_propose_bindings', { figlens_propose_bindings: 'deny' }, undefined, { proposals: [proposal] }), /denied|deny|blocked/i);
  assert.deepEqual(await readFile(flow.guide), before);
  assert.equal(prompts.length, 0);
  return { guideUnchanged: true };
});
await check('real approval wrapper ask cancellation leaves no proposal', async () => {
  const before = await readFile(flow.guide);
  answers.push('deny');
  await assert.rejects(invoke('figlens_propose_bindings', {}, undefined, { proposals: [proposal] }), /denied|declined|reject/i);
  assert.deepEqual(await readFile(flow.guide), before);
  assert.equal(prompts.length, 1);
  return { guideUnchanged: true, approvalDialogRequested: true };
});
await check('allow and ask-once reach the real native CLI and pending writer', async () => {
  const bindings = await invoke('figlens_bindings', { figlens_bindings: 'allow' });
  assert.equal(bindings.details.total, 2);
  answers.push('once');
  await invoke('figlens_propose_bindings', {}, undefined, { proposals: [proposal] });
  assert.equal((await flow.pending()).length, 1);
  return { instanceCount: bindings.details.total, pending: 1 };
});
await check('native command context edits four fields and confirms shared state', async () => {
  answers.push('Edit', '', '', '', 'Button({ label: "reviewed" })', true);
  await loaded.extensions[0].commands.get('figlens').handler('review', runner.createCommandContext());
  const { state } = await flow.read();
  assert.equal(state.mappings[0].status, 'approved');
  assert.equal(state.mappings[0].target.example, 'Button({ label: "reviewed" })');
  assert.ok(shown.some(message => message.content.includes('Agent-reported checks')));
  assert.ok((await readFile(flow.guide, 'utf8')).endsWith('\nHuman note: preserve this line.\n'));
  return { approved: true, fourFieldEdit: true, humanNotePreserved: true, dialogTransport: 'scripted real runner UI context' };
});
await check('cancellation reaches the shared writer without changing guide', async () => {
  const before = await readFile(flow.guide);
  const controller = new AbortController(); controller.abort();
  await assert.rejects(invoke('figlens_propose_bindings', { figlens_propose_bindings: 'allow' }, controller.signal, { proposals: [proposal] }));
  assert.deepEqual(await readFile(flow.guide), before);
  return { guideUnchanged: true };
});
await check('manual reference export rejects a symlinked preview directory', async () => {
  const external = path.join(output, 'external-previews');
  await mkdir(external);
  const directory = path.join(flow.directory, 'previews');
  await symlink(external, directory, 'junction');
  try {
    const before = shown.length;
    const notice = notices.length;
    await loaded.extensions[0].commands.get('figlens').handler('open "mapping.fig" --frame 0:1', runner.createCommandContext());
    assert.deepEqual(await readdir(external), [], 'External destination must remain empty');
    assert.ok(shown.slice(before).every(message => message.customType !== 'figlens-preview'));
    assert.ok(notices.slice(notice).some(message => message.includes('symbolic link')));
  } finally { await rm(directory); }
  return { externalDirectoryUnchanged: true, imageNotAttached: true };
});
await check('manual selection attaches PNG bytes and explicit tool scope leaves selection unchanged', async () => {
  const command = loaded.extensions[0].commands.get('figlens').handler;
  await command('open "mapping.fig" --frame 0:1', runner.createCommandContext());
  const image = shown.findLast(message => message.customType === 'figlens-preview')?.content.find(part => part.type === 'image');
  assert.ok(image);
  assert.equal(Buffer.from(image.data, 'base64').subarray(1, 4).toString(), 'PNG');
  await copyFile(path.join(project, 'mapping.fig'), path.join(project, 'override.fig'));
  const active = (await flow.read()).state.active;
  const read = await invoke('figlens_bindings', { figlens_bindings: 'allow' }, undefined, { file: 'override.fig', frame: '0:1' });
  assert.equal(read.details.file, 'override.fig');
  assert.deepEqual((await flow.read()).state.active, active);
  await command('status', runner.createCommandContext());
  assert.ok(shown.at(-1).content.includes('Mapping review'));
  return { imageBytes: Buffer.from(image.data, 'base64').length, pairedOverridePreservesSelection: true };
});
await check('unverified MCP shortcuts do not dispatch and preserve local state', async () => {
  const before = await readFile(flow.guide);
  await loaded.extensions[0].commands.get('figlens').handler('plan Only the card', runner.createCommandContext());
  assert.ok(shown.at(-1).content.includes('MCP shortcut unavailable'));
  assert.deepEqual(await readFile(flow.guide), before);
  return { modelCalled: false, existingGuideUnchanged: true };
});
await check('real viewer is owned and removed on session switch', async () => {
  const preview = await invoke('figlens_preview', { figlens_preview: 'allow' }, undefined, { file: path.join(project, 'mapping.fig') });
  const url = preview.details.url;
  const catalog = await fetch(`${url}catalog`);
  assert.equal(catalog.status, 200);
  await catalog.arrayBuffer();
  await runner.emit({ type: 'session_switch', reason: 'new', previousSessionFile: undefined });
  await assert.rejects(fetch(url));
  return { catalogReady: true, sessionSwitchClosesOwnedViewer: true };
});
await runner.emit({ type: 'session_shutdown' });
if (process.argv.includes('--tui')) await check('native OMP TUI four-field edit and final confirmation', async () => {
  const native = path.join(output, 'native');
  const project = path.join(native, 'project 空白');
  await mkdir(path.join(project, 'src'), { recursive: true });
  await mkdir(path.join(native, 'agent-tui'));
  await writeFile(path.join(native, 'agent-tui/config.yml'), 'startup:\n  setupWizard: false\n  checkUpdate: false\n  showSplash: false\n');
  await copyFile(new URL('../samples/plugin-research/fixtures/mapping-library.fig', import.meta.url), path.join(project, 'mapping.fig'));
  await writeFile(path.join(project, 'src/Button.mjs'), 'export const Button = () => "button";\n');
  const flow = new Workflow(project, async args => JSON.parse((await execute(binary, args, { cwd: project })).stdout));
  await flow.select('mapping.fig', '0:1');
  await flow.propose({ proposals: [{ instance: '0:2',
    target: { path: 'src/Button.mjs', export: 'Button', import: 'import { Button } from "./src/Button.mjs";', example: 'Button()' },
    rationale: 'Original synthetic fixture' }] });
  const terminal = await execute('/usr/bin/python3', [fileURLToPath(new URL('./check-omp-tui.py', import.meta.url)), process.execPath,
    path.join(sdk, 'dist/cli.js'), path.resolve(packageInput, 'omp/index.ts'), binary, native], { timeout: 120000 });
  const state = (await flow.read()).state;
  assert.equal(state.mappings[0].status, 'approved');
  assert.equal(state.mappings[0].target.example, 'Button({ label: "native OMP" })');
  result.nativeTui = true;
  return JSON.parse(terminal.stdout);
});
await writeFile(path.join(output, 'results.json'), JSON.stringify(result, null, 2) + '\n');
console.log(JSON.stringify(result, null, 2));
process.exitCode = result.tests.some(test => test.status === 'failed') ? 1 : 0;
