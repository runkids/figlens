// Real SDK + native viewer lifecycle; scope resolution is scripted, not a model run.
import assert from 'node:assert/strict';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const [sdk, binary, fig, output, entry = 'pi/index.ts'] = process.argv.slice(2);
if (!output) throw new Error('Usage: node scripts/check-pi-preview.mjs <sdk> <binary> <sample.fig> <new-output> [entry]');
const root = path.resolve(output);
await mkdir(root);
process.env.PI_CODING_AGENT_DIR = path.join(root, 'agent');
process.env.FIGLENS_BIN = path.resolve(binary);
process.env.XDG_CACHE_HOME = path.join(root, 'cache');
const { loadExtensions, createExtensionRuntime } = await import(pathToFileURL(path.join(sdk, 'dist/core/extensions/loader.js')));
const runtime = createExtensionRuntime();
const prompts = [];
const messages = [];
runtime.sendUserMessage = prompt => prompts.push(prompt);
runtime.sendMessage = message => messages.push(message);
const loaded = await loadExtensions([path.resolve(entry)], root, undefined, runtime);
assert.deepEqual(loaded.errors, []);
const extension = loaded.extensions[0];
const ctx = { cwd: root, hasUI: false, mode: 'rpc', waitForIdle: async () => {} };
const command = args => extension.commands.get('figlens').handler(args, ctx);
const preview = (file, node) => extension.tools.get('figlens_preview').definition.execute('check', { file, node }, new AbortController().signal, () => {}, ctx);
const shutdown = async () => {
  for (const handler of extension.handlers.get('session_shutdown') || []) await handler({}, ctx);
};
try {
  await command('preview List views');
  assert.equal(prompts.length, 1);
  assert.match(prompts[0], /Call figlens_preview/);
  assert.match(prompts[0], /not node IDs/);
  await command(`preview "${path.resolve(fig)}"`);
  assert.equal(prompts.length, 1, 'a known file starts immediately without a model');
  const direct = messages.findLast(message => message.content.includes('Open design viewer'));
  assert.ok(direct, `no viewer link; extension messages: ${JSON.stringify(messages.map(message => message.content))}`);
  const directUrl = direct.content.match(/\((http:[^)]+)\)/)[1];
  const directResponse = await fetch(directUrl);
  assert.equal(directResponse.status, 200);
  await directResponse.arrayBuffer();
  const first = (await preview(path.resolve(fig))).structuredContent;
  await assert.rejects(fetch(directUrl));
  assert.match(await (await fetch(first.url)).text(), /figlens/i);
  await assert.rejects(preview(path.join(root, 'missing.fig')), /Preview (exited|is not ready)/);
  await assert.rejects(preview(path.resolve(fig), '4294967295:4294967295'), /node not found/);
  const readyResponse = await fetch(first.url);
  assert.equal(readyResponse.status, 200);
  await readyResponse.arrayBuffer();
  await command('close');
  assert.ok(messages.some(message => message.content.includes('viewer stopped')));
  await assert.rejects(fetch(first.url));
  const second = (await preview(path.resolve(fig))).structuredContent;
  const secondResponse = await fetch(second.url);
  assert.equal(secondResponse.status, 200);
  await secondResponse.arrayBuffer();
  await shutdown();
  await assert.rejects(fetch(second.url));
  const version = JSON.parse(await readFile(path.join(sdk, 'package.json'))).version;
  const result = { sdk: version, entry: path.resolve(entry), modelCalled: false, checks: ['conversational dispatch', 'direct file without a model', 'real native viewer URL', 'failed startup preserves viewer', 'invalid node preserves viewer', 'close stops viewer', 'session shutdown stops viewer'], first, second };
  await writeFile(path.join(root, 'results.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify({ sdk: version, checks: result.checks, output: root }));
} catch (error) {
  console.error('Preview check failed:', error);
  throw error;
} finally {
  await shutdown();
}
