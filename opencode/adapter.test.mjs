import assert from 'node:assert/strict';
import { copyFile, link, mkdir, mkdtemp, readFile, readdir, realpath, rm, symlink, writeFile } from 'node:fs/promises';
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

// The paths a read asks, deduped as the adapter does: a design's canonical path equals its requested one unless a tmp
// ancestor is a link (macOS /var -> /private/var).
const reads = (...files) => [...new Set(files)];

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

test('OpenCode closes a session\'s viewer and owner even when one of its commands fails to stop', async t => {
  const { adapter } = await context(t);
  const a = await adapter.scope('a');
  let viewerClosed = false;
  const closeViewer = a.viewer.close.bind(a.viewer);
  a.viewer.close = async () => { viewerClosed = true; await closeViewer(); };
  a.commands.add(() => Promise.reject(new Error('stop failed')));
  await assert.rejects(adapter.close('a'), /stop failed/);
  const again = await adapter.scope('a');
  assert.deepEqual([viewerClosed, again.viewer === a.viewer, again.signal.aborted], [true, false, false]);
  await adapter.dispose();
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

test('CLI path runs the MCP index, jobs, search, usages, diff, changes, watch and inspect arguments as the same commands', () => {
  assert.deepEqual(cliArgs('index', { action: 'prepare', file: '/d.fig', scope: '0:0', requestId: 'r' }),
    ['index', 'prepare', '/d.fig', '--scope', '0:0', '--request-id', 'r']);
  assert.deepEqual(cliArgs('index', { action: 'status', file: '/d.fig' }), ['index', 'status', '--file', '/d.fig']);
  assert.deepEqual(cliArgs('index', { action: 'add', file: '/b.fig', workspaceId: 'ws_1' }), ['index', 'add', '/b.fig', '--id', 'ws_1']);
  assert.deepEqual(cliArgs('index', { action: 'relocate', sourceId: 's', file: '/n.fig', workspace: '/p' }),
    ['index', 'relocate', 's', '/n.fig', '--workspace', '/p']);
  assert.deepEqual(cliArgs('index', { action: 'refresh', workspaceId: 'ws_1', sourceId: 's', scope: '1:2' }),
    ['index', 'refresh', '--id', 'ws_1', '--source', 's', '--scope', '1:2']);
  assert.deepEqual(cliArgs('index', { action: 'rebuild', file: '/d.fig', scope: '0:0' }), ['index', 'rebuild', '--file', '/d.fig', '--scope', '0:0']);
  assert.deepEqual(cliArgs('index', { action: 'track', workspaceId: 'ws_1', sourceId: 's', scope: '1:2' }),
    ['index', 'track', '--id', 'ws_1', '--source', 's', '--scope', '1:2']);
  assert.deepEqual(cliArgs('changes', { workspaceId: 'ws_1', scope: '1:2', consumer: 'ci', limit: 5 }),
    ['changes', '--id', 'ws_1', '--scope', '1:2', '--consumer', 'ci', '--limit', '5']);
  assert.deepEqual(cliArgs('changes', { workspaceId: 'ws_1', sourceId: 's', revision: 'r', cursor: 'd1.x', storage: '/s' }),
    ['changes', '--id', 'ws_1', '--source', 's', '--storage', '/s', '--cursor', 'd1.x', '--revision', 'r']);
  assert.deepEqual(cliArgs('watch', { action: 'ack', workspaceId: 'ws_1', scope: '1:2', consumer: 'ci', cursor: 'c1.x' }),
    ['watch', 'ack', '--id', 'ws_1', '--scope', '1:2', '--consumer', 'ci', '--cursor', 'c1.x']);
  assert.throws(() => cliArgs('watch', { action: 'ack', workspaceId: 'ws_1', consumer: 'ci' }), /needs cursor/);
  assert.deepEqual(cliArgs('watch', { action: 'subscribe', workspaceId: 'ws_1', sourceId: 's', consumer: 'ci', node: '1:2', owner: 't' }),
    ['watch', 'subscribe', '--id', 'ws_1', '--source', 's', '--consumer', 'ci', '--node', '1:2', '--owner', 't']);
  assert.deepEqual(cliArgs('watch', { action: 'subscribe', workspaceId: 'ws_1', sourceId: 's', consumer: 'ci', query: 'Pay now', type: 'TEXT' }),
    ['watch', 'subscribe', '--id', 'ws_1', '--source', 's', '--consumer', 'ci', '--query', 'Pay now', '--type', 'TEXT']);
  assert.deepEqual(cliArgs('watch', { action: 'subscribe', workspaceId: 'ws_1', sources: 'visible', consumer: 'ci',
    library: { kind: 'component', namespace: 'ns', key: 'k', definition: { sourceId: 's', snapshotId: 'sha256:a', nodeId: '1:2' } } }),
    ['watch', 'subscribe', '--id', 'ws_1', '--sources', 'visible', '--consumer', 'ci', '--library', 'component', '--namespace', 'ns',
      '--key', 'k', '--definition-source', 's', '--definition-snapshot', 'sha256:a', '--definition-node', '1:2']);
  assert.deepEqual(cliArgs('watch', { action: 'evaluate', workspaceId: 'ws_1', subscriptionId: 'sub_1' }),
    ['watch', 'evaluate', '--id', 'ws_1', '--subscription', 'sub_1']);
  assert.deepEqual(cliArgs('watch', { action: 'start', workspaceId: 'ws_1', sourceId: 's', scope: '0:0', storage: '/s', requestId: 'r' }),
    ['watch', 'run', '--id', 'ws_1', '--source', 's', '--scope', '0:0', '--storage', '/s', '--request-id', 'r']);
  assert.throws(() => cliArgs('watch', { action: 'start', workspaceId: 'ws_1' }), /needs scope/);
  assert.deepEqual(cliArgs('watch', { action: 'status', workspaceId: 'ws_1' }), ['watch', 'status', '--id', 'ws_1']);
  assert.deepEqual(cliArgs('watch', { action: 'unsubscribe', workspaceId: 'ws_1', subscriptionId: 'sub_1' }),
    ['watch', 'unsubscribe', '--id', 'ws_1', '--subscription', 'sub_1']);
  assert.deepEqual(cliArgs('changes', { workspaceId: 'ws_1', subscriptionId: 'sub_1' }), ['changes', '--id', 'ws_1', '--subscription', 'sub_1']);
  assert.throws(() => cliArgs('changes', { scope: '1:2' }), /needs workspaceId/);
  assert.deepEqual(cliArgs('index', { action: 'untrack', file: '/d.fig', scope: '1:2' }), ['index', 'untrack', '--file', '/d.fig', '--scope', '1:2']);
  assert.deepEqual(cliArgs('jobs', { action: 'cancel', jobId: 'j' }), ['jobs', 'cancel', 'j']);
  assert.deepEqual(cliArgs('search', { workspaceId: 'ws_1', query: 'Pay now', limit: 5 }), ['search', '--id', 'ws_1', '--limit', '5', '--', 'Pay now']);
  assert.deepEqual(cliArgs('search', { file: '/d.fig', query: 'Pay', includeHidden: true }), ['search', '/d.fig', 'Pay', '--include-hidden']);
  assert.deepEqual(cliArgs('inspect', { file: '/d.fig', node: 'I1:2;3:4', depth: 0 }), ['inspect', '/d.fig', 'I1:2;3:4', '--depth', '0']);
  assert.deepEqual(cliArgs('diff', { old: '/a.fig', file: '/b.fig', node: '1:2', limit: 5, cursor: 'c' }),
    ['diff', '/a.fig', '/b.fig', '--node', '1:2', '--limit', '5', '--cursor', 'c', '--json']);
  assert.deepEqual(cliArgs('diff', { old: '/a.fig', file: '/b.fig' }), ['diff', '/a.fig', '/b.fig', '--json']);
  assert.throws(() => cliArgs('diff', { file: '/b.fig' }), /needs old/);
  assert.deepEqual(cliArgs('inspect', { file: '/d.fig', node: '1:2', compareVariant: '1:3' }), ['inspect', '/d.fig', '1:2', '--compare-variant', '1:3']);
  for (const input of [{ file: '/a.fig', workspaceId: 'ws_p', sourceId: 'b', query: 'Pay' }, { file: '/a.fig', storage: '/s', query: 'Pay' }, { query: 'Pay' }])
    assert.throws(() => cliArgs('search', input), /file or workspaceId, not both/);
  assert.throws(() => cliArgs('index', { action: 'remove', workspaceId: 'ws_1' }), /needs sourceId/);
  assert.deepEqual(cliArgs('usages', { file: '/d.fig', node: 'I1:2;3:4', relation: 'instance_of,uses_style', direction: 'out', includeHidden: true, limit: 5 }),
    ['usages', '/d.fig', 'I1:2;3:4', '--relation', 'instance_of,uses_style', '--out', '--include-hidden', '--limit', '5']);
  assert.deepEqual(cliArgs('usages', { workspaceId: 'ws_1', sourceId: 's', resource: 'VariableID:1:2', scope: '0:1', direction: 'in', snapshotId: 'snap', cursor: 'c' }),
    ['usages', '--id', 'ws_1', '--source', 's', 'VariableID:1:2', '--scope', '0:1', '--snapshot', 'snap', '--cursor', 'c']);
  for (const [input, message] of [[{ file: '/a.fig', workspaceId: 'ws_p', node: '1:2' }, /file or workspaceId, not both/],
    [{ file: '/a.fig', sourceId: 's', node: '1:2' }, /file or workspaceId, not both/], [{ file: '/a.fig', node: '1:2', resource: 'image:a' }, /node or resource, not both/],
    [{ file: '/a.fig', resource: '1:2' }, /pass a layer id as node/], [{ file: '/a.fig', node: 'StyleKey:k' }, /as resource/]])
    assert.throws(() => cliArgs('usages', input), message);
  const library = { kind: 'component', namespace: 'lib', key: 'k1' };
  assert.deepEqual(cliArgs('usages', { workspaceId: 'ws_1', sourceId: 's', library, scope: '0:1', includeHidden: true, limit: 5, cursor: 'c' }),
    ['usages', '--id', 'ws_1', '--source', 's', '--library', 'component', '--namespace', 'lib', '--key', 'k1', '--scope', '0:1', '--include-hidden', '--limit', '5', '--cursor', 'c']);
  for (const extra of [{ node: '1:2' }, { resource: 'image:a' }, { snapshotId: 'snap' }, { relation: 'uses_style' }, { direction: 'in' }])
    assert.throws(() => cliArgs('usages', { workspaceId: 'ws_1', library, ...extra }), new RegExp(`library takes no ${Object.keys(extra)[0]}`));
  assert.throws(() => cliArgs('usages', { file: '/a.fig', library }), /library takes no file/);
  // Without namespace, and with an exact definition ref, as MCP takes them.
  assert.deepEqual(cliArgs('usages', { workspaceId: 'ws_1', library: { kind: 'component', key: 'k1', definition: { sourceId: 's', snapshotId: 'sha256:a', nodeId: '1:2' } } }),
    ['usages', '--id', 'ws_1', '--library', 'component', '--key', 'k1', '--definition-source', 's', '--definition-snapshot', 'sha256:a', '--definition-node', '1:2']);
  assert.throws(() => cliArgs('usages', { workspaceId: 'ws_1', library: { kind: 'component', key: 'k1', definition: { sourceId: 's' } } }), /needs snapshotId/);
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
  assert.deepEqual(asked, [['read', '/d.fig'], ['edit', '/state', '/config']]);
  status = { ...status, paths: { workspace: '/custom/state/workspaces/ws_1', lock: '/custom/state/workspaces/ws_1.lock' }, storage: { rule: 'remembered', remembered: '/custom' } };
  await adapter.cli(current, 'index', { action: 'reset', file: '/d.fig' });
  await adapter.cli(current, 'index', { action: 'reset', file: '/d.fig' });
  assert.deepEqual(asked.slice(2), [['figlens_index', 'reset'], ['edit', '/custom/state/workspaces/ws_1', '/custom/state/workspaces/ws_1.lock']]);
  assert.equal(asked.length, 4, 'the second reset is already granted');
  assert.deepEqual(ran, ['index status', 'index status', 'index prepare', 'index status', 'index reset', 'index status', 'index reset']);
  // A relocation records the new path's alias under the config root.
  const other = await adapter.scope('other');
  other.run = current.run;
  await adapter.cli(other, 'index', { action: 'relocate', file: '/moved.fig', sourceId: 'local:1', workspaceId: 'ws_1' });
  assert.deepEqual(asked.slice(4), [['read', '/moved.fig'], ['edit', '/custom/state/workspaces/ws_1', '/custom/state/workspaces/ws_1.lock', '/config']]);
});

test('CLI-path writes also ask edit on the canonical target of a symlinked root, so denying it blocks the write', async t => {
  const { root, ctx, adapter } = await context(t);
  await mkdir(path.join(root, 'target'));
  await symlink(path.join(root, 'target'), path.join(root, 'alias'));
  const real = path.join(await realpath(root), 'target');
  const asked = [], ran = [];
  ctx.permission.assert = async ({ action, resources }) => {
    asked.push([action, ...resources]);
    if (action === 'edit' && resources.some(file => file.startsWith(real))) throw Object.assign(new Error('denied'), { _tag: 'Permission.DeclinedError' });
  };
  const current = await adapter.scope('session');
  const roots = { state: { path: path.join(root, 'alias', 'state') }, config: { path: path.join(root, 'alias', 'config') } };
  current.run = async args => {
    ran.push(args.slice(0, 2).join(' '));
    return { stdout: JSON.stringify(args[1] === 'status' ? { paths: null, roots } : {}) };
  };
  await assert.rejects(adapter.cli(current, 'index', { action: 'prepare', file: '/d.fig', scope: '0:0' }), /denied/);
  assert.deepEqual(asked.at(-1), ['edit', roots.state.path, path.join(real, 'state'), roots.config.path, path.join(real, 'config')]);
  // Recreation places a new storage that does not exist yet; its existing parent resolves through the link.
  await assert.rejects(adapter.cli(current, 'index', { action: 'prepare', file: '/d.fig', storage: path.join(root, 'alias', 'new', 'store'), replaceUnavailable: true }), /denied/);
  assert.deepEqual(asked.at(-1), ['edit', path.join(root, 'alias', 'new', 'store'), path.join(real, 'new', 'store'),
    path.join(root, 'alias', 'new', 'store', 'state'), path.join(real, 'new', 'store', 'state'), roots.config.path, path.join(real, 'config')]);
  // A `..` after the link climbs from the link's target, as the binary's canonicalize does.
  await mkdir(path.join(root, 'target', 'deep'));
  await symlink(path.join(root, 'target', 'deep'), path.join(root, 'down'));
  await assert.rejects(adapter.cli(current, 'index', { action: 'prepare', file: '/d.fig', storage: `${path.join(root, 'down')}${path.sep}..${path.sep}s`, replaceUnavailable: true }), /denied/);
  assert.ok(asked.at(-1).includes(path.join(real, 's')), 'the physical parent of the link is asked');
  // A dangling link is followed to where the write would land.
  await symlink(path.join(real, 'gone'), path.join(root, 'dangling'));
  await assert.rejects(adapter.cli(current, 'index', { action: 'prepare', file: '/d.fig', storage: path.join(root, 'dangling'), replaceUnavailable: true }), /denied/);
  assert.ok(asked.at(-1).includes(path.join(real, 'gone', 'state')), 'the dangling target is asked');
  assert.deepEqual(ran, ['index status', 'index status', 'index status', 'index status'], 'nothing writes past a denied target');
});

test('CLI-path reads ask on the design a `..` after a link really names, so denying it blocks the read', async t => {
  const { root, ctx, adapter } = await context(t);
  const real = await realpath(root);
  await mkdir(path.join(root, 'denied', 'deep'), { recursive: true });
  await writeFile(path.join(root, 'denied', 'design.fig'), '');
  await mkdir(path.join(root, 'allowed'));
  await symlink(path.join(root, 'denied', 'deep'), path.join(root, 'allowed', 'link'));
  const asked = [], ran = [];
  ctx.permission.assert = async ({ action, resources }) => {
    asked.push([action, ...resources]);
    if (resources.some(file => file.startsWith(path.join(real, 'denied')))) throw Object.assign(new Error('denied'), { _tag: 'Permission.DeclinedError' });
  };
  const current = await adapter.scope('session');
  current.run = async args => { ran.push(args[0]); return { stdout: '{}' }; };
  const file = `${path.join(root, 'allowed', 'link')}${path.sep}..${path.sep}design.fig`;
  await assert.rejects(adapter.cli(current, 'search', { file, query: 'x' }), /denied/);
  assert.deepEqual(asked, [['read', path.join(root, 'allowed', 'design.fig'), path.join(real, 'denied', 'design.fig')]]);
  assert.deepEqual(ran, [], 'nothing reads past a denied target');
});

test('CLI-path recreation asks edit on the state child the raw storage really names, so a denied one blocks it', async t => {
  const { root, ctx, adapter } = await context(t);
  const real = await realpath(root);
  // allowed/link leads to actual/deep, so allowed/link/../store is actual/store, whose state links to denied.
  for (const dir of ['allowed', 'actual/deep', 'actual/store', 'denied']) await mkdir(path.join(root, dir), { recursive: true });
  await symlink(path.join(root, 'actual', 'deep'), path.join(root, 'allowed', 'link'));
  await symlink(path.join(root, 'denied'), path.join(root, 'actual', 'store', 'state'));
  const asked = [], ran = [];
  ctx.permission.assert = async ({ action, resources }) => {
    asked.push([action, ...resources]);
    if (action === 'edit' && resources.some(file => file.startsWith(path.join(real, 'denied')))) throw Object.assign(new Error('denied'), { _tag: 'Permission.DeclinedError' });
  };
  const current = await adapter.scope('session');
  current.run = async args => {
    ran.push(args.slice(0, 2).join(' '));
    if (args[1] === 'status') throw Object.assign(new Error('exit 1'), { stderr: 'figlens: workspace is missing\n' + JSON.stringify({ recovery: [{ action: 'recreate_elsewhere', roots: { config: { path: path.join(root, 'allowed', 'config') }, state: { path: path.join(root, 'allowed', 'state') } } }] }) });
    return { stdout: '{}' };
  };
  const storage = `${path.join(root, 'allowed', 'link')}${path.sep}..${path.sep}store`;
  await assert.rejects(adapter.cli(current, 'index', { action: 'prepare', file: '/d.fig', storage, replaceUnavailable: true }), /denied/);
  assert.ok(asked.at(-1).includes(path.join(real, 'actual', 'store')), 'the storage the link really names is asked');
  assert.ok(asked.at(-1).includes(path.join(real, 'denied')), 'its state child is asked where it really leads');
  assert.deepEqual(ran, ['index status'], 'nothing writes past a denied state child');
});

test('CLI-path writes fail closed before any edit ask when a write root runs through a file or a link loop', async t => {
  const { root, ctx, adapter } = await context(t);
  await writeFile(path.join(root, 'file'), '');
  await symlink(path.join(root, 'loop'), path.join(root, 'loop'));
  const asked = [], ran = [];
  ctx.permission.assert = async ({ action, resources }) => { asked.push([action, ...resources]); };
  const current = await adapter.scope('session');
  const roots = { state: { path: path.join(root, 'state') }, config: { path: path.join(root, 'config') } };
  current.run = async args => { ran.push(args.slice(0, 2).join(' ')); return { stdout: JSON.stringify(args[1] === 'status' ? { paths: null, roots } : {}) }; };
  for (const [storage, code] of [[path.join(root, 'file', 'store'), 'ENOTDIR'], [path.join(root, 'loop', 'store'), 'ELOOP']])
    await assert.rejects(adapter.cli(current, 'index', { action: 'prepare', file: '/d.fig', storage, replaceUnavailable: true }), { code });
  assert.deepEqual(asked.filter(([action]) => action === 'edit'), []);
  assert.deepEqual(ran, ['index status', 'index status'], 'nothing runs past the failed resolution');
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
  assert.deepEqual(asked, [['read', '/d.fig'], ['figlens_index', 'replace_unavailable'], ['edit', '/new', '/new/state', '/config']]);
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
      : { paths: { workspace: '/state/workspaces/ws_1', lock: '/state/workspaces/ws_1.lock' }, storage: { rule: 'default' }, roots: {} }) };
  };
  // Cancelling opens the workspace exclusively, which recreates a missing sibling lock.
  await assert.rejects(adapter.cli(current, 'jobs', { action: 'cancel', jobId: 'ws_1.j' }), /denied \/state\/workspaces\/ws_1,\/state\/workspaces\/ws_1\.lock$/);
  // An ack writes its checkpoint into the same database.
  await assert.rejects(adapter.cli(current, 'watch', { action: 'ack', workspaceId: 'ws_1', consumer: 'ci', cursor: 'c1.x' }), /denied \/state\/workspaces\/ws_1,\/state\/workspaces\/ws_1\.lock$/);
  assert.deepEqual(ran, ['jobs status', 'index status', 'index status']);
});

