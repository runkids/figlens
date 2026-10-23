// Runs the extension through an installed Pi loader with Aurora and copied Skillshare components.
// Scripted dialogs exercise review behavior; no model calls, app edits or human approvals are claimed.
import assert from 'node:assert/strict';
import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const [piRoot, binary, fig, project, output] = process.argv.slice(2);
if (!output) throw new Error('Usage: node scripts/check-pi-integration.mjs <pi-package-root> <figlens-binary> <aurora.fig> <skillshare-checkout> <new-output-directory>');
const root = path.resolve(output);
await mkdir(root); // Refuse to overwrite an earlier run.
await mkdir(path.join(root, 'ui/src/components'), { recursive: true });
for (const name of ['Card.tsx', 'Button.tsx', 'SegmentedControl.tsx']) {
  await copyFile(path.join(project, 'ui/src/components', name), path.join(root, 'ui/src/components', name));
}
process.env.FIGLENS_BIN = path.resolve(binary);
const { loadExtensions, createExtensionRuntime } = await import(pathToFileURL(path.join(piRoot, 'dist/core/extensions/loader.js')).href);
const messages = [];
const prompts = [];
const runtime = createExtensionRuntime();
runtime.sendMessage = message => messages.push(message);
runtime.sendUserMessage = prompt => prompts.push(prompt);
const extensionPath = path.resolve('pi/index.ts');
async function load() {
  const loaded = await loadExtensions([extensionPath], root, undefined, runtime);
  assert.deepEqual(loaded.errors, []);
  return loaded.extensions[0];
}
let extension = await load();
const notifications = [];
const ctx = {
  cwd: root, mode: 'rpc', hasUI: true, signal: undefined,
  waitForIdle: async () => {},
  ui: {
    select: async (_title, options) => options.includes('Accept') ? 'Accept' : options[0],
    confirm: async () => true,
    input: async () => 'Use the visible descendant text; inspect the actual frame before implementation.',
    notify: (message, level) => notifications.push({ message, level }),
  },
};
const command = args => extension.commands.get('figlens').handler(args, ctx);
const tool = (name, params) => extension.tools.get(name).definition.execute('integration', params, undefined, undefined, ctx);
await command(`open "${path.resolve(fig)}" --frame 10:200`);
assert.deepEqual(notifications, []);
assert.ok(messages.some(message => message.customType === 'figlens-preview' && message.content.some(part => part.type === 'image')));
const initial = (await tool('figlens_bindings', {})).structuredContent;
assert.ok(initial.items.some(item => item.id === '10:201'));
const proposals = [
  {
    instance: '10:201',
    target: { path: 'ui/src/components/SegmentedControl.tsx', export: 'default', import: 'import SegmentedControl from "./components/SegmentedControl";', example: '<SegmentedControl value={selected} onChange={setSelected} options={optionsFromCurrentFrame} />' },
    rationale: 'Controlled value/onChange/options API; labels come from the current design.',
    checks: ['Copied implementation source inspected. No compilation or browser checks executed by this integration run.'],
  },
  {
    instance: 'I10:227;7:64',
    target: { path: 'ui/src/components/Card.tsx', export: 'default', import: 'import Card from "./components/Card";', example: '<Card padding="none">{contentFromCurrentFrame}</Card>' },
    rationale: 'Closed suggestion uses Card as a container with explicit child composition; no expansion behavior assumed.',
    checks: ['Copied implementation source inspected. No compilation or browser checks executed by this integration run.'],
  },
];
const submitted = (await tool('figlens_propose_bindings', { proposals })).structuredContent;
assert.equal(submitted.pending.length, 2);
assert.equal((await tool('figlens_bindings', {})).structuredContent.items.filter(item => item.status === 'approved').length, 0);
await command('map');
assert.equal(prompts.length, 1);
assert.match(prompts[0], /does not authorize application edits/);
await command('review');
assert.deepEqual(notifications, []);
const reviewed = (await tool('figlens_bindings', {})).structuredContent;
assert.ok(reviewed.items.filter(item => item.status === 'approved').length >= 2);
extension = await load();
await command(`open "${path.resolve(fig)}" --frame 22:103`);
const hero = (await tool('figlens_bindings', {})).structuredContent;
assert.equal(hero.items.find(item => item.id === 'I22:250;7:64').status, 'approved');
assert.equal(hero.items.find(item => item.id === 'I22:233;7:81').status, 'unmatched');
assert.ok(hero.items.find(item => item.id === 'I22:233;7:81').needsReview.length);
assert.ok(hero.items.find(item => item.id === 'I22:233;7:81;7:47').issues.some(issue => issue.includes('Not today')));
await tool('figlens_propose_bindings', { proposals: [{
  instance: 'I22:233;7:81;7:47',
  target: { path: 'ui/src/components/Button.tsx', export: 'default', import: 'import Button from "./components/Button";', example: '<Button variant="ghost" size="md">{reviewedLabel}</Button>' },
  rationale: 'Candidate ghost/md API; source text conflict requires explicit review.',
  checks: ['Source inspected only.'],
}] });
ctx.hasUI = false;
await command('review');
assert.equal((await tool('figlens_bindings', {})).structuredContent.items.find(item => item.id === 'I22:233;7:81;7:47').status, 'pending');
assert.equal(extension.handlers.has('before_agent_start'), false);
await writeFile(path.join(root, 'results.json'), JSON.stringify({ initial, submitted, reviewed, hero, prompt: prompts[0], notifications, checks: ['real Pi extension loader', 'real .fig frame discovery and PNG preview', 'pending tool proposals', 'scripted review dialogs', 'new-session reload', 'cross-frame reuse', 'unknown variant', 'source text conflict', 'headless review stays pending', 'no automatic old-scope injection'] }, null, 2));
const version = JSON.parse(await readFile(path.join(piRoot, 'package.json'), 'utf8')).version;
console.log(JSON.stringify({ pi: version, frame: hero.frame, reused: 'I22:250;7:64', unreviewed: 'I22:233;7:81', textConflict: 'I22:233;7:81;7:47', checks: 10, output: root }, null, 2));
