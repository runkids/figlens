// Exercise the real Pi loader and command without calling a provider or CLI.
import assert from 'node:assert/strict';
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const [sdk, output, entry = 'pi/index.ts'] = process.argv.slice(2);
if (!output) throw new Error('Usage: node scripts/check-pi-planning.mjs <pi-package-root> <new-output-directory> [extension-entry]');
const root = path.resolve(output);
await mkdir(root); // Refuse to overwrite an earlier run.
process.env.PI_CODING_AGENT_DIR = path.join(root, 'agent');
const { loadExtensions, createExtensionRuntime } = await import(pathToFileURL(path.join(sdk, 'dist/core/extensions/loader.js')));
const { loadSkills } = await import(pathToFileURL(path.join(sdk, 'dist/core/skills.js')));
const runtime = createExtensionRuntime();
const prompts = [];
const messages = [];
runtime.sendUserMessage = prompt => prompts.push(prompt);
runtime.sendMessage = message => messages.push(message);
runtime.exec = async () => { throw new Error('Planning command must not invoke the CLI.'); };
const extensionPath = path.resolve(entry);
const loaded = await loadExtensions([extensionPath], root, undefined, runtime);
assert.deepEqual(loaded.errors, []);
const extension = loaded.extensions[0];
assert.equal(extension.commands.has('plan'), false, 'leave the global /plan command to the host');
assert.equal(extension.handlers.has('before_agent_start'), false, 'no automatic scope injection');
const skillPath = fileURLToPath(new URL('../skills/figlens/SKILL.md', pathToFileURL(extensionPath)));
const planningPath = path.join(path.dirname(skillPath), 'references/planning.md');
const planning = await readFile(planningPath, 'utf8');
const skills = loadSkills({ cwd: root, agentDir: path.join(root, 'agent'), skillPaths: [path.dirname(skillPath)], includeDefaults: false });
assert.deepEqual(skills.diagnostics, []);
assert.equal(skills.skills.length, 1);
assert.equal(path.resolve(skills.skills[0].filePath), skillPath);
const ctx = {
  cwd: root, mode: 'rpc', hasUI: false, waitForIdle: async () => {},
  ui: new Proxy({}, { get: () => { throw new Error('Planning must not require a dialog.'); } }),
};
const command = args => extension.commands.get('figlens').handler(args, ctx);

const request = 'Plan only the "hero image" in ./design files/shop.fig, node 12:8.\nWrite the plan in French; plan only, with all other pages out of scope.';
for (const mode of ['tui', 'rpc', 'print']) {
  ctx.mode = mode;
  const before = prompts.length;
  await command(`plan ${request}`);
  assert.equal(prompts.length, before + 1);
  assert.ok(prompts.at(-1).endsWith(request), 'preserve quoted paths, language and scope verbatim');
  assert.ok(prompts.at(-1).includes(planning), 'load the actual adjacent planning reference');
  assert.ok(prompts.at(-1).includes(JSON.stringify(planningPath)), 'preserve the source path for on-demand reference links');
  assert.ok(prompts.at(-1).includes(JSON.stringify(skillPath)), 'resolve the skill from the installed entry');
  assert.match(prompts.at(-1), /does not authorize application edits or implementation/);
}
assert.deepEqual(await readdir(root), [], 'command creates no project or mapping files');

for (const description of ['', 'VIP panel', '"./design files/shop.fig"', 'Homepage login section']) {
  const before = prompts.length;
  await command(`open ${description}`);
  assert.equal(prompts.length, before + 1, 'open starts a conversation without a file or node ID');
  assert.match(prompts.at(-1), /not node IDs/);
  assert.match(prompts.at(-1), /does not authorize application edits or implementation/);
  if (description && !description.startsWith('"')) assert.ok(prompts.at(-1).endsWith(description));
}
for (const description of ['', 'Homepage hero in ./design files/shop.fig']) {
  const before = prompts.length;
  await command(`preview ${description}`);
  assert.equal(prompts.length, before + 1);
  assert.match(prompts.at(-1), /Call figlens_preview/);
  assert.match(prompts.at(-1), /not node IDs/);
  assert.match(prompts.at(-1), /does not authorize application edits/);
  assert.match(prompts.at(-1), /actual clickable URL/);
  if (description) assert.ok(prompts.at(-1).endsWith(description));
}
assert.deepEqual(await readdir(root), [], 'discovery shortcut does not save a manual selection');

// Even malformed old mapping state must neither supply scope nor block this command.
await mkdir(path.join(root, '.figlens'));
const mapping = 'Old selection: unrelated.fig / frame 99:1. Deliberately not mapping JSON.\n';
await writeFile(path.join(root, '.figlens/component-mappings.md'), mapping);
await command('plan');
assert.match(prompts.at(-1), /ask for the source and scope/);
assert.equal(prompts.at(-1).includes('unrelated.fig'), false);
await command('open VIP panel');
assert.equal(prompts.at(-1).includes('unrelated.fig'), false);
await command('map');
assert.match(prompts.at(-1), /not node IDs/);
assert.equal(prompts.at(-1).includes('unrelated.fig'), false);
await command('preview');
assert.equal(prompts.at(-1).includes('unrelated.fig'), false);
assert.equal(await readFile(path.join(root, '.figlens/component-mappings.md'), 'utf8'), mapping);

let resume;
ctx.waitForIdle = () => new Promise(resolve => { resume = resolve; });
const before = prompts.length;
const pending = command('plan Only the card empty state');
assert.equal(prompts.length, before, 'wait for idle before dispatching');
resume();
await pending;
assert.equal(prompts.length, before + 1);
assert.deepEqual(messages, [], 'no hidden command error was swallowed');
const version = JSON.parse(await readFile(path.join(sdk, 'package.json'), 'utf8')).version;
await writeFile(path.join(root, 'results.json'), JSON.stringify({ sdk: version, entry: extensionPath, checks: ['actual Pi loader and skill discovery', 'adjacent planning reference', 'TUI/RPC/print plan-only dispatch', 'verbatim local scope', 'no filesystem writes or manual selection', 'idle ordering', 'conversational open without IDs or saved state'], modelCalled: false, prompt: prompts[0] }, null, 2));
console.log(JSON.stringify({ sdk: version, entry: extensionPath, checks: 7, modelCalled: false, output: root }, null, 2));