test('CLI-path flat inspect returns its text lines as the MCP flat field', async t => {
  const { adapter } = await context(t);
  const current = await adapter.scope('session');
  current.run = async () => ({ stdout: '0:1 FRAME "First success" @0,0 320x200\n' });
  assert.deepEqual(await adapter.cli(current, 'inspect', { file: '/d.fig', node: '0:1', flat: true }), { flat: '0:1 FRAME "First success" @0,0 320x200\n' });
});

test('CLI-path prepare and reset run the recovery a failing status names, after asking for its place', async t => {
  const { ctx, adapter } = await context(t);
  const asked = [], ran = [];
  let deny = null;
  ctx.permission.assert = async ({ action, resources }) => {
    asked.push([action, ...resources]);
    if (resources.includes(deny)) throw Object.assign(new Error('denied'), { _tag: 'Permission.DeclinedError' });
  };
  const current = await adapter.scope('session');
  const roots = { config: { path: '/config' } };
  const lock = '/state/workspaces/ws_1.lock';
  const recovery = [{ action: 'prepare', requires: 'none', path: '/state/workspaces/ws_1', lock, roots },
    { action: 'reset', requires: 'user_confirmation', path: '/state/workspaces/ws_1', lock, roots }];
  current.run = async args => {
    ran.push(args.slice(0, 2).join(' '));
    if (args[1] === 'status') throw Object.assign(new Error('exit 1'), { stderr: 'figlens: ws_1.lock is missing\n' + JSON.stringify({ recovery }) });
    return { stdout: '{}' };
  };
  // Denying the config root or the sibling lock the recovery recreates stops it before any write.
  for (deny of ['/config', lock]) await assert.rejects(adapter.cli(current, 'index', { action: 'prepare', file: '/d.fig' }), /denied/);
  deny = null;
  await adapter.cli(current, 'index', { action: 'prepare', file: '/d.fig' });
  await adapter.cli(current, 'index', { action: 'reset', file: '/d.fig' });
  const asks = ['edit', '/state/workspaces/ws_1', lock, '/config'];
  assert.deepEqual(asked, [['read', '/d.fig'], asks, asks, asks, ['figlens_index', 'reset']]);
  await assert.rejects(adapter.cli(current, 'index', { action: 'refresh', file: '/d.fig' }), /lock is missing/);
  assert.deepEqual(ran, ['index status', 'index status', 'index status', 'index prepare', 'index status', 'index reset', 'index status']);
  // An older binary's recovery without the lock path is refused.
  recovery.forEach(action => delete action.lock);
  await assert.rejects(adapter.cli(current, 'index', { action: 'prepare', file: '/d.fig' }), /does not report its workspace's lock file/);
});

test('CLI-path design reads ask read on the requested and canonical path, and builds run without the command timeout', async t => {
  const { root, ctx, adapter } = await context(t);
  const asked = [], timeouts = [];
  ctx.permission.assert = async ({ action, resources }) => {
    asked.push([action, ...resources]);
    if (action === 'read') throw Object.assign(new Error('read denied'), { _tag: 'Permission.DeclinedError' });
  };
  const current = await adapter.scope('session');
  current.run = async (args, timeout) => { timeouts.push([args[1], timeout]); return { stdout: '{}' }; };
  await writeFile(path.join(root, 'real.fig'), '');
  await symlink(path.join(root, 'real.fig'), path.join(root, 'link.fig'));
  await assert.rejects(adapter.cli(current, 'search', { file: 'link.fig', query: 'a' }), /read denied/);
  assert.deepEqual(asked, [['read', path.join(current.flow.root, 'link.fig'), await realpath(path.join(root, 'real.fig'))]]);
  assert.deepEqual(timeouts, [], 'nothing ran');
  ctx.permission.assert = async () => {};
  current.run = async (args, timeout) => {
    timeouts.push([args[1], timeout]);
    return { stdout: JSON.stringify({ paths: { workspace: '/w', lock: '/w.lock' }, storage: { rule: 'default' }, roots: { config: { path: '/c' } }, sources: [] }) };
  };
  await adapter.cli(current, 'index', { action: 'refresh', workspaceId: 'ws_1' });
  assert.deepEqual(timeouts, [['status', undefined], ['refresh', 0]]);
});

test('CLI-path directory add asks read on the directory and on every file the binary lists, before any write', async t => {
  const { root, ctx, adapter } = await context(t);
  await mkdir(path.join(root, 'lib'));
  const real = await realpath(root);
  const listed = [path.join(real, 'lib', 'a.fig'), path.join(real, 'lib', 'sub', 'b.fig')];
  const asked = [], ran = [];
  let deny = listed[1];
  ctx.permission.assert = async ({ action, resources }) => {
    asked.push([action, ...resources]);
    if (resources.includes(deny)) throw Object.assign(new Error('denied'), { _tag: 'Permission.DeclinedError' });
  };
  const run = async args => {
    ran.push(args.join(' '));
    if (args.includes('--dry-run')) return { stdout: JSON.stringify({ directory: { files: listed, skipped: [], manifest: 'm1' } }) };
    return { stdout: JSON.stringify(args[1] === 'status' ? { paths: { workspace: '/w', lock: '/w.lock' }, roots: { config: { path: '/c' } } } : {}) };
  };
  const input = { action: 'add', file: 'lib', workspaceId: 'ws_1' };
  const current = await adapter.scope('session');
  current.run = run;
  await assert.rejects(adapter.cli(current, 'index', input), /denied/);
  assert.deepEqual(asked, [['read', ...new Set([path.join(current.flow.root, 'lib'), path.join(real, 'lib')])], ['read', ...listed]]);
  assert.deepEqual(ran, ['index add lib --id ws_1 --dry-run'], 'nothing registers past a denied file');
  deny = null;
  const other = await adapter.scope('other');
  other.run = run;
  ran.length = 0;
  await adapter.cli(other, 'index', input);
  assert.deepEqual(asked.slice(-2), [['read', ...listed], ['edit', '/w', '/w.lock', '/c']]);
  assert.deepEqual(ran, ['index add lib --id ws_1 --dry-run', 'index status --id ws_1', 'index add lib --id ws_1 --manifest m1'], 'the add is bound to the files the dry run listed');
});

test('CLI-path directory add detects a directory reached through a link and .., as the binary does', async t => {
  const { root, ctx, adapter } = await context(t);
  // link/../designs is outside/designs to the binary, while a lexical resolve would look for <root>/designs.
  const outside = await mkdtemp(path.join(tmpdir(), 'figlens-outside-'));
  t.after(() => rm(outside, { recursive: true, force: true }));
  await mkdir(path.join(outside, 'sub'));
  await mkdir(path.join(outside, 'designs'));
  await symlink(path.join(outside, 'sub'), path.join(root, 'link'));
  const listed = [path.join(await realpath(outside), 'designs', 'a.fig')];
  const ran = [];
  let deny = listed[0];
  ctx.permission.assert = async ({ resources }) => {
    if (resources.includes(deny)) throw Object.assign(new Error('denied'), { _tag: 'Permission.DeclinedError' });
  };
  const run = async args => {
    ran.push(args.join(' '));
    if (args.includes('--dry-run')) return { stdout: JSON.stringify({ directory: { files: listed, skipped: [], manifest: 'm3' } }) };
    return { stdout: JSON.stringify(args[1] === 'status' ? { paths: { workspace: '/w', lock: '/w.lock' }, roots: { config: { path: '/c' } } } : {}) };
  };
  const input = { action: 'add', file: 'link/../designs', workspaceId: 'ws_1' };
  const current = await adapter.scope('session');
  current.run = run;
  await assert.rejects(adapter.cli(current, 'index', input), /denied/);
  assert.deepEqual(ran, ['index add link/../designs --id ws_1 --dry-run'], 'nothing registers past a denied file');
  deny = null;
  const other = await adapter.scope('other');
  other.run = run;
  ran.length = 0;
  await adapter.cli(other, 'index', input);
  assert.deepEqual(ran.at(-1), 'index add link/../designs --id ws_1 --manifest m3');
});

test('CLI-path directory add forwards its cursor to the dry run and the add, asking read on that page', async t => {
  const { root, ctx, adapter } = await context(t);
  await mkdir(path.join(root, 'lib'));
  const page = [path.join(await realpath(root), 'lib', 'k.fig')];
  const asked = [], ran = [];
  ctx.permission.assert = async ({ action, resources }) => { asked.push([action, ...resources]); };
  const current = await adapter.scope('session');
  current.run = async args => {
    ran.push(args.join(' '));
    if (args.includes('--dry-run')) return { stdout: JSON.stringify({ directory: { files: page, skipped: [], manifest: 'm2' } }) };
    return { stdout: JSON.stringify(args[1] === 'status' ? { paths: { workspace: '/w', lock: '/w.lock' }, roots: { config: { path: '/c' } } } : {}) };
  };
  await adapter.cli(current, 'index', { action: 'add', file: 'lib', workspaceId: 'ws_1', cursor: 'd1.c' });
  assert.deepEqual(asked[1], ['read', ...page]);
  assert.deepEqual(ran, ['index add lib --id ws_1 --cursor d1.c --dry-run', 'index status --id ws_1', 'index add lib --id ws_1 --cursor d1.c --manifest m2']);
});

test('CLI-path saved queries ask read on the sources they search, so a denied read runs no subscribe or evaluate', async t => {
  const { root, ctx, adapter } = await context(t);
  const asked = [], ran = [];
  ctx.permission.assert = async ({ action, resources }) => {
    asked.push([action, ...resources]);
    if (action === 'read') throw Object.assign(new Error('read denied'), { _tag: 'Permission.DeclinedError' });
  };
  const design = path.join(root, 'design.fig');
  await writeFile(design, '');
  const current = await adapter.scope('session');
  current.run = async args => {
    ran.push(args[1]);
    return { stdout: JSON.stringify({ state: 'ready', paths: { workspace: '/w', lock: '/w.lock' }, storage: { rule: 'default' },
      roots: { config: { path: '/c' } }, sources: [{ sourceId: 's', path: design }] }) };
  };
  await assert.rejects(adapter.cli(current, 'watch', { action: 'subscribe', workspaceId: 'ws_1', consumer: 'c', sourceId: 's', query: 'badge' }), /read denied/);
  await assert.rejects(adapter.cli(current, 'watch', { action: 'evaluate', workspaceId: 'ws_1', subscriptionId: 'sub_1' }), /read denied/);
  assert.deepEqual([asked.map(([action]) => action), ran], [['read', 'read'], ['status', 'status', 'list']]);
});

test('CLI-path subscribe over a listed set asks read on exactly those sources, and on every one for visible or an unknown id', async t => {
  const { root, ctx, adapter } = await context(t);
  const [a, b] = [path.join(root, 'a.fig'), path.join(root, 'b.fig')];
  await writeFile(a, '');
  await writeFile(b, '');
  const [realA, realB] = [await realpath(a), await realpath(b)];
  const asked = [];
  let ran = [];
  // Read of A is allowed, B denied.
  ctx.permission.assert = async ({ action, resources }) => {
    if (action !== 'read') return;
    asked.push(resources);
    if (resources.includes(b)) throw Object.assign(new Error('read denied'), { _tag: 'Permission.DeclinedError' });
  };
  const run = async args => {
    ran.push(args[1]);
    return { stdout: JSON.stringify({ state: 'ready', paths: { workspace: '/w', lock: '/w.lock' }, storage: { rule: 'default' },
      roots: { config: { path: '/c' } }, sources: [{ sourceId: 'A', path: a }, { sourceId: 'B', path: b }] }) };
  };
  // Each case in its own session, so no earlier grant hides what it asks.
  const subscribe = async more => adapter.cli(Object.assign(await adapter.scope(JSON.stringify(more)), { run }), 'watch',
    { action: 'subscribe', workspaceId: 'ws_1', consumer: 'c', ...more });
  await subscribe({ sources: 'A', query: 'badge' });
  await subscribe({ sources: ' A ', library: { kind: 'component', key: 'k' } });
  assert.deepEqual([asked, ran], [[reads(a, realA), reads(a, realA)], ['status', 'subscribe', 'status', 'subscribe']]);
  for (const sources of ['A,B', 'visible', 'A,unknown']) {
    ran = [];
    await assert.rejects(subscribe({ sources, query: 'badge' }), /read denied/);
    assert.deepEqual([asked.at(-1), ran], [reads(a, realA, b, realB), ['status']], sources);
  }
});

test('CLI-path subscription reads ask read on exactly the subscription\'s sources that watch list names', async t => {
  const { root, ctx, adapter } = await context(t);
  const [a, b] = [path.join(root, 'a.fig'), path.join(root, 'b.fig')];
  await writeFile(a, '');
  await writeFile(b, '');
  const [realA, realB] = [await realpath(a), await realpath(b)];
  const subscriptions = [{ subscriptionId: 'one', sourceId: 'A' }, { subscriptionId: 'set', sourceId: null, sources: ['A', 'B'] },
    { subscriptionId: 'other', sourceId: 'B' }, { subscriptionId: 'unreadable', sourceId: null, sources: null }];
  const asked = [];
  let ran = [];
  // Read of A is allowed, B denied.
  ctx.permission.assert = async ({ action, resources }) => {
    if (action !== 'read') return;
    asked.push(resources);
    if (resources.includes(b)) throw Object.assign(new Error('read denied'), { _tag: 'Permission.DeclinedError' });
  };
  // Each case in its own session, so no earlier grant hides what it asks.
  const session = async id => Object.assign(await adapter.scope(id), { run });
  const run = async args => {
    ran.push(args[0] === 'changes' ? 'changes' : args[1]);
    // The list reads only the named subscription.
    if (args[1] === 'list') {
      const id = args[args.indexOf('--subscription') + 1];
      return { stdout: JSON.stringify({ subscriptions: subscriptions.filter(s => s.subscriptionId === id), next: null }) };
    }
    return { stdout: JSON.stringify({ state: 'ready', paths: { workspace: '/w', lock: '/w.lock' }, storage: { rule: 'default' },
      roots: { config: { path: '/c' } }, sources: [{ sourceId: 'A', path: a }, { sourceId: 'B', path: b }] }) };
  };
  const evaluate = async more => adapter.cli(await session(JSON.stringify(more)), 'watch', { action: 'evaluate', workspaceId: 'ws_1', ...more });
  await evaluate({ subscriptionId: 'one' });
  await adapter.cli(await session('changes'), 'changes', { workspaceId: 'ws_1', subscriptionId: 'one' });
  assert.deepEqual([asked, ran], [[reads(a, realA), reads(a, realA)], ['status', 'list', 'evaluate', 'status', 'list', 'changes']]);
  // A set, a source the caller narrows away, an unreadable set and an unseen subscription all ask B: nothing runs.
  // evaluate refuses any sourceId outright, so the cases that pass one go through changes, where it is a real filter.
  const read = more => more.sourceId === undefined ? evaluate(more)
    : session(JSON.stringify(more)).then(s => adapter.cli(s, 'changes', { workspaceId: 'ws_1', ...more }));
  for (const more of [{ subscriptionId: 'set' }, { subscriptionId: 'set', sourceId: 'A' }, { subscriptionId: 'other', sourceId: 'A' },
    { subscriptionId: 'unreadable' }, { subscriptionId: 'unseen', sourceId: 'A' }]) {
    ran = [];
    await assert.rejects(read(more), /read denied/);
    // One indexed list read per call, never a page walk, and nothing after it.
    assert.deepEqual([asked.at(-1).includes(b), ran], [true, ['status', 'list']], JSON.stringify(more));
  }
  assert.deepEqual(asked.at(-1), reads(a, realA, b, realB), 'an unseen subscription asks every source');
  // search and usages ignore a subscriptionId, so it never narrows their ask: B is asked, and nothing runs.
  for (const tool of ['search', 'usages']) {
    ran = [];
    await assert.rejects(adapter.cli(await session(tool), tool, { workspaceId: 'ws_1', query: 'badge', node: '1:1', subscriptionId: 'one' }), /read denied/);
    assert.deepEqual([asked.at(-1), ran], [reads(a, realA, b, realB), ['status']], tool);
  }
});

test('CLI-path library questions also ask read on the definition\'s source, so a denied one runs no usages, subscribe, evaluate or changes', async t => {
  const { root, ctx, adapter } = await context(t);
  const [a, b] = [path.join(root, 'a.fig'), path.join(root, 'b.fig')];
  await writeFile(a, '');
  await writeFile(b, '');
  const [realA, realB] = [await realpath(a), await realpath(b)];
  const definition = { sourceId: 'B', snapshotId: 'snap', nodeId: '1:1' };
  const library = { kind: 'component', key: 'k', definition };
  const row = { sourceId: 'A', library: { kind: 'component', namespace: null, key: 'k' } };
  // A row whose definition source was removed names it null, so it cannot narrow the ask.
  const subscriptions = [{ ...row, subscriptionId: 'lib', definition: { sourceId: 'B', nodeId: '1:1', baselineSnapshotId: 'snap' } },
    { ...row, subscriptionId: 'hidden', definition: { sourceId: null, nodeId: null, baselineSnapshotId: null } }];
  const asked = [];
  let ran = [], denied = b;
  ctx.permission.assert = async ({ action, resources }) => {
    if (action !== 'read') return;
    asked.push(resources);
    if (resources.includes(denied)) throw Object.assign(new Error('read denied'), { _tag: 'Permission.DeclinedError' });
  };
  const run = async args => {
    ran.push(['changes', 'usages'].includes(args[0]) ? args[0] : args[1]);
    if (args[1] === 'list') {
      const id = args[args.indexOf('--subscription') + 1];
      return { stdout: JSON.stringify({ subscriptions: subscriptions.filter(s => s.subscriptionId === id), next: null }) };
    }
    return { stdout: JSON.stringify({ state: 'ready', paths: { workspace: '/w', lock: '/w.lock' }, storage: { rule: 'default' },
      roots: { config: { path: '/c' } }, sources: [{ sourceId: 'A', path: a }, { sourceId: 'B', path: b }] }) };
  };
  // Each case in its own session, so no earlier grant hides what it asks.
  const call = async (tool, input) => adapter.cli(Object.assign(await adapter.scope(JSON.stringify([tool, input, denied])), { run }), tool, input);
  const usages = { workspaceId: 'ws_1', sourceId: 'A', library };
  for (const [tool, input] of [['usages', usages], ['usages', { ...usages, library: { ...library, definition: { ...definition, sourceId: 'Z' } } }],
    ['watch', { action: 'subscribe', workspaceId: 'ws_1', consumer: 'c', sourceId: 'A', library }],
    ['watch', { action: 'subscribe', workspaceId: 'ws_1', consumer: 'c', sources: 'A', library }],
    ['watch', { action: 'evaluate', workspaceId: 'ws_1', subscriptionId: 'lib' }], ['changes', { workspaceId: 'ws_1', subscriptionId: 'lib' }],
    ['changes', { workspaceId: 'ws_1', subscriptionId: 'hidden' }]]) {
    ran = [];
    await assert.rejects(call(tool, input), /read denied/);
    // Only the read-only status and list ran.
    assert.deepEqual([asked.at(-1), ran.filter(step => !['status', 'list'].includes(step))], [reads(a, realA, b, realB), []], JSON.stringify(input));
  }
  [denied, ran] = [undefined, []];
  await call('usages', usages);
  assert.deepEqual([asked.at(-1), ran], [reads(a, realA, b, realB), ['status', 'usages']]);
});

test('CLI-path evaluate refuses any sourceId before asking or running, and otherwise asks read on every source of an unlisted subscription', async t => {
  const { root, ctx, adapter } = await context(t);
  const designs = ['one.fig', 'two.fig'].map(name => path.join(root, name));
  for (const design of designs) await writeFile(design, '');
  let denied, asked = [], ran = [];
  ctx.permission.assert = async ({ action, resources }) => {
    asked.push([action, ...resources]);
    if (action === 'read' && resources.includes(denied)) throw Object.assign(new Error('read denied'), { _tag: 'Permission.DeclinedError' });
  };
  const current = await adapter.scope('session');
  current.run = async args => {
    ran.push(args[1]);
    return { stdout: JSON.stringify({ state: 'ready', paths: { workspace: '/w', lock: '/w.lock' }, storage: { rule: 'default' },
      roots: { config: { path: '/c' } }, sources: [{ sourceId: 's1', path: designs[0] }, { sourceId: 's2', path: designs[1] }] }) };
  };
  const evaluate = sourceId => adapter.cli(current, 'watch', { action: 'evaluate', workspaceId: 'ws_1', subscriptionId: 'sub_denied', sourceId });
  // A fake, a matching and another valid source are all refused: none of them chooses what the binary reads.
  for (const sourceId of ['nonexistent', 's1', 's2']) await assert.rejects(evaluate(sourceId), /takes no sourceId/);
  assert.deepEqual([asked, ran], [[], []]);
  denied = designs[1];
  // This fake lists no subscription, so the lookup finds nothing and every source is asked.
  await assert.rejects(evaluate(undefined), /read denied/);
  assert.deepEqual(ran, ['status', 'list'], 'a denied source ran no evaluate');
  [denied, asked, ran] = [undefined, [], []];
  await evaluate(undefined);
  const read = asked.find(([action]) => action === 'read') ?? [];
  assert.ok(designs.every(design => read.includes(design)), JSON.stringify(read));
  assert.deepEqual(ran, ['status', 'list', 'evaluate']);
});

test('CLI-path prepare with an explicit storage asks edit on the state root the binary chose, not the remembered one', async t => {
  const { ctx, adapter } = await context(t);
  const asked = [], ran = [];
  // A rule that allows the remembered directory and denies the new one.
  ctx.permission.assert = async ({ action, resources }) => {
    if (action !== 'read') asked.push([action, ...resources]);
    if (resources.includes('/new/state')) throw Object.assign(new Error('denied /new/state'), { _tag: 'Permission.DeclinedError' });
  };
  const current = await adapter.scope('session');
  current.run = async args => {
    ran.push(args[1]);
    return { stdout: JSON.stringify({ paths: null, storage: { rule: '--storage', remembered: '/old' },
      roots: { state: { path: '/new/state' }, config: { path: '/config' } } }) };
  };
  await assert.rejects(adapter.cli(current, 'index', { action: 'prepare', file: '/d.fig', storage: '/new' }), /denied/);
  assert.deepEqual(asked, [['edit', '/new/state', '/config']]);
  assert.deepEqual(ran, ['status'], 'nothing was written');
});

test('CLI-path workspace search of a missing workspace returns the binary\'s own error, not an update request', async t => {
  const { ctx, adapter } = await context(t);
  const asked = [];
  ctx.permission.assert = async ({ action, resources }) => { asked.push([action, ...resources]); };
  const current = await adapter.scope('session');
  current.run = async args => {
    if (args[1] === 'status') return { stdout: JSON.stringify({ state: 'absent', paths: null, roots: { config: { path: '/c' } } }) };
    throw Object.assign(new Error('exit 1'), { stderr: 'figlens: workspace ws_0 is not prepared; run `figlens index prepare <file.fig>` first' });
  };
  await assert.rejects(adapter.cli(current, 'search', { workspaceId: 'ws_0', query: 'a' }), /ws_0 is not prepared/);
  assert.deepEqual(asked, []);
});

test('CLI-path calls refuse a gs:// design before any permission or command, since its cache read is unauthorizable', async t => {
  const { ctx, adapter } = await context(t);
  const asked = [], ran = [];
  ctx.permission.assert = async ({ action, resources }) => { asked.push([action, ...resources]); };
  const current = await adapter.scope('session');
  current.run = async args => { ran.push(args[0]); return { stdout: '{}' }; };
  for (const [tool, input] of [['search', { file: 'gs://b/d.fig', query: 'a' }], ['inspect', { file: 'gs://b/d.fig', node: '0:1' }],
    ['index', { action: 'prepare', file: 'gs://b/d.fig', scope: '0:0' }], ['usages', { file: 'gs://b/d.fig', node: '0:1' }],
    ['diff', { old: 'gs://b/a.fig', file: '/b.fig' }]])
    await assert.rejects(adapter.cli(current, tool, input), /local \.fig files only.*figlens MCP server/);
  // Conflicting selectors fail the same way, before the read check of the file they name.
  await assert.rejects(adapter.cli(current, 'usages', { file: '/d.fig', workspaceId: 'ws_1', node: '0:1' }), /not both/);
  assert.deepEqual([asked, ran], [[], []]);
});

test('CLI-path workspace search, usages, changes, rebuild and track ask read on the selected registered designs before reading them', async t => {
  const { ctx, adapter } = await context(t);
  const asked = [], ran = [];
  ctx.permission.assert = async ({ action, resources }) => {
    asked.push([action, ...resources]);
    if (action === 'read') throw Object.assign(new Error('read denied'), { _tag: 'Permission.DeclinedError' });
  };
  const current = await adapter.scope('new session');
  current.run = async args => {
    ran.push(args[1]);
    return { stdout: JSON.stringify({ paths: { workspace: '/w', lock: '/w.lock' }, storage: { rule: 'default' }, roots: { config: { path: '/c' } },
      sources: [{ sourceId: 's1', path: '/a.fig' }, { sourceId: 's2', path: '/b.fig' }] }) };
  };
  await assert.rejects(adapter.cli(current, 'search', { workspaceId: 'ws_1', sourceId: 's1', node: '0:1', query: 'a' }), /read denied/);
  await assert.rejects(adapter.cli(current, 'index', { action: 'rebuild', workspaceId: 'ws_1' }), /read denied/);
  await assert.rejects(adapter.cli(current, 'usages', { workspaceId: 'ws_1', sourceId: 's2', node: '0:1' }), /read denied/);
  await assert.rejects(adapter.cli(current, 'usages', { file: '/c.fig', node: '0:1' }), /read denied/);
  await assert.rejects(adapter.cli(current, 'changes', { workspaceId: 'ws_1', sourceId: 's2' }), /read denied/);
  await assert.rejects(adapter.cli(current, 'index', { action: 'track', workspaceId: 'ws_1', scope: '0:1' }), /read denied/);
  assert.deepEqual(asked, [['read', '/a.fig'], ['read', '/a.fig', '/b.fig'], ['read', '/b.fig'], ['read', '/c.fig'], ['read', '/b.fig'], ['read', '/a.fig', '/b.fig']]);
  assert.deepEqual(ran, ['status', 'status', 'status', 'status', 'status'], 'only the read-only status ran');
});

test('CLI-path diff asks read on each design on its own and runs only after both are allowed', async t => {
  const { ctx, adapter } = await context(t);
  const asked = [], ran = [];
  let deny = '/a.fig';
  ctx.permission.assert = async ({ action, resources }) => {
    asked.push([action, ...resources]);
    if (resources.includes(deny)) throw Object.assign(new Error('read denied'), { _tag: 'Permission.DeclinedError' });
  };
  const current = await adapter.scope('session');
  current.run = async args => { ran.push(args); return { stdout: '{"frames":[]}' }; };
  // The earlier design is denied: the later one is never asked and nothing runs.
  await assert.rejects(adapter.cli(current, 'diff', { old: '/a.fig', file: '/b.fig' }), /read denied/);
  assert.deepEqual([asked, ran], [[['read', '/a.fig']], []]);
  deny = '/b.fig';
  await assert.rejects(adapter.cli(current, 'diff', { old: '/a.fig', file: '/b.fig' }), /read denied/);
  assert.deepEqual(ran, [], 'nothing ran while the later design is denied');
  deny = null;
  assert.deepEqual(await adapter.cli(current, 'diff', { old: '/a.fig', file: '/b.fig', node: '0:1' }), { frames: [] });
  assert.deepEqual(ran, [['diff', '/a.fig', '/b.fig', '--node', '0:1', '--json']]);
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
    return { stdout: JSON.stringify({ paths: { workspace: '/state/workspaces/ws_a', lock: '/state/workspaces/ws_a.lock' }, storage: { rule: 'default' }, roots: { config: { path: '/config' } } }) };
  };
  await assert.rejects(adapter.cli(current, 'index', { action: 'reset', workspaceId: 'ws_a', file: '/b.fig' }), /usage/);
  assert.deepEqual(asked, [], 'conflicting selectors fail before any permission');
  await assert.rejects(adapter.cli(current, 'index', { action: 'reset', workspaceId: 'ws_a', sourceId: 's' }), /usage/);
  await assert.rejects(adapter.cli(current, 'index', { action: 'prepare', file: '/b.fig', workspaceId: 'ws_a', storage: '/new', replaceUnavailable: true }), /usage/);
  assert.deepEqual(ran.filter(command => !command.startsWith('index status')), ['index reset --id ws_a --source s']);
});

