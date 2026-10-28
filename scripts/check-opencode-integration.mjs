// Real OpenCode server and native UI against the packaged entries; never starts a model.
import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises';
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
await writeFile(path.join(project, 'src/Button.mjs'), 'export const Button = () => "button";\n');
await writeFile(path.join(config, 'opencode.json'), JSON.stringify({ plugins: [pathToFileURL(packaged).href] }));
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
    let status;
    for (let attempt = 0; attempt < 60; attempt++) {
      status = await rpc.run({ sessionID: session.id, action: 'status' }, options);
      if (status.mcp.status.status === 'connected') break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    assert.equal(status.mcp.managed, true);
    assert.equal(status.mcp.status.status, 'connected');
    for (const tool of ['figlens_preview', 'figlens_bindings', 'figlens_propose_bindings']) assert.ok(status.mcp.tools.includes(tool), tool);
    const skills = await client.skill.list({ location });
    assert.ok(JSON.stringify(skills).includes('figlens'));
    return { mcp: status.mcp, skills: skills.data.map(skill => skill.name) };
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
    assert.equal((await fetch(`${a.url}catalog`)).status, 200);
    assert.equal((await fetch(`${b.url}catalog`)).status, 200);
    await client.session.remove({ sessionID: other.id });
    let closed = false;
    for (let attempt = 0; attempt < 40; attempt++) {
      try { await fetch(b.url); } catch { closed = true; break; }
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    assert.ok(closed);
    assert.equal((await fetch(a.url)).status, 200);
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
} finally {
  server.kill('SIGTERM');
  const timer = setTimeout(() => server.kill('SIGKILL'), 10000);
  result.serverExit = await exited;
  clearTimeout(timer);
  await writeFile(path.join(output, 'results.json'), JSON.stringify(result, null, 2) + '\n');
  await writeFile(path.join(output, 'server.log'), logs.replaceAll(password, '[redacted]'));
  console.log(JSON.stringify(result, null, 2));
}
process.exitCode = result.tests.some(item => item.status === 'failed') ? 1 : 0;
