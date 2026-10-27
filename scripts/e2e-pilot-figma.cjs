// Capture independent Figma evidence; requires the existing MCP SDK on NODE_PATH.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { Client } = require('@modelcontextprotocol/sdk/client/index.js');
const { StreamableHTTPClientTransport } = require('@modelcontextprotocol/sdk/client/streamableHttp.js');

async function main() {
  const [output, phase, nodeId] = process.argv.slice(2);
  assert(output && ['before', 'after'].includes(phase) && /^\d+:\d+$/.test(nodeId));
  const dir = path.join(output, 'official', `${phase}-reference`);
  await fs.mkdir(dir, { recursive: true });
  const client = new Client({ name: 'figlens-e2e-pilot', version: '1.0.0' });
  const measurements = [];
  try {
    await client.connect(new StreamableHTTPClientTransport(new URL('http://127.0.0.1:3845/mcp')));
    // Root design-context capture timed out twice; preserve that failure separately.
    for (const name of ['get_metadata', 'get_screenshot', 'get_variable_defs']) {
      const args = name === 'get_screenshot' ? { nodeId } : {
        nodeId, clientLanguages: 'html,css,javascript', clientFrameworks: 'none',
      };
      const start = performance.now();
      const result = await client.callTool({ name, arguments: args });
      measurements.push({ name, arguments: args, wallSeconds: (performance.now() - start) / 1000 });
      await fs.writeFile(path.join(dir, `${name}.json`), JSON.stringify(result, null, 2) + '\n', { flag: 'wx' });
      assert(!result.isError, JSON.stringify(result));
      const text = [];
      for (const [i, block] of result.content.entries()) {
        if (block.type === 'text') text.push(block.text);
        if (block.type === 'image') {
          assert.equal(block.mimeType, 'image/png');
          const image = Buffer.from(block.data, 'base64');
          assert.equal(image.subarray(1, 4).toString(), 'PNG');
          await fs.writeFile(path.join(dir, `${name}-${i}.png`), image, { flag: 'wx' });
          measurements.at(-1).pixels = [image.readUInt32BE(16), image.readUInt32BE(20)];
        }
      }
      await fs.writeFile(path.join(dir, `${name}.md`), text.join('\n\n'), { flag: 'wx' });
    }
    await fs.writeFile(path.join(dir, 'measurements.json'), JSON.stringify(measurements, null, 2) + '\n', { flag: 'wx' });
    console.log(JSON.stringify({ directory: dir, measurements }, null, 2));
  } finally {
    await client.close();
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
