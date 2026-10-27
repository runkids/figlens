// Offline characterization of the installed Pi runtime; no provider requests or credentials.
// Usage: node scripts/check-e2e-pilot-images.mjs <pi-package-dir> <events.jsonl> <new-proof.json>
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { findPackageJSON } from 'node:module';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const [packageDir, eventsFile, outputFile] = process.argv.slice(2);
assert(packageDir && eventsFile && outputFile, 'Provide the installed Pi package, retained events, and a new proof path');
const aiPackage = findPackageJSON('@earendil-works/pi-ai', pathToFileURL(path.join(path.resolve(packageDir), 'package.json')));
const aiEntry = path.join(path.dirname(aiPackage), 'dist/index.js');
const load = filename => import(pathToFileURL(filename).href);
const { loadExtensions } = await load(path.join(packageDir, 'dist/core/extensions/loader.js'));
const { ExtensionRunner } = await load(path.join(packageDir, 'dist/core/extensions/runner.js'));
const { convertToLlm } = await load(path.join(packageDir, 'dist/core/messages.js'));
const { convertResponsesMessages } = await load(path.join(path.dirname(aiEntry), 'api/openai-responses-shared.js'));
const catalog = JSON.parse(await readFile(path.join(path.dirname(aiEntry), 'providers/data/openai-codex.json'), 'utf8'));
const model = catalog['openai-codex-responses']['chat:gpt-6.1-sol'];
assert(model && model.input.includes('image'), 'The original model must still support image input');
const events = (await readFile(eventsFile, 'utf8')).trim().split('\n').map(line => JSON.parse(line));
const call = events.find(event => event.type === 'tool_execution_end' && event.toolName === 'figma_get_screenshot');
assert(call && !call.isError && !call.result.isError, 'Retain a successful screenshot tool result');
const rawImages = call.result.details.raw.content.filter(block => block.type === 'image');
assert(rawImages.length > 0, 'The original MCP result must contain an image');
const event = {
  type: 'tool_result', toolCallId: call.toolCallId, toolName: call.toolName,
  input: {}, ...call.result, isError: false,
};
const extensionPath = fileURLToPath(new URL('./e2e-pilot-images.mjs', import.meta.url));
const loaded = await loadExtensions([extensionPath], process.cwd());
assert.equal(loaded.errors.length, 0, JSON.stringify(loaded.errors));
assert.equal(loaded.extensions.length, 1);
// This hook never reads session/model-registry context; no agent or model run is started.
const runner = new ExtensionRunner(loaded.extensions, loaded.runtime, process.cwd(), undefined, undefined);
const errors = [];
runner.onError(error => errors.push(error));
const repaired = { ...event, ...await runner.emitToolResult(event) };
assert.equal(errors.length, 0, JSON.stringify(errors));

function serialize(result) {
  const messages = convertToLlm([
    { role: 'user', content: 'Inspect this design.', timestamp: 0 },
    { role: 'assistant', provider: model.provider, api: model.api, model: model.id,
      content: [{ type: 'toolCall', id: call.toolCallId, name: call.toolName, arguments: {} }],
      stopReason: 'toolUse', timestamp: 1 },
    { role: 'toolResult', toolCallId: call.toolCallId, toolName: call.toolName,
      content: result.content, isError: false, timestamp: 2 },
  ]);
  return convertResponsesMessages(model, { messages }, new Set([model.provider]));
}
function images(payload) {
  return payload.filter(item => item.type === 'function_call_output')
    .flatMap(item => Array.isArray(item.output) ? item.output : [])
    .filter(block => block.type === 'input_image');
}
const before = images(serialize(event));
const after = images(serialize(repaired));
assert.equal(before.length, 0, 'Negative control must reproduce the placeholder-only defect');
assert.equal(after.length, rawImages.length);
for (const [index, image] of rawImages.entries()) {
  assert.equal(after[index].image_url, `data:${image.mimeType};base64,${image.data}`);
}
const pkg = JSON.parse(await readFile(path.join(packageDir, 'package.json'), 'utf8'));
const proof = {
  mode: 'offline Pi extension loader/runner and original-model Responses serializer',
  piVersion: pkg.version, model: `${model.provider}/${model.id}`, api: model.api,
  providerRequests: 0, modelRuns: 0,
  negativeControlImageCount: before.length, repairedPayloadImageCount: after.length,
  byteIdenticalImages: rawImages.map(image => ({
    mimeType: image.mimeType,
    sha256: createHash('sha256').update(Buffer.from(image.data, 'base64')).digest('hex'),
  })),
  limitations: 'Verifies the serialized model input, not remote acceptance or visual understanding. No inference was performed.',
};
await writeFile(outputFile, JSON.stringify(proof, null, 2) + '\n', { flag: 'wx' });
console.log(JSON.stringify(proof, null, 2));
