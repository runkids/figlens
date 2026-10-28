// Stage maintained Pi resources into the existing CLI package for one release.
const { copyFileSync, mkdirSync } = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '..');
const output = process.argv[2] ? path.resolve(process.argv[2]) : path.join(root, 'npm');
mkdirSync(path.join(output, 'pi'), { recursive: true });
for (const file of ['index.ts', 'workflow.mjs', 'viewer.mjs', 'package.json', 'README.md']) {
  copyFileSync(path.join(root, 'pi', file), path.join(output, 'pi', file));
}
for (const file of ['SKILL.md', 'references/audit.md', 'references/planning.md', 'references/planning-details.md', 'references/yolo.md']) {
  const destination = path.join(output, 'skills/figlens', file);
  mkdirSync(path.dirname(destination), { recursive: true });
  copyFileSync(path.join(root, 'skills/figlens', file), destination);
}
