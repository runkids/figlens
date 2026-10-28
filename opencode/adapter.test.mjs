import assert from 'node:assert/strict';
import { mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { createAdapter } from './adapter.mjs';

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
