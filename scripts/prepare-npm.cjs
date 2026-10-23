// Stage maintained Pi resources into the existing CLI package for one release.
const { copyFileSync, mkdirSync } = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '..');
mkdirSync(path.join(root, 'npm/pi'), { recursive: true });
for (const file of ['index.ts', 'workflow.mjs', 'package.json', 'README.md']) {
  copyFileSync(path.join(root, 'pi', file), path.join(root, 'npm/pi', file));
}
mkdirSync(path.join(root, 'npm/skills/figlens'), { recursive: true });
copyFileSync(path.join(root, 'skills/figlens/SKILL.md'), path.join(root, 'npm/skills/figlens/SKILL.md'));
