const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } = require('node:fs');
const { tmpdir } = require('node:os');
const path = require('node:path');
const test = require('node:test');

test('native tar and Windows zip include the skill and workflow references', { skip: process.platform === 'win32' }, t => {
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
while [ "$1" != "--target" ]; do shift; done
target="$2"
directory="target/dist-build/$target/release"
mkdir -p "$directory"
case "$target" in
  *windows*) printf fixture > "$directory/figlens.exe" ;;
  *) printf fixture > "$directory/figlens"; chmod +x "$directory/figlens" ;;
esac
`, { mode: 0o755 });
  const version = readFileSync(path.join(root, 'Cargo.toml'), 'utf8').match(/^version = "([^"]+)"/m)[1];
  execFileSync('sh', ['scripts/build-dist.sh'], { cwd: fixture, env: { ...process.env, PATH: `${path.join(fixture, 'bin')}${path.delimiter}${process.env.PATH}`, FIGLENS_TARGETS: 'x86_64-unknown-linux-musl x86_64-pc-windows-gnu' } });
  const tar = path.join(fixture, `dist/figlens-${version}-x86_64-unknown-linux-musl.tar.gz`);
  const zip = path.join(fixture, `dist/figlens-${version}-x86_64-pc-windows-gnu.zip`);
  const verifyZip = `import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    assert z.read("figlens.exe") == b"fixture"
    for member in sys.argv[2:]:
        assert z.read(member) == open(member, "rb").read()
`;
  const resources = ['skills/figlens/SKILL.md', 'skills/figlens/references/planning.md', 'skills/figlens/references/planning-details.md', 'skills/figlens/references/yolo.md'];
  for (const file of resources) {
    assert.equal(execFileSync('tar', ['-xOf', tar, file], { encoding: 'utf8' }), readFileSync(path.join(root, file), 'utf8'));
  }
  execFileSync('python3', ['-c', verifyZip, zip, ...resources], { cwd: root });
  assert.equal(execFileSync('tar', ['-xOf', tar, 'figlens'], { encoding: 'utf8' }), 'fixture');
});