test('CLI-path track runs until its baseline job finishes, and untrack asks only edit', async t => {
  const { ctx, adapter } = await context(t);
  const asked = [], timeouts = [];
  ctx.permission.assert = async ({ action, resources }) => { asked.push([action, ...resources]); };
  const current = await adapter.scope('session');
  current.run = async (args, timeout) => {
    timeouts.push([args.slice(0, 2).join(' '), timeout]);
    return { stdout: JSON.stringify({ paths: { workspace: '/w', lock: '/w.lock' }, roots: { config: { path: '/c' } }, sources: [{ sourceId: 's', path: '/a.fig' }] }) };
  };
  await adapter.cli(current, 'index', { action: 'track', workspaceId: 'ws_1', scope: '0:1' });
  await adapter.cli(current, 'index', { action: 'untrack', workspaceId: 'ws_1', scope: '0:1' });
  assert.deepEqual(timeouts, [['index status', undefined], ['index track', 0], ['index status', undefined], ['index untrack', undefined]]);
  assert.deepEqual(asked, [['read', '/a.fig'], ['edit', '/w', '/w.lock', '/c']]);
});

// A bundled package whose native CLI answers status and holds `watch run` leases as the real one does.
async function watching(t) {
  const { root, ctx } = await context(t);
  const override = process.env.FIGLENS_BIN;
  delete process.env.FIGLENS_BIN;
  t.after(() => {
    if (override === undefined) delete process.env.FIGLENS_BIN; else process.env.FIGLENS_BIN = override;
  });
  await mkdir(path.join(root, 'package/bin'), { recursive: true });
  await writeFile(path.join(root, 'package/bin/figlens.js'), `require('node:child_process').spawnSync(process.execPath, [require('node:path').join(__dirname, 'native.cjs'), ...process.argv.slice(2)], { stdio: 'inherit' });`);
  const native = `const fs = require('node:fs');
const say = value => console.log(JSON.stringify(value));
const [command, action] = process.argv.slice(2);
if (command === 'index' && action === 'status') say({ paths: { workspace: '/w', lock: '/w.lock' }, roots: { config: { path: '/c' } }, sources: [{ sourceId: 's1', path: '/a.fig' }, { sourceId: 's2', path: '/b.fig' }] });
else if (command === 'watch' && action === 'status') say({ leases: [], runner: { state: 'stopped' } });
else if (command === 'watch' && action === 'run') {
  fs.appendFileSync('runs', process.argv.slice(4).join(' ') + '\\n');
  const request = process.argv.includes('--request-id') ? 'ended-' + process.argv[process.argv.indexOf('--request-id') + 1] : null;
  if (request && fs.existsSync(request)) { say({ leaseId: fs.readFileSync(request, 'utf8'), state: 'ended', reason: 'released' }); process.exit(1); }
  const leaseId = 'l' + fs.readFileSync('runs', 'utf8').split('\\n').length;
  say({ leaseId, state: 'active' });
  if (process.argv.includes('stubborn')) { fs.writeFileSync('stubborn', String(process.pid)); setInterval(() => {}, 1000); return; }
  process.stdin.on('end', () => { fs.appendFileSync('released', leaseId + '\\n'); if (request) fs.writeFileSync(request, leaseId); say({ leaseId, state: 'ended', reason: 'released' }); process.exit(0); });
  process.stdin.resume();
} else throw new Error('Unexpected command');`;
  await writeFile(path.join(root, 'package/bin/native.cjs'), native);
  // The launcher's cached binary, which a lease holder runs directly.
  await copyFile(new URL('../npm/target.js', import.meta.url), path.join(root, 'package/target.js'));
  await writeFile(path.join(root, 'package/package.json'), JSON.stringify({ version: '0.0.0-test' }));
  const cache = path.join(root, 'cache');
  const cached = path.join(cache, 'figlens/npm/0.0.0-test', process.platform === 'win32' ? 'figlens.exe' : 'figlens');
  await mkdir(path.dirname(cached), { recursive: true });
  // A copy of node stands in for it on every OS: run as `watch run ...` in the holder's cwd, it runs this script.
  try { await link(process.execPath, cached); } catch { await copyFile(process.execPath, cached); }
  await writeFile(path.join(root, 'watch'), `process.argv.splice(2, 0, 'watch');\n${native}`);
  const saved = process.env.XDG_CACHE_HOME;
  process.env.XDG_CACHE_HOME = cache;
  t.after(() => { if (saved === undefined) delete process.env.XDG_CACHE_HOME; else process.env.XDG_CACHE_HOME = saved; });
  const adapter = createAdapter(ctx, pathToFileURL(path.join(root, 'package/opencode/index.mjs')).href);
  t.after(() => adapter.dispose());
  const lines = async name => (await readFile(path.join(root, name), 'utf8').catch(() => '')).split('\n').filter(Boolean);
  return { ctx, adapter, lines };
}

