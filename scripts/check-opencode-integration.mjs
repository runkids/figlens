// Real OpenCode server and native UI against the packaged entries; never starts a model.
import assert from 'node:assert/strict';
import { execFile, execFileSync, spawn } from 'node:child_process';
import { existsSync, renameSync, writeFileSync } from 'node:fs';
import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { randomBytes } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { promisify } from 'node:util';
import { operationRpc, reviewRpc } from '../opencode/rpc.mjs';
import { Workflow } from '../pi/workflow.mjs';

const [hostInput, sdkInput, packageInput, binaryInput, outputInput] = process.argv.slice(2);
assert.ok(outputInput, 'Usage: node scripts/check-opencode-integration.mjs <opencode> <client-root> <installed-package> <binary> <fresh-output> [--tui]');
const [host, sdk, packaged, binary, output] = [hostInput, sdkInput, packageInput, binaryInput, outputInput].map(value => path.resolve(value));
await mkdir(output);
const project = path.join(output, 'project'), config = path.join(output, 'config');
await mkdir(path.join(project, 'src'), { recursive: true });
await mkdir(config);
await copyFile(new URL('../samples/plugin-research/fixtures/mapping.fig', import.meta.url), path.join(project, 'mapping.fig'));
await copyFile(path.join(project, 'mapping.fig'), path.join(project, 'mapping-old.fig'));
await writeFile(path.join(project, 'src/Button.mjs'), 'export const Button = () => "button";\n');
// Scripted loopback provider: each turn returns a fixed Code Mode `execute` call (find the figlens
// paths, search, inspect the returned ref, then diff two copies of the file), so the real session loop runs the managed MCP
// tools. Deterministic routing evidence, not a model.
const scripted = { results: [], tools: null };
const fixture = path.join(project, 'mapping.fig');
// A step without its prerequisite ends the turn, so a missing tool fails the check instead of retrying.
const tool = (paths, name) => Array.isArray(paths) && paths.find(item => item.endsWith(`.${name}`) || item.endsWith(`["${name}"]`));
const catalog = names => `return ${JSON.stringify(names)}.flatMap(query => search({ query, namespace: 'figlensmcp' }).items.map(item => item.path));`;
const searchSteps = [
  () => catalog(['search', 'inspect', 'diff']),
  paths => tool(paths, 'search') && `return await ${tool(paths, 'search')}(${JSON.stringify({ file: fixture, query: 'Mapping review' })});`,
  (paths, found) => found?.results?.[0] && `return await ${tool(paths, 'inspect')}(${JSON.stringify({ file: fixture, node: found.results[0].ref.nodeId })});`,
  paths => tool(paths, 'diff') && `return await ${tool(paths, 'diff')}(${JSON.stringify({ old: path.join(project, 'mapping-old.fig'), file: fixture })});`,
];
// First use as spec 3.1 describes it: prepare the scope, poll its job, search the published index, inspect the ref.
// A step either returns Code Mode source or names a plugin tool to call directly.
const terminal = ['succeeded', 'failed', 'cancelled', 'interrupted'];
const firstUseCode = paths => ['index', 'jobs', 'search', 'inspect', 'usages'].every(name => tool(paths, name)) && `
  const before = await ${tool(paths, 'index')}(${JSON.stringify({ action: 'status', file: fixture })});
  const prepared = await ${tool(paths, 'index')}(${JSON.stringify({ action: 'prepare', file: fixture, scope: '0:0' })});
  let job = prepared.job, polls = 0;
  while (!${JSON.stringify(terminal)}.includes(job.state) && polls++ < 5000) job = await ${tool(paths, 'jobs')}({ action: 'status', jobId: job.jobId });
  const found = await ${tool(paths, 'search')}({ workspaceId: prepared.workspaceId, query: 'Mapping review' });
  const node = found.results?.[0] && await ${tool(paths, 'inspect')}({ file: ${JSON.stringify(fixture)}, node: found.results[0].ref.nodeId });
  const used = await ${tool(paths, 'usages')}({ workspaceId: prepared.workspaceId, node: '0:10' });
  return { usedBy: used.results?.map(item => item.relation + ' ' + item.ref.nodeId), usedSource: used.results?.[0]?.ref.sourceId === found.results?.[0]?.ref.sourceId, before: before.state, workspaceId: prepared.workspaceId, dispatched: prepared.job.state, job: job.state, polls,
    ref: found.results?.[0]?.ref, sources: found.sources, complete: found.coverage?.complete, inspected: node?.node?.id };`;
