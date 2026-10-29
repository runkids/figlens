import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import test from 'node:test';
import { setTimeout } from 'node:timers/promises';
import { cliArgs, createAdapter, indexGrant, launcherRuntime } from './adapter.mjs';

async function context(t) {
  const root = await mkdtemp(path.join(tmpdir(), 'figlens-opencode-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const writes = [];
  const ctx = { location: { directory: root },
    session: { get: async () => ({ location: { directory: root } }), wait: async () => {}, prompt: async input => writes.push(input) },
    mcp: { list: async () => ({ data: [{ name: 'figlensmcp', status: { status: 'connected' } }] }) },
    tool: { list: async () => [] },
    permission: { assert: async () => {} },
  };
  const adapter = createAdapter(ctx);
  t.after(() => adapter.dispose());
  return { root, ctx, adapter, writes };
}

test('OpenCode preserves existing local, remote and disabled MCP settings and blocks unverified dispatch', async t => {
  const { root, adapter, writes } = await context(t);
  for (const config of [{ type: 'local', command: ['other', 'mcp'] }, { type: 'remote', url: 'https://example.invalid/mcp' },
    { type: 'local', command: ['other'], disabled: true }, { type: 'local', command: ['other'], enabled: false }]) {
    adapter.configureMcp({ get: () => config, set: () => assert.fail('User configuration must remain unchanged') });
    assert.equal(adapter.cliPath, config.disabled === true || config.enabled === false);
    await assert.rejects(adapter.commandHandler({ sessionID: 'session', prompt: { text: 'plan Only the hero' } }), /MCP is unavailable/);
  }
  assert.deepEqual(writes, []);
  assert.deepEqual(await readdir(root), []);
});

test('OpenCode shortcuts wait until the managed MCP tools reach the tool registry', async t => {
  const { root, ctx, writes } = await context(t);
  const override = process.env.FIGLENS_BIN;
  delete process.env.FIGLENS_BIN;
  t.after(() => {
    if (override === undefined) delete process.env.FIGLENS_BIN; else process.env.FIGLENS_BIN = override;
  });
  await mkdir(path.join(root, 'package/bin'), { recursive: true });
  await writeFile(path.join(root, 'package/package.json'), JSON.stringify({ version: '1.2.3' }));
  await writeFile(path.join(root, 'package/bin/figlens.js'), 'console.log("figlens 1.2.3");');
  let lists = 0;
  // Connected from the start; the server's tools appear only on the third registry read.
  ctx.tool.list = async () => ++lists < 3 ? [] : [{ id: 'figlensmcp_search', name: 'search', options: { namespace: 'figlensmcp' } }];
  const adapter = createAdapter(ctx, pathToFileURL(path.join(root, 'package/opencode/index.mjs')).href);
  t.after(() => adapter.dispose());
  adapter.configureMcp({ get: () => undefined, set: () => {} });
  await adapter.commandHandler({ sessionID: 'session', prompt: { text: 'open the hero' } });
  assert.equal(lists, 3);
  assert.equal(writes.length, 1);
});

test('OpenCode manages its default under a key apart from its figlens_* tools and keeps user servers under either key', async t => {
  const { adapter, ctx } = await context(t);
  const set = [];
  adapter.configureMcp({ get: () => undefined, set: (name, config) => set.push([name, config.command]) });
  assert.deepEqual(set.map(([name]) => name), ['figlensmcp']);
  assert.deepEqual((await adapter.operation({ sessionID: 'session', action: 'status' })).mcp.server, 'figlensmcp');
  for (const name of ['figlensmcp', 'figlens']) {
    ctx.mcp.list = async () => ({ data: [{ name, status: { status: 'connected' } }] });
    adapter.configureMcp({ get: key => key === name ? { type: 'local', command: ['other', 'mcp'] } : undefined,
      set: () => assert.fail('User configuration must remain unchanged') });
    const { mcp } = await adapter.operation({ sessionID: 'session', action: 'status' });
    assert.deepEqual([mcp.server, mcp.managed, mcp.status.status], [name, false, 'connected']);
    assert.equal(/rename it to figlensmcp/.test(mcp.reason), name === 'figlens');
    assert.equal(adapter.cliPath, name === 'figlens');
  }
});

test('OpenCode rejects a foreign session before creating project state', async t => {
  const { root, ctx, adapter } = await context(t);
  ctx.session.get = async () => ({ location: { directory: tmpdir() } });
  await assert.rejects(adapter.operation({ sessionID: 'foreign', action: 'select', file: 'missing.fig', node: '0:1' }), /different project/);
  assert.deepEqual(await readdir(root), []);
});

test('OpenCode gives each session an independent viewer lifetime', async t => {
  const { adapter } = await context(t);
  const a = await adapter.scope('a'), b = await adapter.scope('b');
  assert.notEqual(a.viewer, b.viewer);
  await adapter.close('a');
  assert.equal(a.signal.aborted, true);
  assert.equal(b.signal.aborted, false);
  await adapter.dispose();
  assert.equal(b.signal.aborted, true);
});

test('selection never exports through symlinked preview or guide directories', async t => {
  const { root, ctx } = await context(t);
  const external = await mkdtemp(path.join(tmpdir(), 'figlens-external-preview-'));
  t.after(() => rm(external, { recursive: true, force: true }));
  const override = process.env.FIGLENS_BIN;
  delete process.env.FIGLENS_BIN;
  t.after(() => {
    if (override === undefined) delete process.env.FIGLENS_BIN; else process.env.FIGLENS_BIN = override;
  });
  await mkdir(path.join(root, 'package/bin'), { recursive: true });
  await writeFile(path.join(root, 'package/bin/figlens.js'), `const fs = require('node:fs');
if (process.argv[2] === 'nodes') console.log(JSON.stringify({ document: { id: '0:1', name: 'Frame', type: 'FRAME', children: [] } }));
else if (process.argv[2] === 'export') fs.writeFileSync(process.argv[5], 'reference');
else throw new Error('Unexpected command');`);
  await writeFile(path.join(root, 'design.fig'), 'Synthetic snapshot');
  ctx.permission.assert = async ({ action, resources }) => {
    if (action === 'edit' && resources.some(resource => resource.startsWith(external))) throw new Error('External edit denied');
  };
  const adapter = createAdapter(ctx, pathToFileURL(path.join(root, 'package/opencode/index.mjs')).href);
  t.after(() => adapter.dispose());
  const directory = path.join(root, '.figlens');
  for (const linked of ['previews', '.figlens']) {
    if (linked === 'previews') await mkdir(directory);
    await symlink(external, linked === 'previews' ? path.join(directory, linked) : directory, 'junction');
    let failure;
    try { await adapter.operation({ sessionID: 'session', action: 'select', file: 'design.fig', node: '0:1' }); }
    catch (error) { failure = error; }
    assert.deepEqual(await readdir(external), [], 'Denied external destination must remain empty');
    assert.match(failure?.message ?? '', /symbolic link/);
    await rm(directory, { recursive: true, force: true });
  }
});

test('cancelling a bundled export stops its native child before cleanup', async t => {
  const { root, ctx } = await context(t);
  const override = process.env.FIGLENS_BIN;
  delete process.env.FIGLENS_BIN;
  t.after(() => {
    if (override === undefined) delete process.env.FIGLENS_BIN; else process.env.FIGLENS_BIN = override;
  });
  await mkdir(path.join(root, 'package/bin'), { recursive: true });
  await writeFile(path.join(root, 'package/bin/figlens.js'), `require('node:child_process').spawnSync(process.execPath, [require('node:path').join(__dirname, 'native.cjs'), ...process.argv.slice(2)], { stdio: 'inherit' });`);
  await writeFile(path.join(root, 'package/bin/native.cjs'), `const fs = require('node:fs');
if (process.argv[2] === 'nodes') console.log(JSON.stringify({ document: { id: '0:1', name: 'Frame', type: 'FRAME', children: [] } }));
else if (process.argv[2] === 'export') {
  fs.writeFileSync('export-started', process.argv[5]);
  setTimeout(() => fs.writeFileSync(process.argv[5], 'late reference'), 500);
} else throw new Error('Unexpected command');`);
  await writeFile(path.join(root, 'design.fig'), 'Synthetic snapshot');
  const adapter = createAdapter(ctx, pathToFileURL(path.join(root, 'package/opencode/index.mjs')).href);
  t.after(() => adapter.dispose());
  for (const cancel of ['rpc', 'session']) {
    const controller = new AbortController();
    const pending = adapter.operation({ sessionID: cancel, action: 'select', file: 'design.fig', node: '0:1' }, controller.signal);
    const rejected = assert.rejects(pending, { name: 'AbortError' });
    let output;
    for (let attempt = 0; attempt < 1000; attempt++) {
      try { output = await readFile(path.join(root, 'export-started'), 'utf8'); break; }
      catch (error) { if (error.code !== 'ENOENT') throw error; }
      await setTimeout(10);
    }
    assert.ok(output, 'Native export must start before cancellation');
    if (cancel === 'rpc') controller.abort(); else await adapter.close(cancel);
    await rejected;
    await setTimeout(700);
    assert.deepEqual(await readdir(path.dirname(output)), [], 'Cancelled native child must not publish a late PNG');
    await rm(path.join(root, 'export-started'));
  }
});

test('bundled launcher runs with node under a Bun-built host', () => {
  assert.equal(launcherRuntime({ bun: '1.4.3' }, '/opt/opencode'), 'node');
  assert.equal(launcherRuntime({ node: '24.0.0' }, '/usr/bin/node'), '/usr/bin/node');
});

test('CLI path runs the MCP index, jobs, search and inspect arguments as the same commands', () => {
  assert.deepEqual(cliArgs('index', { action: 'prepare', file: '/d.fig', scope: '0:0', requestId: 'r' }),
    ['index', 'prepare', '/d.fig', '--scope', '0:0', '--request-id', 'r']);
  assert.deepEqual(cliArgs('index', { action: 'status', file: '/d.fig' }), ['index', 'status', '--file', '/d.fig']);
  assert.deepEqual(cliArgs('index', { action: 'add', file: '/b.fig', workspaceId: 'ws_1' }), ['index', 'add', '/b.fig', '--id', 'ws_1']);
  assert.deepEqual(cliArgs('index', { action: 'relocate', sourceId: 's', file: '/n.fig', workspace: '/p' }),
    ['index', 'relocate', 's', '/n.fig', '--workspace', '/p']);
  assert.deepEqual(cliArgs('index', { action: 'refresh', workspaceId: 'ws_1', sourceId: 's', scope: '1:2' }),
    ['index', 'refresh', '--id', 'ws_1', '--source', 's', '--scope', '1:2']);
  assert.deepEqual(cliArgs('index', { action: 'rebuild', file: '/d.fig', scope: '0:0' }), ['index', 'rebuild', '--file', '/d.fig', '--scope', '0:0']);
  assert.deepEqual(cliArgs('jobs', { action: 'cancel', jobId: 'j' }), ['jobs', 'cancel', 'j']);
  assert.deepEqual(cliArgs('search', { workspaceId: 'ws_1', query: 'Pay now', limit: 5 }), ['search', '--id', 'ws_1', '--limit', '5', '--', 'Pay now']);
  assert.deepEqual(cliArgs('search', { file: '/d.fig', query: 'Pay', includeHidden: true }), ['search', '/d.fig', 'Pay', '--include-hidden']);
  assert.deepEqual(cliArgs('inspect', { file: '/d.fig', node: 'I1:2;3:4', depth: 0 }), ['inspect', '/d.fig', 'I1:2;3:4', '--depth', '0']);
  assert.throws(() => cliArgs('index', { action: 'remove', workspaceId: 'ws_1' }), /needs sourceId/);
});

test('CLI-path writes ask edit where the binary places the workspace; only reset and recreation ask figlens_index', async t => {
  const { ctx, adapter } = await context(t);
  const asked = [], ran = [];
  ctx.permission.assert = async ({ action, resources }) => { asked.push([action, ...resources]); };
  const current = await adapter.scope('session');
  let status = { paths: null, storage: { rule: 'default', remembered: null }, roots: { state: { path: '/state' }, config: { path: '/config' } } };
  current.run = async args => { ran.push(args.slice(0, 2).join(' ')); return { stdout: JSON.stringify(args[1] === 'status' ? status : { args }) }; };
  await adapter.cli(current, 'index', { action: 'status', file: '/d.fig' });
  assert.deepEqual(asked, [], 'status creates nothing and asks nothing');
  await adapter.cli(current, 'index', { action: 'prepare', file: '/d.fig', scope: '0:0' });
  assert.deepEqual(asked, [['edit', '/state', '/config']]);
  status = { ...status, paths: { workspace: '/custom/state/workspaces/ws_1' }, storage: { rule: 'remembered', remembered: '/custom' } };
  await adapter.cli(current, 'index', { action: 'reset', file: '/d.fig' });
  await adapter.cli(current, 'index', { action: 'reset', file: '/d.fig' });
  assert.deepEqual(asked.slice(1), [['figlens_index', 'reset'], ['edit', '/custom/state/workspaces/ws_1']]);
  assert.equal(asked.length, 3, 'the second reset is already granted');
  assert.deepEqual(ran, ['index status', 'index status', 'index prepare', 'index status', 'index reset', 'index status', 'index reset']);
});

test('CLI-path recreation asks edit on the new storage and the reported config root, though status fails', async t => {
  const { ctx, adapter } = await context(t);
  const asked = [], ran = [];
  ctx.permission.assert = async ({ action, resources }) => { asked.push([action, ...resources]); };
  const current = await adapter.scope('session');
  current.run = async args => {
    ran.push(args.slice(0, 2).join(' '));
    if (args[1] === 'status') throw Object.assign(new Error('exit 1'), { stderr: 'figlens: workspace is missing\n' + JSON.stringify({ recovery: [{ action: 'recreate_elsewhere', roots: { config: { path: '/config' }, state: { path: '/state' } } }] }) });
    return { stdout: '{}' };
  };
  await adapter.cli(current, 'index', { action: 'prepare', file: '/d.fig', storage: '/new', replaceUnavailable: true });
  assert.deepEqual(asked, [['figlens_index', 'replace_unavailable'], ['edit', '/new', '/config']]);
  assert.deepEqual(ran, ['index status', 'index prepare']);
});

test('CLI-path cancel asks edit on the job workspace, and a denial leaves the job running', async t => {
  const { ctx, adapter } = await context(t);
  const ran = [];
  ctx.permission.assert = async ({ action, resources }) => {
    if (action === 'edit') throw Object.assign(new Error(`denied ${resources}`), { _tag: 'Permission.DeclinedError' });
  };
  const current = await adapter.scope('session');
  current.run = async args => {
    ran.push(args.slice(0, 2).join(' '));
    return { stdout: JSON.stringify(args[0] === 'jobs' ? { workspaceId: 'ws_1', state: 'running' }
      : { paths: { workspace: '/state/workspaces/ws_1' }, storage: { rule: 'default' }, roots: {} }) };
  };
  await assert.rejects(adapter.cli(current, 'jobs', { action: 'cancel', jobId: 'ws_1.j' }), /denied \/state\/workspaces\/ws_1/);
  assert.deepEqual(ran, ['jobs status', 'index status']);
});

test('CLI-path flat inspect returns its text lines as the MCP flat field', async t => {
  const { adapter } = await context(t);
  const current = await adapter.scope('session');
  current.run = async () => ({ stdout: '0:1 FRAME "First success" @0,0 320x200\n' });
  assert.deepEqual(await adapter.cli(current, 'inspect', { file: '/d.fig', node: '0:1', flat: true }), { flat: '0:1 FRAME "First success" @0,0 320x200\n' });
});

test('CLI-path reset keeps every selector and scope field, so the CLI refuses a narrower or conflicting reset before any write', async t => {
  const { ctx, adapter } = await context(t);
  const asked = [], ran = [];
  ctx.permission.assert = async ({ action, resources }) => { asked.push([action, ...resources]); };
  const current = await adapter.scope('session');
  // As the binary: two selectors, or a field the action does not take, is a usage error.
  current.run = async args => {
    ran.push(args.join(' '));
    if (args.filter(arg => ['--id', '--file', '--workspace'].includes(arg)).length > 1 || (args[1] !== 'status' && args.includes('--source')))
      throw Object.assign(new Error('exit 2'), { stderr: 'usage: figlens index ...' });
    return { stdout: JSON.stringify({ paths: { workspace: '/state/workspaces/ws_a' }, storage: { rule: 'default' }, roots: { config: { path: '/config' } } }) };
  };
  await assert.rejects(adapter.cli(current, 'index', { action: 'reset', workspaceId: 'ws_a', file: '/b.fig' }), /usage/);
  assert.deepEqual(asked, [], 'conflicting selectors fail before any permission');
  await assert.rejects(adapter.cli(current, 'index', { action: 'reset', workspaceId: 'ws_a', sourceId: 's' }), /usage/);
  await assert.rejects(adapter.cli(current, 'index', { action: 'prepare', file: '/b.fig', workspaceId: 'ws_a', storage: '/new', replaceUnavailable: true }), /usage/);
  assert.deepEqual(ran.filter(command => !command.startsWith('index status')), ['index reset --id ws_a --source s']);
});
