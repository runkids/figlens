const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

test('concurrent first use keeps the verified binary while an MCP process is using it', async t => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'figlens-cache-test-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const version = '0.3.0';
  const directory = path.join(root, 'figlens', 'npm', version);
  const binary = path.join(directory, 'figlens.exe');
  const archive = Buffer.from('verified test archive');
  const checksum = crypto.createHash('sha256').update(archive).digest('hex');
  const code = fs.readFileSync(path.join(__dirname, 'bin/figlens.js'), 'utf8');
  let running = false;
  let release;
  const gate = new Promise(resolve => { release = resolve; });
  function launch(wait) {
    const exits = [];
    const errors = [];
    const localFs = {
      ...fs,
      rmSync(file, options) {
        if (file === directory && running) throw Object.assign(new Error('binary is in use'), { code: 'EPERM' });
        return fs.rmSync(file, options);
      },
    };
    const spawnSync = (command, _args, options) => {
      if (command === 'tar') fs.writeFileSync(path.join(options.cwd, 'figlens.exe'), 'verified binary');
      else {
        assert.equal(command, binary);
        assert.equal(fs.readFileSync(binary, 'utf8'), 'verified binary');
        running = true;
      }
      return { status: 0 };
    };
    const completion = vm.runInNewContext(code, {
      Buffer,
      require(name) {
        if (name === 'node:fs') return localFs;
        if (name === 'node:child_process') return { spawnSync };
        if (name === '../target.js') return { asset: () => ({ archive: 'test.zip', binary: 'figlens.exe' }) };
        if (name === '../package.json') return { version };
        return require(name);
      },
      process: { env: { XDG_CACHE_HOME: root }, platform: 'win32', arch: 'x64', argv: ['node', 'figlens'], exit: code => exits.push(code) },
      console: { error: message => errors.push(message) },
      fetch: async url => {
        if (wait) await gate;
        return { ok: true, arrayBuffer: async () => url.endsWith('SHA256SUMS') ? Buffer.from(`${checksum}  test.zip\n`) : archive };
      },
    });
    return { completion, exits, errors };
  }
  const slower = launch(true); // Both callers observe an empty cache before the faster one installs.
  const faster = launch(false);
  await faster.completion;
  assert.deepEqual(faster.exits, [0]);
  release();
  await slower.completion;
  assert.deepEqual(slower.errors, []);
  assert.deepEqual(slower.exits, [0]);
  assert.equal(fs.readFileSync(binary, 'utf8'), 'verified binary');
  assert.deepEqual(fs.readdirSync(path.dirname(directory)), [version]);
});