const firstUseSteps = [() => catalog(['index', 'jobs', 'search', 'inspect', 'usages']), firstUseCode];
const resetSteps = [() => catalog(['index']), paths => tool(paths, 'index') && `
  try { return { reset: await ${tool(paths, 'index')}(${JSON.stringify({ action: 'reset', file: fixture })}) }; }
  catch (error) { return { refused: String(error?.message ?? error) }; }`];
const direct = (name, input) => ({ name, input });
const cliSteps = [
  () => direct('figlens_index', { action: 'status', file: fixture }),
  () => direct('figlens_index', { action: 'prepare', file: fixture, scope: '0:1' }),
  (status, prepared) => prepared?.workspaceId && direct('figlens_jobs', { action: 'status', jobId: prepared.job.jobId }),
  (status, prepared) => direct('figlens_search', { workspaceId: prepared.workspaceId, node: '0:1', query: 'Mapping review' }),
  (status, prepared, job, found) => found?.results?.[0] && direct('figlens_inspect', { file: fixture, node: found.results[0].ref.nodeId }),
  // An exact instance sublayer id longer than 200 bytes passes the host's schema and reaches the binary.
  () => direct('figlens_inspect', { file: fixture, node: deepId }),
  (status, prepared) => direct('figlens_usages', { workspaceId: prepared.workspaceId, node: '0:10', scope: '0:1' }),
  // One result per page: the second page carries the first page's cursor and snapshot.
  (status, prepared) => direct('figlens_usages', { workspaceId: prepared.workspaceId, node: '0:10', scope: '0:1', limit: 1 }),
  (status, prepared, ...rest) => rest[5]?.nextCursor && direct('figlens_usages', { workspaceId: prepared.workspaceId, node: '0:10', scope: '0:1', limit: 1,
    cursor: rest[5].nextCursor, snapshotId: rest[5].source.snapshotId }),
  // The binary's own refusal reaches the agent unchanged.
  (status, prepared) => direct('figlens_usages', { workspaceId: prepared.workspaceId, node: '0:10', scope: '0:1', relation: 'nope' }),
  // Tracking records a baseline; a consumer that acks it reads nothing new after it.
  (status, prepared) => direct('figlens_index', { action: 'track', workspaceId: prepared.workspaceId, scope: '0:1' }),
  (status, prepared) => direct('figlens_changes', { workspaceId: prepared.workspaceId, consumer: 'smoke' }),
  (status, prepared, ...rest) => rest[9]?.nextCursor && direct('figlens_watch', { action: 'ack', workspaceId: prepared.workspaceId, consumer: 'smoke',
    cursor: rest[9].nextCursor }),
  (status, prepared) => direct('figlens_changes', { workspaceId: prepared.workspaceId, consumer: 'smoke' }),
];
const deepId = `I${Array(60).fill('9:9').join(';')}`;
let steps = searchSteps;
// One scripted conversation at a time: each session starts the provider's state again.
function lane(next) { steps = next; for (const key of Object.keys(scripted)) delete scripted[key]; scripted.results = []; }
const value = text => { try { return JSON.parse(text); } catch { return text; } };
const provider = createServer((request, response) => reply(request, response).catch(error => {
  scripted.error = error.stack;
  response.writeHead(500).end();
}));
async function reply(request, response) {
  let body = '';
  for await (const chunk of request) body += chunk;
  const { messages, tools } = JSON.parse(body);
  scripted.tools ??= (tools ?? []).map(tool => tool.function?.name);
  scripted.raw = messages.filter(message => message.role === 'tool').map(message =>
    typeof message.content === 'string' ? message.content : message.content.map(part => part.text ?? '').join(''));
  scripted.results = scripted.raw.map(value);
  const code = steps[scripted.results.length]?.(...scripted.results);
  const call = typeof code === 'string' ? { name: 'execute', arguments: JSON.stringify({ code }) }
    : code && { name: code.name, arguments: JSON.stringify(code.input) };
  const delta = call
    ? { tool_calls: [{ index: 0, id: `call_${scripted.results.length}`, type: 'function', function: call }] }
    : { content: 'done' };
  const chunk = choice => `data: ${JSON.stringify({ id: 'scripted', object: 'chat.completion.chunk', created: 0, model: 'scripted', choices: [choice] })}\n\n`;
  response.writeHead(200, { 'content-type': 'text/event-stream' });
  response.end(chunk({ index: 0, delta: { role: 'assistant', ...delta } })
    + chunk({ index: 0, delta: {}, finish_reason: call ? 'tool_calls' : 'stop' }) + 'data: [DONE]\n\n');
}
await new Promise(resolve => provider.listen(0, '127.0.0.1', resolve));
const probe = path.join(config, 'scripted-provider');
await mkdir(probe);
await writeFile(path.join(probe, 'package.json'), JSON.stringify({ name: 'figlens-scripted-provider', type: 'module', exports: { '.': './index.mjs' } }));
await writeFile(path.join(probe, 'index.mjs'), `import { Plugin } from '@opencode/plugin';
export default Plugin.define({ id: 'figlens-scripted-provider', async setup(ctx) {
  await ctx.provider.transform(editor => editor.add({
    info: { id: 'figlens-scripted', name: 'Scripted loopback', activation: 'auto', package: '@opencode/ai/providers/openai-compatible',
      settings: { baseURL: 'http://127.0.0.1:${provider.address().port}/v1', apiKey: 'dummy-local' } },
    models: [{ id: 'scripted', modelID: 'scripted', providerID: 'figlens-scripted', type: 'chat', name: 'Scripted', settings: {},
      capabilities: { tools: true, input: ['text'], output: ['text'], temperature: false }, variants: [], time: { released: 0 },
      cost: [], status: 'active', enabled: true, limit: { context: 131072, output: 8192 } }],
  }));
} });
`);
await writeFile(path.join(config, 'opencode.json'), JSON.stringify({ plugins: [pathToFileURL(packaged).href, pathToFileURL(probe).href] }));
const execute = promisify(execFile);
const flow = new Workflow(project, async args => JSON.parse((await execute(binary, args, { cwd: project })).stdout));
const proposal = instance => ({ instance, target: { path: 'src/Button.mjs', export: 'Button', import: 'import { Button } from "./src/Button.mjs";', example: 'Button()' }, rationale: 'Original synthetic fixture' });
const password = randomBytes(32).toString('hex');
const env = { PATH: '/usr/bin:/bin', HOME: path.join(output, 'home'),
  OPENCODE_SERVER_PASSWORD: password, OPENCODE_CONFIG_DIR: config, OPENCODE_TEST_HOME: path.join(output, 'home'),
  XDG_CONFIG_HOME: config, XDG_DATA_HOME: path.join(output, 'data'), XDG_STATE_HOME: path.join(output, 'state'), XDG_CACHE_HOME: path.join(output, 'cache'),
  OPENCODE_DISABLE_MODELS_FETCH: '1', OPENCODE_DISABLE_CLAUDE_CODE: '1', OPENCODE_DISABLE_PROJECT_CONFIG: '1', FIGLENS_BIN: binary };
