import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import test from 'node:test';
import { setTimeout } from 'node:timers/promises';
import { createAdapter, launcherRuntime } from './adapter.mjs';

async function context(t) {
  const root = await mkdtemp(path.join(tmpdir(), 'figlens-opencode-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const writes = [];
  const ctx = { location: { directory: root },
    session: { get: async () => ({ location: { directory: root } }), wait: async () => {}, prompt: async input => writes.push(input) },
    mcp: { list: async () => ({ data: [{ name: 'figlens', status: { status: 'connected' } }] }) },
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
    { type: 'local', command: ['other'], disabled: true }]) {
    adapter.configureMcp({ get: () => config, set: () => assert.fail('User configuration must remain unchanged') });
    await assert.rejects(adapter.commandHandler({ sessionID: 'session', prompt: { text: 'plan Only the hero' } }), /MCP is unavailable/);
  }
  assert.deepEqual(writes, []);
  assert.deepEqual(await readdir(root), []);
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