test('CLI-path watch start holds a lease for its session after asking read on every design and edit on the workspace', async t => {
  const { ctx, adapter, lines } = await watching(t);
  const asked = [];
  ctx.permission.assert = async ({ action, resources }) => { asked.push([action, action === 'read' ? resources.join(' ') : resources[0]]); };
  const a = await adapter.scope('a');
  const start = { action: 'start', workspaceId: 'ws_1', sourceId: 's1', scope: '0:0', storage: '/s', requestId: 'r' };
  const lease = await adapter.cli(a, 'watch', start);
  // Besides the bundled launcher's own cache edit.
  const asks = asked.filter(([action, resource]) => action !== 'bash' && !resource.endsWith('figlens')).map(([action, resource]) => `${action} ${resource}`);
  // A fresh session, so no earlier grant hides an edit ask.
  const before = asked.length;
  await adapter.cli(await adapter.scope('b'), 'watch', { action: 'status', workspaceId: 'ws_1' });
  assert.deepEqual([lease.state, await lines('runs'), asks, asked.slice(before).filter(([action, resource]) => action === 'edit' && resource === '/w')],
    ['active', ['--id ws_1 --source s1 --scope 0:0 --storage /s --request-id r'], ['read /a.fig /b.fig', 'edit /w'], []],
    'status asks no edit on the workspace');
});