const server = spawn(host, ['serve', '--hostname', '127.0.0.1', '--port', '0'], { cwd: project, env, stdio: ['ignore', 'pipe', 'pipe'] });
const exited = new Promise(resolve => server.once('exit', (code, signal) => resolve({ code, signal })));
let logs = '';
server.stderr.on('data', data => { logs += data; });
const result = { package: packaged, modelCalled: false, tests: [] };
async function check(name, run) {
  try { result.tests.push({ name, status: 'passed', evidence: await run() }); }
  catch (error) { result.tests.push({ name, status: 'failed', error: error.stack || JSON.stringify(error) }); }
}
try {
  const url = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Server startup timed out')), 15000);
    server.once('error', error => { clearTimeout(timer); reject(error); });
    server.once('exit', () => { clearTimeout(timer); reject(new Error('Server exited before readiness')); });
    server.stdout.on('data', data => {
      logs += data;
      const match = logs.match(/server listening on (http:\/\/127\.0\.0\.1:\d+)/);
      if (match) { clearTimeout(timer); resolve(match[1]); }
    });
  });
  const { OpenCode } = await import(pathToFileURL(path.join(sdk, 'dist/promise/index.js')));
  const client = OpenCode.make({ baseUrl: url, headers: { Authorization: `Basic ${Buffer.from(`opencode:${password}`).toString('base64')}` },
    fetch: (url, options) => fetch(url, { ...options, signal: options?.signal ?? AbortSignal.timeout(20000) }) });
  const location = { directory: project }, options = { location };
  result.host = (await client.server.info()).version;
  const rpc = client.rpc(operationRpc), mapping = client.rpc(reviewRpc);
  const session = await client.session.create({ location, title: 'Installed integration', permissions: [{ action: '*', resource: '*', effect: 'allow' }] });
  let activeUrl;
  await check('production loader, skill and managed MCP ready', async () => {
    // Ready means the managed tools are in OpenCode's tool registry, which `connected` precedes (#125).
    let status, connected;
    for (let attempt = 0; attempt < 100; attempt++) {
      status = await rpc.run({ sessionID: session.id, action: 'status' }, options);
      if (status.mcp.status.status === 'connected') connected ??= Date.now();
      if (status.mcp.catalog.length) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    assert.equal(status.mcp.managed, true);
    assert.equal(status.mcp.server, 'figlensmcp');
    assert.equal(status.mcp.status.status, 'connected');
    for (const tool of ['figlens_preview', 'figlens_bindings', 'figlens_propose_bindings']) assert.ok(status.mcp.tools.includes(tool), tool);
    for (const tool of ['figlensmcp_search', 'figlensmcp_inspect', 'figlensmcp_diff']) assert.ok(status.mcp.catalog.includes(tool), tool);
    // The managed server serves index, jobs, search, usages, changes, watch and inspect; the plugin adds no second copy.
    for (const tool of ['figlens_index', 'figlens_jobs', 'figlens_search', 'figlens_inspect', 'figlens_usages', 'figlens_changes', 'figlens_watch']) assert.ok(!status.mcp.tools.includes(tool), tool);
    const skills = await client.skill.list({ location });
    assert.ok(JSON.stringify(skills).includes('figlens'));
    return { mcp: status.mcp, catalogAfterConnectedMs: Date.now() - connected, skills: skills.data.map(skill => skill.name) };
  });
  await check('session tool calls run managed MCP search, inspect its exact ref and diff two copies', async () => {
    const searching = await client.session.create({ location, title: 'Scripted search', permissions: [{ action: '*', resource: '*', effect: 'allow' }] });
    await client.session.switchModel({ sessionID: searching.id, model: { id: 'scripted', providerID: 'figlens-scripted' } });
    await client.session.prompt({ sessionID: searching.id, text: 'Find the mapping review frame.' });
    await client.session.wait({ sessionID: searching.id }, { signal: AbortSignal.timeout(60000) })
      .catch(error => { throw new Error(`${error.message}; provider: ${scripted.error}; ${JSON.stringify(scripted.raw).slice(0, 2000)}`); });
    assert.ok(!scripted.error, scripted.error + JSON.stringify(scripted.raw).slice(0, 2000));
    const [paths, search, inspect, diff] = scripted.results;
    assert.ok(Array.isArray(paths) && ['search', 'inspect', 'diff'].every(name => paths.some(item => item.includes(name))), JSON.stringify(scripted.raw).slice(0, 2000));
    assert.equal(search.results[0].ref.nodeId, '0:1', JSON.stringify(search).slice(0, 2000));
    assert.equal(search.coverage.complete, true);
    assert.equal(inspect.node.id, search.results[0].ref.nodeId, JSON.stringify(inspect).slice(0, 2000));
    // Wiring only: two copies of one file are two sources with no changes.
    assert.ok(diff?.coverage?.complete === true && diff.coverage.changes === 0, JSON.stringify(diff).slice(0, 2000));
    assert.notEqual(diff.source.old.sourceId, diff.source.new.sourceId);
    const context = await client.session.context({ sessionID: searching.id });
    assert.ok(JSON.stringify(context).includes('done'));
    return { paths: paths.filter(item => /search|inspect|diff/.test(item)), searchRef: search.results[0].ref, inspected: inspect.node.id, diffedTwoCopies: diff.source,
      toolResults: scripted.results.length, provider: 'scripted loopback, not a model' };
  });
  const allowAll = [{ action: '*', resource: '*', effect: 'allow' }];
  async function converse(title, next, permissions = allowAll) {
    lane(next);
    const conversation = await client.session.create({ location, title, permissions });
    converse.session = conversation.id;
    await client.session.switchModel({ sessionID: conversation.id, model: { id: 'scripted', providerID: 'figlens-scripted' } });
    await client.session.prompt({ sessionID: conversation.id, text: title });
    await client.session.wait({ sessionID: conversation.id }, { signal: AbortSignal.timeout(60000) })
      .catch(error => { throw new Error(`${error.message}; provider: ${scripted.error}; ${JSON.stringify(scripted.raw).slice(0, 2000)}`); });
    assert.ok(!scripted.error, scripted.error);
    return scripted.results;
  }
  const state = path.join(env.XDG_STATE_HOME, 'figlens');
  await check('connecting, listing tools and stateless search create no workspace storage', async () => {
    assert.equal(existsSync(state), false, `${state} exists before any index call`);
    return { stateRoot: 'absent' };
  });
  await check('session first use prepares, polls, searches the published index and inspects its ref through managed MCP', async () => {
    const [paths, flow] = await converse('Enable an index for mapping.fig, whole file, then find the mapping review frame.', firstUseSteps);
    assert.ok(flow?.workspaceId, JSON.stringify(scripted.raw).slice(0, 2000));
    assert.equal(flow.before, 'absent');
    assert.equal(flow.job, 'succeeded', JSON.stringify(flow));
    assert.equal(flow.ref.nodeId, '0:1', JSON.stringify(flow));
    assert.deepEqual(flow.sources.map(source => source.state), ['ready']);
    assert.equal(flow.complete, true);
    assert.equal(flow.inspected, '0:1');
    // The Button component's instances, from the same published index and source as the search.
    assert.deepEqual(flow.usedBy, ['instance_of 0:2', 'instance_of 0:3'], JSON.stringify(flow));
    assert.equal(flow.usedSource, true);
    return { ...flow, sources: undefined, paths: paths.filter(item => /index|jobs|search|inspect|usages/.test(item)), provider: 'scripted loopback, not a model' };
  });
  await check('MCP reset asks the figlens_index permission: a deny rule keeps the database, an allow resets it', async () => {
    const [, refused] = await converse('Reset the design index.', resetSteps, [...allowAll, { action: 'figlens_index', resource: 'reset', effect: 'deny' }]);
    assert.match(refused?.refused ?? '', /permission|denied|rule/i, JSON.stringify(refused));
    const status = JSON.parse((await execute(binary, ['index', 'status', '--file', fixture], { cwd: project, env })).stdout);
    assert.equal(status.state, 'initialized', JSON.stringify(status));
    const [, allowed] = await converse('Reset the design index.', resetSteps);
    assert.ok(allowed?.reset, JSON.stringify(allowed));
    const after = JSON.parse((await execute(binary, ['index', 'status', '--file', fixture], { cwd: project, env })).stdout);
    assert.equal(after.state, 'uninitialized', JSON.stringify(after));
    assert.ok(existsSync(fixture));
    return { denied: refused.refused.slice(0, 200), afterAllow: after.state, designKept: true };
  });
  await check('production local select exports real PNG bytes', async () => {
    const selected = await rpc.run({ sessionID: session.id, action: 'select', file: 'mapping.fig', node: '0:1' }, options);
    assert.equal(selected.design.frame, '0:1');
    assert.equal(selected.design.instances.length, 2);
    assert.ok(selected.reference.uri.startsWith('data:image/png;base64,'));
    assert.ok(selected.referenceUrl.startsWith('file:'));
    assert.ok(selected.referenceUrl.includes('/.figlens/previews/'));
    assert.deepEqual(await readFile(new URL(selected.referenceUrl)), Buffer.from(selected.reference.uri.split(',')[1], 'base64'));
    const bytes = Buffer.from(selected.reference.uri.split(',')[1], 'base64');
    assert.equal(bytes.subarray(1, 4).toString(), 'PNG');
    assert.equal((await flow.read()).state.active.frame, '0:1');
    return { frame: selected.design.frame, imageBytes: bytes.length, mime: selected.reference.mime };
  });
  await check('native command attaches PNG bytes without starting a model', async () => {
    await client.session.command({ sessionID: session.id, name: 'figlens', text: 'open "mapping.fig" --frame 0:1' });
    const inbox = await client.session.inbox.list({ sessionID: session.id });
    const attachment = inbox.flatMap(item => item.payload?.files ?? []).find(file => file.mime === 'image/png');
    assert.ok(attachment);
    const bytes = Buffer.from(attachment.data, 'base64');
    assert.equal(bytes.subarray(1, 4).toString(), 'PNG');
    return { pngAttachmentQueued: true, imageBytes: bytes.length, resume: false };
  });
  await check('native review writer preserves Pi state and human notes', async () => {
    await flow.propose({ proposals: [proposal('0:2'), proposal('0:3')] });
    await writeFile(flow.guide, (await readFile(flow.guide, 'utf8')) + '\nHuman note: preserve this line.\n');
    const { candidates } = await mapping.pending({ sessionID: session.id }, options);
    assert.equal(candidates.length, 2);
    await mapping.review({ sessionID: session.id, candidateId: candidates[0].id, action: 'approve' }, options);
    await mapping.review({ sessionID: session.id, candidateId: candidates[1].id, action: 'skip' }, options);
    const bindings = await flow.bindings({});
    assert.equal(bindings.items[0].status, 'approved');
    assert.ok((await readFile(flow.guide, 'utf8')).endsWith('\nHuman note: preserve this line.\n'));
    return { statuses: (await flow.read()).state.mappings.map(item => item.status), humanNotePreserved: true };
  });
  await check('permission denial prevents selection writes and preview spawn', async () => {
    const denied = await client.session.create({ location, permissions: [{ action: '*', resource: '*', effect: 'allow' }, { action: 'edit', resource: '*', effect: 'deny' }] });
    const before = await readFile(flow.guide);
    for (const action of ['select', 'preview']) await assert.rejects(rpc.run({ sessionID: denied.id, action, file: 'mapping.fig', node: '0:1' }, options), error => error.type === 'permission_denied');
    assert.deepEqual(await readFile(flow.guide), before);
    return { guideUnchanged: true, rejectedBeforePreviewSpawn: true };
  });
  await check('two session viewers remain independent and deletion closes its owner', async () => {
    const other = await client.session.create({ location, permissions: [{ action: '*', resource: '*', effect: 'allow' }] });
    const a = await rpc.run({ sessionID: session.id, action: 'preview', file: 'mapping.fig' }, options);
    const b = await rpc.run({ sessionID: other.id, action: 'preview', file: 'mapping.fig' }, options);
    assert.notEqual(a.url, b.url);
    const aCatalog = await fetch(`${a.url}catalog`), bCatalog = await fetch(`${b.url}catalog`);
    assert.equal(aCatalog.status, 200);
    assert.equal(bCatalog.status, 200);
    await aCatalog.arrayBuffer();
    await bCatalog.arrayBuffer();
    await client.session.remove({ sessionID: other.id });
    let closed = false;
    for (let attempt = 0; attempt < 40; attempt++) {
      let response;
      try { response = await fetch(b.url); } catch { closed = true; break; }
      await response.arrayBuffer();
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    assert.ok(closed);
    const remaining = await fetch(a.url);
    assert.equal(remaining.status, 200);
    await remaining.arrayBuffer();
    activeUrl = a.url;
    return { distinctOwners: true, deletedSessionViewerClosed: true, otherViewerReady: true };
  });
  if (process.argv.includes('--tui')) await check('production TUI review command edits and confirms native dialogs', async () => {
    await flow.propose({ proposals: [proposal('0:2'), proposal('0:3')] });
    const ids = (await flow.pending()).map(item => item.id);
    const native = await client.session.create({ location, title: 'Mapping native UI', permissions: [{ action: '*', resource: '*', effect: 'allow' }] });
    const terminal = await execute('/usr/bin/python3', [fileURLToPath(new URL('./check-opencode-tui.py', import.meta.url)), host, url, native.id, output],
      { env: { ...env, TERM: 'xterm-256color', COLORTERM: 'truecolor', LANG: 'en_US.UTF-8' }, timeout: 180000 });
    const states = (await flow.read()).state.mappings.filter(item => ids.includes(item.id));
    assert.deepEqual(states.map(item => item.status), ['approved', 'skipped']);
    assert.equal(states[0].target.example, 'Button({ label: "native UI" })');
    return { ...JSON.parse(terminal.stdout), storedStates: states.map(item => item.status) };
  });
  await check('production reload closes remaining viewer and restores registration', async () => {
    assert.ok(activeUrl);
    await client.location.reload();
    await rpc.run({ sessionID: session.id, action: 'status' }, options);
    await assert.rejects(fetch(activeUrl));
    return { ownedViewerClosed: true, operationRpcReloaded: true };
  });
  await check('with a disabled MCP override the CLI path prepares, polls, searches, inspects and follows changes through plugin tools', async () => {
    // A disabled user server under the managed key: the plugin keeps it and serves the same flow over the CLI.
    await writeFile(path.join(config, 'opencode.json'), JSON.stringify({ plugins: [pathToFileURL(packaged).href, pathToFileURL(probe).href],
      mcp: { figlensmcp: { type: 'local', command: [binary, 'mcp'], enabled: false } } }));
    await client.location.reload();
    const status = await rpc.run({ sessionID: session.id, action: 'status' }, options);
    assert.equal(status.mcp.managed, false, JSON.stringify(status.mcp));
    for (const name of ['figlens_index', 'figlens_jobs', 'figlens_search', 'figlens_inspect', 'figlens_usages', 'figlens_changes', 'figlens_watch']) assert.ok(status.mcp.tools.includes(name), name);
    await execute(binary, ['index', 'reset', '--file', fixture], { cwd: project, env }).catch(() => {});
    const [before, prepared, job, found, node] = await converse('Enable an index for the mapping review frame (0:1) of mapping.fig, then find it.', cliSteps);
    assert.ok(node, JSON.stringify(scripted.raw).slice(0, 2000));
    assert.equal(prepared.job.state, 'succeeded', JSON.stringify(prepared.job));
    assert.equal(job.state, 'succeeded');
    assert.equal(found.results[0].ref.nodeId, '0:1');
    assert.equal(node.node.id, '0:1');
    const deep = scripted.raw[5] ?? '';
    assert.ok(deep.includes(`node ${deepId}`), `a ${deepId.length}-byte id must reach the binary, not fail the schema: ${deep.slice(0, 500)}`);
    const used = value(scripted.raw[6] ?? '');
    assert.deepEqual(used?.results?.map(item => item.ref.nodeId), ['0:2', '0:3'], (scripted.raw[6] ?? '').slice(0, 500));
    assert.equal(used.results[0].ref.sourceId, found.results[0].ref.sourceId);
    const [first, second] = [7, 8].map(index => value(scripted.raw[index] ?? ''));
    assert.deepEqual([first?.results?.map(item => item.ref.nodeId), first?.hasMore], [['0:2'], true], (scripted.raw[7] ?? '').slice(0, 500));
    assert.deepEqual([second?.results?.map(item => item.ref.nodeId), second?.hasMore], [['0:3'], false], (scripted.raw[8] ?? '').slice(0, 500));
    assert.match(JSON.parse(scripted.raw[9] ?? '{}').error?.message ?? '', /unknown relation "nope"/, (scripted.raw[9] ?? '').slice(0, 500));
    const [polled, acked, after] = [11, 12, 13].map(index => value(scripted.raw[index] ?? ''));
    assert.deepEqual(polled?.revisions?.map(revision => revision.kind), ['baseline'], (scripted.raw[11] ?? '').slice(0, 500));
    assert.equal(acked?.advanced, true, (scripted.raw[12] ?? '').slice(0, 500));
    assert.deepEqual(after?.revisions, [], (scripted.raw[13] ?? '').slice(0, 500));
    return { pages: [first.results[0].ref.nodeId, second.results[0].ref.nodeId], refused: 'unknown relation', trackedBaselineAcked: true, usedBy: used.results.map(item => item.ref.nodeId), deepIdBytes: deepId.length, mcp: status.mcp.reason, before: before.state, job: job.state, ref: found.results[0].ref, inspected: node.node.id, tools: scripted.tools.filter(name => /^figlens_/.test(name)) };
  });
  await check('with a disabled MCP override figlens_watch holds a lease per session that records a save, and only its session, deletion or a reload ends it', async () => {
    const cli = args => JSON.parse(execFileSync(binary, args, { cwd: project, env, encoding: 'utf8' }));
    const { workspaceId } = cli(['index', 'status', '--file', fixture]);
    const recorded = () => cli(['changes', '--id', workspaceId, '--scope', '0:1']).revisions.filter(revision => revision.kind === 'changes').length;
    const active = () => cli(['watch', 'status', '--id', workspaceId]).leases.filter(lease => lease.state === 'active');
    const until = async (what, done) => {
      for (const start = Date.now(); !done(); await new Promise(resolve => setTimeout(resolve, 100)))
        assert.ok(Date.now() - start < 15000, `timed out waiting for ${what}`);
    };
    const start = () => direct('figlens_watch', { action: 'start', workspaceId, scope: '0:1' });
    const original = await readFile(fixture);
    const before = recorded();
    try {
      const [denied] = await converse('Watch the review frame.', [start], [...allowAll, { action: 'read', resource: '*', effect: 'deny' }]);
      assert.deepEqual(active(), [], `a denied read starts no lease: ${JSON.stringify(denied).slice(0, 500)}`);
      const [status] = await converse('Show the watch status.', [() => direct('figlens_watch', { action: 'status', workspaceId })],
        [...allowAll, { action: 'edit', resource: '*', effect: 'deny' }]);
      assert.ok(Array.isArray(status?.leases), `status needs no edit: ${JSON.stringify(status).slice(0, 500)}`);
      // One session: start, a save the runner records with no index call, the plugin's changes, then its own stop.
      const [lease, , seen, stopped] = await converse('Watch the review frame while I save.', [start, held => {
        if (held?.state !== 'active') return null;
        writeFileSync(`${fixture}.next`, Buffer.concat([original, Buffer.alloc(16)]));
        renameSync(`${fixture}.next`, fixture);
        for (const start = Date.now(); recorded() === before && Date.now() - start < 15000;) execFileSync('/bin/sleep', ['0.1']);
        return direct('figlens_watch', { action: 'status', workspaceId });
      }, () => direct('figlens_changes', { workspaceId, scope: '0:1' }), held => direct('figlens_watch', { action: 'stop', leaseId: held.leaseId })]);
      assert.equal(lease?.state, 'active', JSON.stringify(lease));
      assert.equal(seen?.revisions?.filter(revision => revision.kind === 'changes').length, before + 1, JSON.stringify(seen).slice(0, 500));
      assert.deepEqual(stopped, { leaseId: lease.leaseId, state: 'ended', reason: 'released' });
      // Another session cannot stop a lease it did not start; deleting the owner releases it.
      const request = () => direct('figlens_watch', { action: 'start', workspaceId, scope: '0:1', requestId: 'acceptance' });
      const [owned, replayed] = await converse('Watch the review frame.', [request, request]);
      assert.equal(replayed?.leaseId, owned?.leaseId, `a requestId replays its lease in its session: ${JSON.stringify(replayed).slice(0, 500)}`);
      const owner = converse.session;
      const [refused] = await converse('Stop that watch.', [() => direct('figlens_watch', { action: 'stop', leaseId: owned.leaseId })]);
      assert.match(JSON.stringify(refused), /not a lease this session holds/);
      assert.deepEqual(active().map(lease => lease.leaseId), [owned.leaseId]);
      await client.session.remove({ sessionID: owner });
      await until('the deleted session to release its lease', () => !active().length);
      // A reload disposes the plugin, which releases every lease it held.
      const [reloaded] = await converse('Watch the review frame.', [start]);
      assert.equal(reloaded?.state, 'active', JSON.stringify(reloaded));
      await client.location.reload();
      await until('the reload to release its lease', () => !active().length);
      return { deniedReadStartsNothing: true, statusWithoutEdit: true, saveRecorded: true, ownStop: stopped.reason, requestIdReplayed: true, otherSessionRefused: true, deletionReleases: true, reloadReleases: true };
    } finally { await writeFile(fixture, original); }
  });
} finally {
  provider.close();
  server.kill('SIGTERM');
  const timer = setTimeout(() => server.kill('SIGKILL'), 10000);
  result.serverExit = await exited;
  clearTimeout(timer);
  await writeFile(path.join(output, 'results.json'), JSON.stringify(result, null, 2) + '\n');
  await writeFile(path.join(output, 'server.log'), logs.replaceAll(password, '[redacted]'));
  console.log(JSON.stringify(result, null, 2));
}
process.exitCode = result.tests.some(item => item.status === 'failed') ? 1 : 0;
