// Real SDK and sample checks. Scripted input is not a live model or GUI acceptance run.
import assert from 'node:assert/strict';
import { mkdir, readFile, realpath, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const [sdk, binary, fig, output, entry = 'pi/index.ts'] = process.argv.slice(2);
if (!output) throw new Error('Usage: node scripts/check-pi-usability.mjs <pi-package-root> <binary> <mcp-apps.fig> <new-output-directory> [extension-entry]');
const root = path.resolve(output);
await mkdir(root); // Refuse to overwrite another run.
process.env.PI_CODING_AGENT_DIR = path.join(root, 'agent');
const extensionPath = path.resolve(entry);
const packaged = existsSync(path.resolve(path.dirname(extensionPath), '../bin/figlens.js'));
if (packaged) delete process.env.FIGLENS_BIN;
else process.env.FIGLENS_BIN = path.resolve(binary);
const { loadExtensions, createExtensionRuntime } = await import(pathToFileURL(path.join(sdk, 'dist/core/extensions/loader.js')));
const { loadSkills } = await import(pathToFileURL(path.join(sdk, 'dist/core/skills.js')));
const { initTheme } = await import(pathToFileURL(path.join(sdk, 'dist/modes/interactive/theme/theme.js')));
initTheme('dark');
const requireSdk = createRequire(path.join(await realpath(sdk), 'package.json'));
const tui = await import(pathToFileURL(requireSdk.resolve('@earendil-works/pi-tui')));
const runtime = createExtensionRuntime();
const messages = [];
const prompts = [];
runtime.sendMessage = message => messages.push(message);
runtime.sendUserMessage = prompt => prompts.push(prompt);
const loaded = await loadExtensions([extensionPath], root, undefined, runtime);
assert.deepEqual(loaded.errors, []);
const extension = loaded.extensions[0];
assert.equal(extension.handlers.has('before_agent_start'), false);
const notify = [];
const ctx = { cwd: root, mode: 'rpc', hasUI: true, waitForIdle: async () => {}, ui: {
  input: async () => 'List views', select: async (_title, choices) => choices.includes('Accept') ? 'Accept' : choices[0],
  confirm: async () => true, notify: (...args) => notify.push(args), setStatus: () => {},
} };
const command = args => extension.commands.get('figlens').handler(args, ctx);
const tool = (name, params) => extension.tools.get(name).definition.execute('usability', params, undefined, undefined, ctx);
await command('');
assert.match(messages.at(-1).content, /Ask Pi directly/);
assert.doesNotMatch(messages.at(-1).content, /^\{/);
await command('not-a-command');
assert.match(messages.at(-1).content, /Unknown figlens command/);
await command('open missing.fig --frame 1:1');
assert.match(messages.at(-1).content, /Check that the file exists/);
await command('status');
assert.match(messages.at(-1).content, /No manually saved selection/);
await command('map');
assert.match(prompts.at(-1), /Propose reusable component mappings/);
assert.match(prompts.at(-1), /not node IDs/);

// No open: current file/frame works, and proposals can be reviewed while active is null.
const file = path.resolve(fig);
const params = { file, frame: '467:24651' };
const initial = (await tool('figlens_bindings', params)).structuredContent;
const instance = initial.items.find(item => item.id === '467:24657');
assert.ok(instance);
await writeFile(path.join(root, 'app.html'), '<header class="app-header">App</header>');
await tool('figlens_propose_bindings', { ...params, proposals: [{ instance: instance.id,
  target: { path: 'app.html', export: 'none', import: 'Reuse the existing .app-header block; no import.', example: '<header class="app-header">{currentTitle}</header>' },
  rationale: 'Existing HTML header; scripted source evidence only.', checks: [],
}] });
await command('review');
assert.match(messages.at(-1).content, /0 proposal\(s\) remain/);
assert.equal((await tool('figlens_bindings', params)).structuredContent.items.find(item => item.id === instance.id).status, 'approved');

// A stale manual Button selection must not change an explicit read of another screen.
await command(`open "${file}" --frame 448:4752`);
const preview = messages.findLast(message => message.customType === 'figlens-preview');
const reference = preview.content.find(part => part.type === 'text').text;
assert.match(reference, /no component instances.*Pi can still/s);
const png = fileURLToPath(reference.match(/\((file:[^)]+)\)/)[1]);
assert.equal((await readFile(png)).subarray(1, 4).toString(), 'PNG');
const rendered = extension.messageRenderers.get('figlens-preview')(preview, { expanded: true, outputPad: 1 }, { fg: (_role, text) => text });
assert.equal(rendered.children.length, 2); // Markdown and the actual host Image component.
assert.ok(rendered.render(80).some(line => line.includes('Open reference image')));
const capabilities = tui.getCapabilities();
try {
  tui.setCapabilities({ ...capabilities, images: 'kitty' });
  rendered.invalidate();
  assert.ok(rendered.render(80).some(line => line.includes('\x1b_G')), 'host renderer did not emit Kitty image data');
  tui.setCapabilities({ ...capabilities, images: null });
  rendered.invalidate();
  assert.ok(rendered.render(80).some(line => line.includes('Open reference image')));
} finally { tui.setCapabilities(capabilities); }
const other = (await tool('figlens_bindings', params)).structuredContent;
assert.equal(other.frame, params.frame);
assert.equal(other.items.find(item => item.id === instance.id).status, 'approved');
assert.match(await readFile(path.join(root, '.figlens/component-mappings.md'), 'utf8'), /"frame": "448:4752"/);

// RPC never receives the whole long list; next/previous preserve reachable pages.
const select = ctx.ui.select;
ctx.ui.input = async () => 'a';
const pages = [];
ctx.ui.select = async (title, choices) => {
  assert.ok(choices.length <= 8);
  pages.push(title);
  return pages.length === 1 ? 'Next page' : pages.length === 2 ? 'Previous page' : undefined;
};
await command('frame');
assert.equal(pages.length, 3);
assert.match(pages[1], /7–12/);
assert.equal(pages[0], pages[2]);
ctx.ui.select = select;

// Actual TUI SelectList: bounded after resizing; keyboard and wheel move to off-screen rows.
ctx.mode = 'tui';
ctx.ui.input = async () => 'a';
ctx.ui.custom = async factory => {
  let selected;
  const terminal = { rows: 20 };
  const component = factory({ terminal, requestRender: () => {} }, { fg: (_role, text) => text }, tui.getKeybindings(), value => { selected = value; });
  assert.ok(component.render(60).length <= 15);
  component.handleInput('\x1b[A');
  assert.match(component.render(60).join('\n'), /\((\d+)\/\1\)/, 'last off-screen row is unreachable');
  for (let i = 0; i < 30; i++) component.handleInput('\x1b[B');
  const beforeWheel = component.render(60).join('\n');
  component.handleMouse({ type: 'wheel', button: 'none', x: 1, y: 3, wheelDelta: 1 });
  assert.notEqual(component.render(60).join('\n'), beforeWheel);
  terminal.rows = 14;
  component.invalidate();
  assert.ok(component.render(40).length <= 9);
  for (const character of 'List views') component.handleInput(character);
  assert.ok(component.render(60).some(line => line.includes('No matching')));
  component.handleInput('\x1b');
  assert.equal(selected, undefined);
  return selected;
};
await command('frame');

// Nested name search reaches the demo screen, and a persistent image is emitted.
ctx.mode = 'rpc';
ctx.ui.input = async () => 'List views';
await command('frame');
assert.match(messages.findLast(message => typeof message.content === 'string' && message.content.startsWith('Saved selection')).content, /467:24651/);
assert.equal(notify.some(([, level]) => level === 'error'), false);

// Unmatched descriptions and headless legacy navigation return to conversational discovery.
ctx.ui.input = async () => 'An unmatched description of the account panel';
await command('frame');
assert.match(prompts.at(-1), /An unmatched description of the account panel/);
ctx.hasUI = false;
await command('frame');
assert.match(prompts.at(-1), /not node IDs/);

const skillRoot = packaged ? path.resolve(path.dirname(extensionPath), '../skills') : path.resolve('skills');
const skills = loadSkills({ cwd: root, agentDir: path.join(root, 'agent'), skillPaths: [skillRoot], includeDefaults: false });
assert.deepEqual(skills.diagnostics, []);
assert.equal(skills.skills.length, 1);
assert.equal(skills.skills[0].name, 'figlens');
assert.match(await readFile(skills.skills[0].filePath, 'utf8'), /already registered figlens MCP tools/);
console.log(JSON.stringify({ checks: ['readable help and errors', 'explicit scope without open', 'review without active selection', 'old Button does not change explicit scope', 'persistent PNG and host Image renderer', 'bounded and resizable TUI picker with wheel', 'nested frame name search', 'host skill discovery'], sdk: JSON.parse(await readFile(path.join(sdk, 'package.json'))).version, output: root }, null, 2));
