import assert from 'node:assert/strict';
import test from 'node:test';
import register from './e2e-pilot-images.mjs';

const png = {
  type: 'image', mimeType: 'image/png',
  data: 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=',
};

function apply(event) {
  let handler;
  register({ on(_event, callback) { handler = callback; } });
  return { ...event, ...handler(event) };
}

function screenshot() {
  return {
    type: 'tool_result', toolCallId: 'call_image', toolName: 'figma_get_screenshot',
    input: { nodeId: '1:2' }, isError: false,
    content: [{ type: 'text', text: '[image content: image/png]' }],
    details: { mcpTool: 'get_screenshot', raw: { content: [png] } },
  };
}

test('forwards the raw PNG as a model-visible image without changing its bytes or text', () => {
  const event = screenshot();
  const result = apply(event);
  assert.deepEqual(result.content, [event.content[0], png]);
  assert.equal(result.details, event.details);
  assert.equal(result.isError, false);
  assert.equal(event.content.length, 1);
});

test('also forwards images returned with design-context text and preserves structured data', () => {
  const event = screenshot();
  event.toolName = 'figma_get_design_context';
  event.content = [{ type: 'text', text: 'Design context' }];
  event.structuredContent = { node: '1:2' };
  event.details.raw.content = [{ type: 'text', text: 'Design context' }, png, png];
  assert.deepEqual(apply(event).content, [event.content[0], png, png]);
  assert.equal(apply(event).structuredContent, event.structuredContent);
});

test('does not duplicate existing images or modify unrelated and text-only tool results', () => {
  for (const change of [
    { content: [png] },
    { toolName: 'read' },
    { details: { raw: { content: [{ type: 'text', text: 'No image' }] } } },
    { details: undefined },
  ]) {
    const event = { ...screenshot(), ...change };
    assert.deepEqual(apply(event), event);
  }
});

test('preserves tool errors rather than promoting them to success', () => {
  const event = { ...screenshot(), isError: true };
  assert.equal(apply(event).isError, true);
});
