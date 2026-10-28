import assert from 'node:assert/strict';
import test from 'node:test';
import { reviewMappings } from './review-ui.mjs';

const entry = { id: 'candidate', source: { name: 'Button', file: 'design.fig', frame: '0:1', instance: '0:2' },
  target: { path: 'src/Button.js', export: 'Button', import: 'import { Button } from "./Button.js";', example: '<Button />' },
  rationale: 'Existing project component', checks: ['Source inspected'], issues: ['Text differs'] };

function ui({ action = 'Accept', prompts = ['Intentional text'], confirm = true } = {}) {
  const writes = [], shown = [];
  const ctx = { location: { directory: '/project' }, client: { rpc: () => ({
    pending: async () => ({ candidates: [structuredClone(entry)] }),
    review: async (input, options) => { writes.push({ input, options }); },
  }) }, ui: { dialog: {
    alert: async options => { shown.push(options); }, select: async () => action,
    prompt: async () => prompts.shift(), confirm: async options => { shown.push(options); return confirm; },
  } } };
  return { ctx, writes, shown };
}

test('review shows all evidence and confirms edited fields with resolution', async () => {
  const { ctx, writes, shown } = ui({ action: 'Edit', prompts: ['src/New.js', 'New', 'import New from "./New.js";', '<New />', 'Use the requested label'] });
  await reviewMappings(ctx, 'session');
  for (const text of ['0:1', '0:2', 'src/Button.js', 'Source inspected', 'Text differs', 'Existing project component']) assert.ok(shown[0].message.includes(text));
  assert.ok(shown[1].message.includes('src/New.js'));
  assert.ok(shown[1].message.includes('Use the requested label'));
  assert.deepEqual(writes, [{ input: { sessionID: 'session', candidateId: 'candidate', action: 'approve',
    target: { path: 'src/New.js', export: 'New', import: 'import New from "./New.js";', example: '<New />' }, resolution: 'Use the requested label' },
  options: { location: { directory: '/project' } } }]);
});

test('cancelled edit, missing conflict resolution, Stop and declined confirmation never write', async () => {
  for (const options of [{ action: 'Edit', prompts: ['src/New.js', undefined] }, { prompts: [undefined] }, { action: 'Stop' }, { action: undefined }, { confirm: false }]) {
    const { ctx, writes } = ui(options);
    if (Object.hasOwn(options, 'action') && options.action === undefined) ctx.ui.dialog.select = async () => undefined;
    await reviewMappings(ctx, 'session');
    assert.deepEqual(writes, []);
  }
});

test('Skip persists through the same session-scoped RPC and does not approve', async () => {
  const { ctx, writes, shown } = ui({ action: 'Skip' });
  await reviewMappings(ctx, 'session');
  assert.equal(shown.length, 1);
  assert.deepEqual(writes[0].input, { sessionID: 'session', candidateId: 'candidate', action: 'skip' });
});

test('lost approve or skip replies reload stored state without resubmitting', async () => {
  for (const action of ['Accept', 'Skip']) {
    const { ctx, shown } = ui({ action });
    let pendingCalls = 0, reviewCalls = 0;
    ctx.client.rpc = () => ({
      pending: async () => ({ candidates: ++pendingCalls === 1 ? [structuredClone(entry)] : [] }),
      review: async () => { reviewCalls++; throw new Error('Reply lost after commit'); },
    });
    await reviewMappings(ctx, 'session');
    assert.equal(reviewCalls, 1);
    assert.equal(pendingCalls, 2);
    assert.ok(shown.some(item => item.title === 'Review result unconfirmed'));
    assert.ok(shown.some(item => item.message.includes('no longer pending')));
  }
});

test('an unavailable reload keeps the review result unconfirmed and never retries', async () => {
  const { ctx, shown } = ui();
  let pendingCalls = 0, reviewCalls = 0;
  ctx.client.rpc = () => ({
    pending: async () => {
      if (++pendingCalls > 1) throw new Error('Disconnected');
      return { candidates: [structuredClone(entry)] };
    },
    review: async () => { reviewCalls++; throw new Error('Connection lost'); },
  });
  await reviewMappings(ctx, 'session');
  assert.equal(reviewCalls, 1);
  assert.ok(shown.some(item => item.title === 'Stored state unavailable'));
});
