import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

test('the existing CLI npm package includes the maintained Pi entry and its adjacent launcher', t => {
  const root = fileURLToPath(new URL('..', import.meta.url));
  const cache = mkdtempSync(path.join(tmpdir(), 'figlens-npm-pack-'));
  t.after(() => rmSync(cache, { recursive: true, force: true }));
  execFileSync(process.execPath, ['scripts/prepare-npm.cjs'], { cwd: root });
  const [packed] = JSON.parse(execFileSync(process.platform === 'win32' ? 'npm.cmd' : 'npm',
    ['pack', '--dry-run', '--ignore-scripts', '--json', '--cache', cache],
    { cwd: path.join(root, 'npm'), shell: process.platform === 'win32', encoding: 'utf8' }));
  const manifest = JSON.parse(readFileSync(path.join(root, 'npm/package.json'), 'utf8'));
  assert.equal(manifest.name, '@runkids/figlens');
  assert.ok(manifest.keywords.includes('pi-package'));
  assert.deepEqual(manifest.pi.extensions, ['./pi/index.ts']);
  assert.deepEqual(manifest.pi.skills, ['./skills']);
  assert.ok(manifest.pi.image.endsWith('/assets/pi-demo-poster.png'));
  assert.ok(manifest.pi.video.endsWith('/assets/pi-demo.mp4'));
  const files = packed.files.map(file => file.path);
  assert.ok(files.includes('bin/figlens.js'));
  assert.ok(files.includes('skills/figlens/SKILL.md'));
  assert.equal(readFileSync(path.join(root, 'npm/skills/figlens/SKILL.md'), 'utf8'), readFileSync(path.join(root, 'skills/figlens/SKILL.md'), 'utf8'));
  for (const file of ['index.ts', 'workflow.mjs', 'package.json', 'README.md']) {
    assert.ok(files.includes(`pi/${file}`));
    assert.equal(readFileSync(path.join(root, 'npm/pi', file), 'utf8'), readFileSync(path.join(root, 'pi', file), 'utf8'));
  }
  assert.ok(files.every(file => !file.endsWith('.fig') && !file.includes('samples/') && !file.endsWith('.test.mjs')));
  assert.ok(Object.values(manifest.peerDependenciesMeta).every(peer => peer.optional));
});