test('CLI-path watch stop ends only a lease this session started, and closing a session releases its own', async t => {
  const { adapter, lines } = await watching(t);
  const [a, b] = [await adapter.scope('a'), await adapter.scope('b')];
  const start = current => adapter.cli(current, 'watch', { action: 'start', workspaceId: 'ws_1', scope: '0:0' });
  const first = await start(a);
  await start(b);
  await assert.rejects(adapter.cli(b, 'watch', { action: 'stop', leaseId: first.leaseId }), /not a lease this session holds/);
  const end = await adapter.cli(a, 'watch', { action: 'stop', leaseId: first.leaseId });
  await start(a);
  await adapter.close('a');
  assert.deepEqual([end, await lines('released')], [{ leaseId: 'l2', state: 'ended', reason: 'released' }, ['l2', 'l4']],
    "b's lease l3 outlives a's close");
});

test('CLI-path watch start replays a requestId within its session, and only there, after asking again', async t => {
  const { ctx, adapter, lines } = await watching(t);
  const [a, b] = [await adapter.scope('a'), await adapter.scope('b')];
  const start = (current, scope = '0:0') => adapter.cli(current, 'watch', { action: 'start', workspaceId: 'ws_1', scope, requestId: 'r' });
  const [first, concurrent] = await Promise.all([start(a), start(a)]);
  const repeat = await start(a);
  await assert.rejects(start(a, '0:1'), /requestId r names a different watch/);
  const foreign = await start(b);
  const spawned = (await lines('runs')).length;
  await adapter.cli(a, 'watch', { action: 'stop', leaseId: first.leaseId });
  const ended = await start(a);
  ctx.permission.assert = async ({ action }) => {
    if (action === 'read') throw Object.assign(new Error('read denied'), { _tag: 'Permission.DeclinedError' });
  };
  await assert.rejects(start(await adapter.scope('b')), /read denied/, "a replay's call asks read again, so a revoked grant fails closed");
  assert.deepEqual([concurrent, repeat, foreign.leaseId !== first.leaseId, spawned, ended, (await lines('runs')).length],
    [first, first, true, 2, { leaseId: first.leaseId, state: 'ended', reason: 'released' }, 3]);
});

