import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

async function run({ occupied = false, missingFont = false, brokenText = false } = {}) {
  const page = { name: 'Page 1', children: occupied ? [{ name: 'Existing design' }] : [] };
  const messages = [];
  const figma = {
    root: { children: [page] }, currentPage: page,
    viewport: { scrollAndZoomIntoView() {} },
    async loadFontAsync(font) {
      assert.deepEqual(JSON.parse(JSON.stringify(font)), { family: 'Source Code Pro', style: 'Regular' });
      if (missingFont) throw new Error('Font unavailable');
    },
    closePlugin(message) { messages.push(message); },
  };
  const create = type => {
    const node = {
      type, children: [], parent: null,
      resize(width, height) { this.width = width; this.height = height; },
      appendChild(child) {
        if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1);
        this.children.push(child); child.parent = this;
      },
      remove() { this.parent.children.splice(this.parent.children.indexOf(this), 1); },
    };
    page.children.push(node); node.parent = page;
    return node;
  };
  figma.createFrame = () => create('FRAME');
  figma.createText = () => {
    if (brokenText) throw new Error('Text creation failed');
    return create('TEXT');
  };
  figma.createEllipse = () => create('ELLIPSE');
  const code = await readFile(new URL('./e2e-fixture-plugin/code.js', import.meta.url), 'utf8');
  await vm.runInNewContext(code, { figma });
  return { page, messages };
}

test('refuses an occupied page without changing existing layers', async () => {
  const { page, messages } = await run({ occupied: true });
  assert.deepEqual(page.children, [{ name: 'Existing design' }]);
  assert.equal(page.name, 'Page 1');
  assert.match(messages.at(-1), /blank Figma Design file/);
});

test('missing font leaves the blank document untouched', async () => {
  const { page, messages } = await run({ missingFont: true });
  assert.equal(page.children.length, 0);
  assert.equal(page.name, 'Page 1');
  assert.match(messages.at(-1), /Font unavailable/);
});

test('creates one editable native-sized fixture with four list items', async () => {
  const { page } = await run();
  assert.equal(page.children.length, 1);
  const frame = page.children[0];
  assert.equal(frame.width, 960);
  assert.equal(frame.height, 640);
  assert.equal(frame.cornerRadius, 20);
  assert.equal(frame.strokeAlign, 'OUTSIDE');
  const texts = frame.children.filter(node => node.type === 'TEXT');
  assert.equal(texts.filter(node => node.name.startsWith('Task title /')).length, 4);
  assert.ok(texts.some(node => node.characters === 'Review design notes'));
  assert.ok(texts.every(node => node.fontName.family === 'Source Code Pro'));
});

test('a construction failure removes partial layers', async () => {
  const { page, messages } = await run({ brokenText: true });
  assert.equal(page.children.length, 0);
  assert.equal(page.name, 'Page 1');
  assert.match(messages.at(-1), /Text creation failed/);
});
