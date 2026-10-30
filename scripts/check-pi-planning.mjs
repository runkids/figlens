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
runtime.exec = async () => { throw new Error('Dispatch commands must not invoke the CLI.'); };
const extensionPath = path.resolve(entry);
const loaded = await loadExtensions([extensionPath], root, undefined, runtime);
assert.deepEqual(loaded.errors, []);
const extension = loaded.extensions[0];
assert.equal(extension.commands.has('plan'), false, 'leave the global /plan command to the host');
assert.equal(extension.commands.has('yolo'), false, 'no global /yolo command');
// The one before-prompt handler is the stale-context check: with no watch target it adds no scope or context.
const run = async (event, payload) => {
  let out;
  for (const handler of extension.handlers.get(event) || []) out = await handler(payload, { cwd: root, mode: 'print' }) ?? out;
  return out;
};
assert.equal(await run('before_agent_start', { type: 'before_agent_start', prompt: 'p', messageType: 'user' }), undefined, 'no automatic scope injection');
assert.equal(await run('context', { type: 'context', messages: [] }), undefined, 'no context without a watch target');
const skillPath = fileURLToPath(new URL('../skills/figlens/SKILL.md', pathToFileURL(extensionPath)));
const planningPath = path.join(path.dirname(skillPath), 'references/planning.md');
const planning = await readFile(planningPath, 'utf8');
const yoloPath = path.join(path.dirname(skillPath), 'references/yolo.md');
const yolo = await readFile(yoloPath, 'utf8');
const skills = loadSkills({ cwd: root, agentDir: path.join(root, 'agent'), skillPaths: [path.dirname(skillPath)], includeDefaults: false });
assert.deepEqual(skills.diagnostics, []);
assert.equal(skills.skills.length, 1);
assert.equal(path.resolve(skills.skills[0].filePath), skillPath);
const ctx = {
  cwd: root, mode: 'rpc', hasUI: false, waitForIdle: async () => {},
  ui: new Proxy({}, { get: () => { throw new Error('Planning must not require a dialog.'); } }),
};
const command = args => extension.commands.get('figlens').handler(args, ctx);
const completions = extension.commands.get('figlens').getArgumentCompletions;
assert.deepEqual((await completions('yo')).map(item => item.value), ['yolo']);
assert.match((await completions('yo'))[0].description, /implement.*verify/i);
assert.ok((await completions('')).every(item => item.description), 'every subcommand completion has a description');
assert.deepEqual(await completions('yolo a request'), []);
await command('help');
assert.match(messages.pop().content, /`\/figlens yolo \[request\]`.*temporary plan/);

const request = 'Plan only the "hero image" in ./design files/shop.fig, node 12:8.\nWrite the plan in French; plan only, with all other pages out of scope.';
const yoloRequest = '  Implement only the "hero image" in ./design files/shop.fig, node 12:8.\nUse this project\'s components; report in French.  \n';
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
  assert.equal(prompts.at(-1).includes(yolo), false, 'plan-only never loads implementation authorization');
  await command(`yolo ${yoloRequest}`);
  assert.equal(prompts.length, before + 2);
  assert.ok(prompts.at(-1).endsWith(yoloRequest), 'preserve the exact implementation request, including whitespace');
  assert.ok(prompts.at(-1).includes(yolo), 'load the actual adjacent yolo reference');
  assert.ok(prompts.at(-1).includes(JSON.stringify(yoloPath)), 'resolve yolo links from the packaged reference');
  assert.ok(prompts.at(-1).includes(JSON.stringify(skillPath)));
  assert.match(prompts.at(-1), /same agent context/);
  assert.match(prompts.at(-1), /authorizes application edits within that scope/);
  assert.match(prompts.at(-1), /without an intermediate plan-approval question/);
  assert.match(prompts.at(-1), /report the missing prerequisite rather than guessing another file or widening/);
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
for (const args of ['yolo', 'yolo   \n']) {
  await command(args);
  assert.match(prompts.at(-1), /Implement the design task discussed in this conversation/);
  assert.match(prompts.at(-1), /report the missing prerequisite if its source or scope cannot be established/);
  assert.equal(prompts.at(-1).includes('unrelated.fig'), false);
}
await command('open VIP panel');
assert.equal(prompts.at(-1).includes('unrelated.fig'), false);
await command('map');
assert.match(prompts.at(-1), /not node IDs/);
assert.equal(prompts.at(-1).includes('unrelated.fig'), false);
await command('preview');
assert.equal(prompts.at(-1).includes('unrelated.fig'), false);
assert.equal(await readFile(path.join(root, '.figlens/component-mappings.md'), 'utf8'), mapping);

for (const name of ['plan', 'yolo']) {
  let resume;
  ctx.waitForIdle = () => new Promise(resolve => { resume = resolve; });
  const before = prompts.length;
  const pending = command(`${name} Only the card empty state`);
  assert.equal(prompts.length, before, 'wait for idle before dispatching');
  resume();
  await pending;
  assert.equal(prompts.length, before + 1);
}
assert.deepEqual(messages, [], 'no hidden command error was swallowed');
const version = JSON.parse(await readFile(path.join(sdk, 'package.json'), 'utf8')).version;
const checks = ['actual Pi loader and skill discovery', 'adjacent planning and yolo references', 'TUI/RPC/print plan-only and implementation dispatch', 'verbatim local scope', 'no filesystem writes or manual selection', 'idle ordering', 'conversational open without IDs or saved state', 'yolo help and autocomplete', 'empty yolo request resolves conversation'];
await writeFile(path.join(root, 'results.json'), JSON.stringify({ sdk: version, entry: extensionPath, checks, modelCalled: false, prompt: prompts[0] }, null, 2));
console.log(JSON.stringify({ sdk: version, entry: extensionPath, checks: checks.length, modelCalled: false, output: root }, null, 2));