test('CLI-path watch holds the cached native binary itself, so stop ends it even when it ignores its stdin', async t => {
  const { adapter, lines } = await watching(t);
  const a = await adapter.scope('a');
  const lease = await adapter.cli(a, 'watch', { action: 'start', workspaceId: 'ws_1', scope: '0:0', requestId: 'stubborn' });
  const [pid] = await lines('stubborn');
  const end = await adapter.cli(a, 'watch', { action: 'stop', leaseId: lease.leaseId });
  const alive = () => { try { process.kill(Number(pid), 0); return true; } catch { return false; } };
  assert.deepEqual([end.state, end.reason, alive(), (await lines('runs')).length], ['unknown', 'release_unconfirmed', false, 1]);
});

test('CLI-path watch start spawns nothing when read on the design is denied', async t => {
  const { ctx, adapter, lines } = await watching(t);
  ctx.permission.assert = async ({ action }) => {
    if (action === 'read') throw Object.assign(new Error('read denied'), { _tag: 'Permission.DeclinedError' });
  };
  await assert.rejects(adapter.cli(await adapter.scope('a'), 'watch', { action: 'start', workspaceId: 'ws_1', scope: '0:0' }), /read denied/);
  assert.deepEqual(await lines('runs'), []);
});

test('CLI-path watch start on an allowed source still asks every source, since its runner evaluates saved sets over others', async t => {
  const { ctx, adapter, lines } = await watching(t);
  // s1 (/a.fig) is allowed, s2 (/b.fig) denied: a saved set over both would be evaluated in the background.
  ctx.permission.assert = async ({ action, resources }) => {
    if (action === 'read' && resources.includes('/b.fig')) throw Object.assign(new Error('read denied'), { _tag: 'Permission.DeclinedError' });
  };
  await assert.rejects(adapter.cli(await adapter.scope('a'), 'watch', { action: 'start', workspaceId: 'ws_1', sourceId: 's1', scope: '0:0' }), /read denied/);
  assert.deepEqual(await lines('runs'), []);
});

test('every shipped OpenCode module parses, including the entry tests cannot import without the host', async () => {
  const { execFileSync } = await import('node:child_process');
  const dir = new URL('.', import.meta.url);
  for (const name of (await readdir(dir)).filter(name => name.endsWith('.mjs') && !name.endsWith('.test.mjs')))
    execFileSync(process.execPath, ['--check', new URL(name, dir).pathname]);
});
