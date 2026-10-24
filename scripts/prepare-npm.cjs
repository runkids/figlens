// Stage maintained Pi resources into the existing CLI package for one release.
const { copyFileSync, mkdirSync } = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '..');
mkdirSync(path.join(root, 'npm/pi'), { recursive: true });
for (const file of ['index.ts', 'workflow.mjs', 'viewer.mjs', 'package.json', 'README.md']) {
  copyFileSync(path.join(root, 'pi', file), path.join(root, 'npm/pi', file));
}
for (const file of ['SKILL.md', 'references/planning.md', 'references/planning-details.md', 'references/yolo.md']) {
  const destination = path.join(root, 'npm/skills/figlens', file);
  mkdirSync(path.dirname(destination), { recursive: true });
  copyFileSync(path.join(root, 'skills/figlens', file), destination);
}
