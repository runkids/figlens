import assert from 'node:assert/strict';
import test from 'node:test';
import { buildRequest } from './requests.mjs';

test('plan and YOLO retain exact requests and separate implementation authority', async () => {
  const request = '  Only "hero" in ./design files/shop.fig\nReply in French.  \n';
  const plan = await buildRequest('plan', request);
  const yolo = await buildRequest('yolo', request);
  assert.ok(plan.endsWith(request));
  assert.ok(yolo.endsWith(request));
  assert.match(plan, /does not authorize application edits or implementation/);
  assert.match(yolo, /authorizes application edits within that scope/);
  assert.match(yolo, /temporary plan, scratch exports and processes created for this run/);
  assert.match(yolo, /after successful verification/);
  assert.match(yolo, /existing plans and notes/);
  assert.match(plan, /planning\.md/);
  assert.match(yolo, /yolo\.md/);
});

test('discovery and mapping do not authorize application edits or inject a selection', async () => {
  for (const kind of ['open', 'preview', 'map']) {
    const prompt = await buildRequest(kind, 'The current conversation');
    assert.match(prompt, /not authorize application edits/);
    assert.match(prompt, /not node IDs/);
  }
  await assert.rejects(buildRequest('unknown'), /Unknown figlens request/);
});
