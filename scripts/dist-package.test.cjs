const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } = require('node:fs');
const { tmpdir } = require('node:os');
const path = require('node:path');
const test = require('node:test');

// Lays out a fixture tree whose fake cargo writes each requested --bin as its own name; with
// TOUCH_CORE set, the companion build also rewrites the core, as a feature leaking into it would.
function fixtureTree(t) {
  const root = path.resolve(__dirname, '..');
  const fixture = mkdtempSync(path.join(tmpdir(), 'figlens-dist-pack-'));
  t.after(() => rmSync(fixture, { recursive: true, force: true }));
  mkdirSync(path.join(fixture, 'scripts'));
  mkdirSync(path.join(fixture, 'bin'));
  cpSync(path.join(root, 'scripts/build-dist.sh'), path.join(fixture, 'scripts/build-dist.sh'));
  cpSync(path.join(root, 'skills'), path.join(fixture, 'skills'), { recursive: true });
  cpSync(path.join(root, 'Cargo.toml'), path.join(fixture, 'Cargo.toml'));
  // Only package fixture bytes; this test never invokes a Rust toolchain.
  writeFileSync(path.join(fixture, 'bin/cargo'), `#!/bin/sh
set -eu
while [ $# -gt 0 ]; do
  case "$1" in --target) target="$2"; shift ;; --bin) bin="$2"; shift ;; esac
  shift
done
directory="target/dist-build/$target/release"
mkdir -p "$directory"
case "$target" in *windows*) exe=.exe ;; *) exe= ;; esac
printf "$bin" > "$directory/$bin$exe"
chmod +x "$directory/$bin$exe"
if [ -n "\${TOUCH_CORE:-}" ] && [ "$bin" = figlens-embed ]; then printf changed > "$directory/figlens$exe"; fi
`, { mode: 0o755 });
  const run = env => execFileSync('sh', ['scripts/build-dist.sh'], { cwd: fixture, stdio: 'pipe', env: { ...process.env, ...env, PATH: `${path.join(fixture, 'bin')}${path.delimiter}${process.env.PATH}`, FIGLENS_TARGETS: 'x86_64-unknown-linux-musl x86_64-pc-windows-gnu', FIGLENS_DIST_DIR: 'candidate' } });
  return { root, fixture, run };
}

test('native tar and Windows zip carry figlens, its model runtime and the skill references', { skip: process.platform === 'win32' }, t => {
  const { root, fixture, run } = fixtureTree(t);
  run({});
  const version = readFileSync(path.join(root, 'Cargo.toml'), 'utf8').match(/^version = "([^"]+)"/m)[1];
  const tar = path.join(fixture, `candidate/figlens-${version}-x86_64-unknown-linux-musl.tar.gz`);
  const zip = path.join(fixture, `candidate/figlens-${version}-x86_64-pc-windows-gnu.zip`);
  const verifyZip = `import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    assert z.read("figlens.exe") == b"figlens"
    assert z.read("figlens-embed.exe") == b"figlens-embed"
    for member in sys.argv[2:]:
        assert z.read(member) == open(member, "rb").read()
`;
  const resources = ['skills/figlens/SKILL.md', 'skills/figlens/references/audit.md', 'skills/figlens/references/planning.md', 'skills/figlens/references/planning-details.md', 'skills/figlens/references/yolo.md'];
  for (const file of resources) {
    assert.equal(execFileSync('tar', ['-xOf', tar, file], { encoding: 'utf8' }), readFileSync(path.join(root, file), 'utf8'));
  }
  execFileSync('python3', ['-c', verifyZip, zip, ...resources], { cwd: root });
  assert.equal(execFileSync('tar', ['-xOf', tar, 'figlens'], { encoding: 'utf8' }), 'figlens');
  assert.equal(execFileSync('tar', ['-xOf', tar, 'figlens-embed'], { encoding: 'utf8' }), 'figlens-embed');
  assert.match(execFileSync('tar', ['-tvzf', tar], { encoding: 'utf8' }), /^-rwx.* figlens-embed$/m);
});

test('a model runtime build that changes the model-free core fails the release build', { skip: process.platform === 'win32' }, t => {
  const { run } = fixtureTree(t);
  assert.throws(() => run({ TOUCH_CORE: '1' }), e => /building figlens-embed changed figlens for x86_64-unknown-linux-musl/.test(e.stderr));
});
