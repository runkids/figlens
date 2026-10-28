// Real installed Pi loader/runtime, public fixture only; no viewer socket or model call.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const [sdkInput, packageInput, fixture, cwdInput] = process.argv.slice(2);
if (!cwdInput) throw new Error('Usage: node scripts/check-pi-integration.mjs <installed-pi-root> <installed-figlens-root> <public.fig> <isolated-consumer>');
const [sdk, packaged, cwd] = [sdkInput, packageInput, cwdInput].map(p => path.resolve(p));
assert.ok(sdk.startsWith(cwd + path.sep) && packaged.startsWith(cwd + path.sep), 'Dependencies must belong to the isolated consumer');
assert.equal(process.env.FIGLENS_BIN, undefined, 'The packaged extension must select its adjacent wrapper');
const { loadExtensions, createExtensionRuntime } = await import(pathToFileURL(path.join(sdk, 'dist/core/extensions/loader.js')));
const { loadSkills } = await import(pathToFileURL(path.join(sdk, 'dist/core/skills.js')));
const runtime = createExtensionRuntime();
const messages = [];
const prompts = [];
runtime.sendMessage = message => messages.push(message);
runtime.sendUserMessage = prompt => prompts.push(prompt);
const loaded = await loadExtensions([path.join(packaged, 'pi/index.ts')], cwd, undefined, runtime);
assert.deepEqual(loaded.errors, []);
assert.equal(loaded.extensions.length, 1);
const extension = loaded.extensions[0];
assert.ok(extension.commands.has('figlens'));
for (const name of ['figlens_bindings', 'figlens_propose_bindings', 'figlens_preview']) assert.ok(extension.tools.has(name), name);
const ctx = { cwd, mode: 'rpc', hasUI: false, waitForIdle: async () => {} };
try {
  for (const handler of extension.handlers.get('session_start') || []) await handler({}, ctx);
  const registrations = runtime.mcpServers.list();
  assert.equal(registrations.length, 1);
  const server = registrations[0];
  assert.equal(server.name, 'figlens');
  assert.equal(server.config.command, process.execPath);
  assert.deepEqual(server.config.args, [path.join(packaged, 'bin/figlens.js'), 'mcp']);
  assert.equal(server.config.exposure, 'direct');
  const command = args => extension.commands.get('figlens').handler(args, ctx);
  await command('help');
  assert.match(messages.at(-1).content, /Ask Pi directly/);
  await command('plan Plan only the First success card');
  for (const name of ['SKILL.md', 'references/planning.md']) {
    assert.ok(prompts.at(-1).includes(JSON.stringify(path.join(packaged, 'skills/figlens', name))), name);
  }
  const read = await extension.tools.get('figlens_bindings').definition.execute('installed-smoke',
    { file: path.resolve(fixture), frame: '0:1' }, new AbortController().signal, undefined, ctx);
  assert.equal(read.structuredContent.frame, '0:1');
  assert.equal(read.structuredContent.name, 'First success');
  assert.equal(read.structuredContent.total, 0);
  const skills = loadSkills({ cwd, agentDir: process.env.PI_CODING_AGENT_DIR,
    skillPaths: [path.join(packaged, 'skills')], includeDefaults: false });
  assert.deepEqual(skills.diagnostics, []);
  assert.equal(skills.skills.length, 1);
  assert.equal(skills.skills[0].name, 'figlens');
  for (const name of ['SKILL.md', 'references/audit.md', 'references/planning.md', 'references/planning-details.md', 'references/yolo.md']) {
    assert.ok((await readFile(path.join(packaged, 'skills/figlens', name), 'utf8')).length > 100);
  }
  const pi = JSON.parse(await readFile(path.join(sdk, 'package.json'), 'utf8'));
  const pkg = JSON.parse(await readFile(path.join(packaged, 'package.json'), 'utf8'));
  console.log(JSON.stringify({ pi: pi.version, nodeMinimum: pi.engines.node, packageVersion: pkg.version,
    modelCalled: false, mcp: server.config,
    checks: ['real installed loader', 'runtime MCP registration', 'commands and tools',
      'public frame through packaged wrapper', 'skill and workflow resources'] }));
} finally {
  for (const handler of extension.handlers.get('session_shutdown') || []) await handler({}, ctx);
}
