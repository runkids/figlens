import assert from 'node:assert/strict';
import test from 'node:test';
import { reviewMappings } from './review.mjs';

const entry = { id: 'candidate', source: { name: 'Button', file: 'design.fig', frame: '0:1', instance: '0:2' },
  target: { path: 'src/Button.js', export: 'Button', import: 'import { Button } from "./Button.js";', example: '<Button />' },
  rationale: 'Existing project component', checks: ['Source inspected'], issues: ['Text differs'] };

function ui({ action = 'Accept', prompts = ['Intentional text'], confirm = true, hasUI = true } = {}) {
  const writes = [], shown = [];
  const flow = { pending: async () => [structuredClone(entry)], review: async (...args) => { writes.push(args); } };
  const ctx = { hasUI, ui: {
    select: async () => action, input: async () => prompts.shift(),
    confirm: async (title, message) => { shown.push(message); return confirm; },
  } };
  return { flow, ctx, writes, shown, show: message => shown.push(message) };
}

test('OMP review shows evidence, edits all four fields and confirms the conflict resolution', async () => {
  const { flow, ctx, writes, shown, show } = ui({ action: 'Edit',
    prompts: ['src/New.js', 'New', 'import New from "./New.js";', '<New />', 'Use the requested label'] });
  const signal = new AbortController().signal;
  await reviewMappings(flow, ctx, show, signal);
  for (const text of ['0:1', '0:2', 'src/Button.js', 'Source inspected', 'Text differs', entry.rationale]) assert.ok(shown[0].includes(text));
  assert.ok(shown.some(text => text.includes('Use the requested label')));
  assert.deepEqual(writes, [['candidate', 'approve', {
    target: { path: 'src/New.js', export: 'New', import: 'import New from "./New.js";', example: '<New />' },
    resolution: 'Use the requested label',
  }, signal]]);
});

test('OMP noninteractive, Stop, cancelled edit, missing resolution and declined confirmation do not write', async () => {
  for (const options of [{ hasUI: false }, { action: 'Stop' }, { action: 'Edit', prompts: ['src/New.js', undefined] },
    { prompts: [undefined] }, { confirm: false }]) {
    const { flow, ctx, writes, show } = ui(options);
    await reviewMappings(flow, ctx, show);
    assert.deepEqual(writes, []);
  }
});

test('OMP Skip never approves and cancellation after confirmation prevents the writer', async () => {
  const skipped = ui({ action: 'Skip' });
  await reviewMappings(skipped.flow, skipped.ctx, skipped.show);
  assert.deepEqual(skipped.writes, [['candidate', 'skip', {}, undefined]]);
  const cancelled = ui();
  const controller = new AbortController();
  cancelled.ctx.ui.confirm = async () => { controller.abort(); return true; };
  await assert.rejects(reviewMappings(cancelled.flow, cancelled.ctx, cancelled.show, controller.signal), { name: 'AbortError' });
  assert.deepEqual(cancelled.writes, []);
});
