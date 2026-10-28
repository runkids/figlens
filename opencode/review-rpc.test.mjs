import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { pathToFileURL } from 'node:url';
import { Workflow } from '../pi/workflow.mjs';
import { createAdapter } from './adapter.mjs';
import { registerReview } from './review-rpc.mjs';

test('review RPC observes the session/plugin lifetime while host permission is pending', async t => {
  const root = await mkdtemp(path.join(tmpdir(), 'figlens-review-rpc-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  let handlers, release, entered;
  const permissionEntered = new Promise(resolve => { entered = resolve; });
  const ctx = { location: { directory: root },
    session: { get: async () => ({ id: 'session', location: { directory: root } }) },
    permission: { assert: async () => { entered(); await new Promise(resolve => { release = resolve; }); } },
    rpc: { register: async (_definition, registered) => { handlers = registered; } },
  };
  const adapter = createAdapter(ctx);
  t.after(() => adapter.dispose());
  await registerReview(ctx, adapter);
  const pending = handlers.pending({ sessionID: 'session' }, {
    signal: new AbortController().signal, error: (code, message) => ({ code, message }),
  });
  await permissionEntered;
  await adapter.close('session');
  release();
  assert.equal((await pending).code, 'cancelled');
  assert.deepEqual(await readdir(root), []);
});

test('cold bundled review respects denied cache writes before launching the wrapper', async t => {
  const root = await mkdtemp(path.join(tmpdir(), 'figlens-review-cache-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const previous = { FIGLENS_BIN: process.env.FIGLENS_BIN, XDG_CACHE_HOME: process.env.XDG_CACHE_HOME };
  delete process.env.FIGLENS_BIN;
  process.env.XDG_CACHE_HOME = path.join(root, 'cache');
  t.after(() => {
    for (const [key, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[key]; else process.env[key] = value;
    }
  });
  const cache = path.join(root, 'cache/figlens'), marker = path.join(cache, 'wrapper-started');
  await mkdir(path.join(root, 'package/bin'), { recursive: true });
  const doc = { document: { id: '0:1', name: 'Frame', type: 'FRAME', children: [
    { id: '0:2', name: 'Button', type: 'INSTANCE', componentId: '0:3', children: [] },
  ] }, components: { '0:3': { componentSetId: null, key: null } } };
  await writeFile(path.join(root, 'package/bin/figlens.js'),
    `const fs = require('node:fs'); fs.mkdirSync(${JSON.stringify(cache)}, { recursive: true }); fs.writeFileSync(${JSON.stringify(marker)}, 'started'); process.stdout.write(${JSON.stringify(JSON.stringify(doc))});`);
  await writeFile(path.join(root, 'design.fig'), 'Synthetic mapping snapshot');
  await writeFile(path.join(root, 'Button.mjs'), 'export const Button = () => "button";');
  const fixture = new Workflow(root, async () => doc);
  const { pending } = await fixture.propose({ file: 'design.fig', frame: '0:1', proposals: [{ instance: '0:2',
    target: { path: 'Button.mjs', export: 'Button', import: 'import { Button } from "./Button.mjs";', example: 'Button()' }, rationale: 'Synthetic fixture',
  }] });
  const before = await readFile(fixture.guide);
  let handlers;
  const ctx = { location: { directory: root }, session: { get: async () => ({ id: 'session', location: { directory: root } }) },
    permission: { assert: async ({ action, resources }) => {
      if (action === 'edit' && resources.includes(cache)) throw Object.assign(new Error('Cache edit denied'), { _tag: 'Tool.Error' });
    } }, rpc: { register: async (_definition, registered) => { handlers = registered; } },
  };
  const adapter = createAdapter(ctx, pathToFileURL(path.join(root, 'package/opencode/index.mjs')).href);
  t.after(() => adapter.dispose());
  await registerReview(ctx, adapter);
  const output = await handlers.review({ sessionID: 'session', candidateId: pending[0].id, action: 'approve' }, {
    signal: new AbortController().signal, error: (code, message) => ({ code, message }),
  });
  assert.equal(output.code, 'permission_denied');
  await assert.rejects(readFile(marker), { code: 'ENOENT' });
  assert.deepEqual(await readFile(fixture.guide), before);
});
