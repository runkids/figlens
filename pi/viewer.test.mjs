import assert from 'node:assert/strict';
import test from 'node:test';
import { Viewer } from './viewer.mjs';

const service = `
const http = require('node:http');
// reset.fig drops its first two connections, a network-level failure fetch reports as TypeError.
let resets = process.argv.includes('reset.fig') ? 2 : 0;
if (process.argv.includes('bad.fig')) { console.error('Invalid design'); process.exit(1); }
if (process.argv.includes('early-exit.fig')) { console.error('Preview: http://127.0.0.1:9/test-token/'); process.exit(1); }
if (!process.argv.includes('slow.fig')) {
  const server = http.createServer((req, res) => {
    if (resets > 0) { resets--; return req.socket.destroy(); }
    if (req.url === '/test-token/catalog') {
      res.end(process.argv.includes('invalid-catalog.fig') ? 'not json'
        : process.argv.includes('null-catalog.fig') ? 'null' : JSON.stringify({ generation: 1 }));
    } else if (req.url.startsWith('/test-token/data/1/')) {
      const id = req.url.slice('/test-token/data/1/'.length);
      if (['999:999', 'I12:3;999:999'].includes(id)) { res.writeHead(400); res.end('node not found'); }
      else res.end(JSON.stringify({ nodes: { document: process.argv.includes('empty-scope.fig') ? null : { id } } }));
    } else res.end(JSON.stringify(process.argv.slice(1)));
  });
  server.listen(0, '127.0.0.1', () => process.stderr.write('Preview: http://127.0.0.1:' + server.address().port + '/test-token/\\n'));
} else { setInterval(() => {}, 1000); }
`;

for (const [name, script] of [
  ['direct', service],
  ['npm-style wrapper', `require('node:child_process').spawnSync(process.execPath, ['-e', ${JSON.stringify(service)}, ...process.argv.slice(1)], {stdio:'inherit'});`],
]) test(`viewer (${name}) passes exact arguments, preserves a running viewer on failure and closes owned servers`, async t => {
  const viewer = new Viewer(process.execPath, ['-e', script]);
  t.after(() => viewer.close());
  const file = 'design with spaces;$(touch unwanted).fig';
  const first = await viewer.start(file, 'I12:3;4:5');
  assert.deepEqual(await (await fetch(first.url)).json(), ['preview', file, 'I12:3;4:5']);
  await assert.rejects(viewer.start('bad.fig'), /Invalid design/);
  await assert.rejects(viewer.start('early-exit.fig'), /Preview (exited|is not ready)/);
  await assert.rejects(viewer.start('invalid-catalog.fig'), /Preview is not ready/);
  await assert.rejects(viewer.start(file, '999:999'), /node not found/);
  await assert.rejects(viewer.start(file, 'I12:3;999:999'), /node not found/);
  await assert.rejects(viewer.start('empty-scope.fig', '12:3'), /Preview.*(node|scope)/);
  assert.equal((await fetch(first.url)).status, 200);
  const alias = await viewer.start(file, 'https://www.figma.com/design/key/name?node-id=12-3&mode=design');
  assert.equal((await fetch(alias.url)).status, 200);
  await assert.rejects(fetch(first.url));
  const second = await viewer.start('other.fig');
  assert.deepEqual(await (await fetch(second.url)).json(), ['preview', 'other.fig']);
  await assert.rejects(fetch(alias.url));
  await viewer.close();
  assert.equal(viewer.children.size, 0);
  await assert.rejects(fetch(second.url));
});

test('a network-level failure before the service answers is retried until startup succeeds', async t => {
  const viewer = new Viewer(process.execPath, ['-e', service]);
  t.after(() => viewer.close());
  const started = await viewer.start('reset.fig');
  assert.deepEqual(await (await fetch(started.url)).json(), ['preview', 'reset.fig']);
  // A TypeError from reading a bad catalog is not a connection failure: it ends startup at once.
  const begun = Date.now();
  await assert.rejects(viewer.start('null-catalog.fig', '12:3'), /Preview is not ready/);
  assert.ok(Date.now() - begun < 5000);
});

test('cancelled startup stops its server and a failed spawn reports its error', async t => {
  const viewer = new Viewer(process.execPath, ['-e', service]);
  t.after(() => viewer.close());
  const abort = new AbortController();
  const pending = viewer.start('slow.fig', undefined, { signal: abort.signal });
  abort.abort();
  await assert.rejects(pending, /cancelled/);
  assert.equal(viewer.children.size, 0);
  await assert.rejects(viewer.start('unused.fig', undefined, { signal: abort.signal }), /abort/i);
  const missing = new Viewer('/nonexistent-figlens');
  await assert.rejects(missing.start('unused.fig'), /ENOENT/);
  await missing.close();
});

test('a live POSIX process-group permission error is reported rather than treated as an exit race', { skip: process.platform === 'win32' }, async t => {
  const viewer = new Viewer(process.execPath, ['-e', service]);
  t.after(() => viewer.close());
  const current = await viewer.start('live.fig');
  const pid = [...viewer.children][0].pid;
  const kill = process.kill;
  try {
    process.kill = (target, signal) => {
      if (target === -pid) throw Object.assign(new Error('permission denied'), { code: 'EPERM' });
      return kill(target, signal);
    };
    await assert.rejects(viewer.close(), /Could not stop viewer process group/);
    assert.equal((await fetch(current.url)).status, 200);
  } finally {
    process.kill = kill;
  }
  await viewer.close();
});
