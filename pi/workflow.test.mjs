import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { Workflow } from './workflow.mjs';

function instance(id, state = 'Closed', label = 'Hello', rendered = label) {
  return {
    id, name: 'Suggestion', type: 'INSTANCE', componentId: state === 'Closed' ? '7:16' : '7:24',
    componentProperties: { State: { type: 'VARIANT', value: state }, Label: { type: 'TEXT', value: label } },
    children: [{ id: `${id};1:1`, type: 'TEXT', characters: rendered }],
  };
}

async function fixture(t) {
  const root = await mkdtemp(path.join(tmpdir(), 'figlens-mapping-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(path.join(root, 'design.fig'), 'snapshot one');
  await mkdir(path.join(root, 'src'));
  await writeFile(path.join(root, 'src/Card.vue'), '<template><article><slot /></article></template>');
  await writeFile(path.join(root, 'src/theme.css'), 'article { border-radius: 4px }');
  const frames = {
    '10:200': [instance('10:201')],
    '22:103': [instance('22:250', 'Closed', 'Different text'), instance('22:233', 'Open')],
  };
  let calls = 0;
  const run = async ([command, , frame]) => {
    calls++;
    if (command === 'tree') return { type: 'DOCUMENT', children: [{ type: 'CANVAS', name: 'Page', children: Object.keys(frames).map(id => ({ type: 'FRAME', id, name: id })) }] };
    if (!frames[frame]) throw new Error('Unknown frame');
    return {
      document: { id: frame, name: `Frame ${frame}`, type: 'FRAME', children: frames[frame] },
      components: { '7:16': { componentSetId: '7:49', key: null }, '7:24': { componentSetId: '7:49', key: null } },
      componentSets: { '7:49': { name: 'Suggestion card' } },
    };
  };
  const flow = new Workflow(root, run);
  const proposal = {
    instance: '10:201', target: {
      path: 'src/Card.vue', export: 'default', import: 'import Card from "./Card.vue";',
      example: '<Card>{{ currentLabel }}</Card>',
    },
    dependencies: ['src/theme.css'], rationale: 'Card provides the container; the slot receives current design text.',
    checks: ['Source inspected; no compilation check executed.'], issues: [],
  };
  await flow.select('design.fig', '10:200');
  return { root, flow, frames, run, proposal, get calls() { return calls; } };
}

test('proposals stay pending, human review persists and reuses a Vue mapping on another frame', async t => {
  const { root, flow, run, proposal } = await fixture(t);
  const { pending } = await flow.propose({ proposals: [proposal] });
  let data = await flow.bindings();
  assert.equal(data.items[0].status, 'pending');
  assert.equal(data.items[0].approved.length, 0);
  await flow.review(pending[0].id, 'approve');
  const reopened = new Workflow(root, run);
  assert.equal((await reopened.bindings()).items[0].status, 'approved');
  await reopened.select('design.fig', '22:103');
  data = await reopened.bindings();
  assert.equal(data.items[0].status, 'approved');
  assert.deepEqual(data.items[0].texts, ['Different text']);
  assert.equal(data.items[1].status, 'unmatched');
  assert.equal((await reopened.read()).state.active.frame, '22:103');
});

test('an unreviewed variant of the same main component is not automatically approved', async t => {
  const { flow, frames, proposal, root } = await fixture(t);
  const { pending } = await flow.propose({ proposals: [proposal] });
  await flow.review(pending[0].id, 'approve');
  frames['10:200'][0].componentProperties.State.value = 'Loading';
  await writeFile(path.join(root, 'design.fig'), 'new variant');
  const data = await flow.bindings();
  assert.equal(data.items[0].status, 'unmatched');
  assert.match(data.items[0].needsReview[0], /variant/);
});

test('unrelated snapshot edits preserve mappings but relevant structure and code changes require review', async t => {
  const fixtureData = await fixture(t);
  const { flow, root, proposal, frames } = fixtureData;
  const { pending } = await flow.propose({ proposals: [proposal] });
  await flow.review(pending[0].id, 'approve');
  await writeFile(path.join(root, 'design.fig'), 'an unrelated frame changed');
  assert.equal((await flow.bindings()).items[0].status, 'approved');
  await writeFile(path.join(root, 'src/theme.css'), 'article { border-radius: 8px }');
  assert.equal((await flow.bindings()).items[0].status, 'stale');
  frames['10:200'][0].children.push({ id: 'new', type: 'RECTANGLE' });
  await writeFile(path.join(root, 'design.fig'), 'relevant structure changed');
  assert.equal((await flow.bindings()).items[0].status, 'unmatched');
  assert.ok(fixtureData.calls >= 3);
});

test('missing code files and cross-file ID collisions never reuse an approval', async t => {
  const { flow, root, proposal } = await fixture(t);
  const { pending } = await flow.propose({ proposals: [proposal] });
  await flow.review(pending[0].id, 'approve');
  await writeFile(path.join(root, 'other.fig'), 'snapshot one');
  assert.equal((await flow.bindings({ file: 'other.fig', frame: '10:200' })).items[0].status, 'unmatched');
  assert.equal((await flow.read()).state.active.file, 'design.fig');
  await rm(path.join(root, 'src/Card.vue'));
  assert.equal((await flow.bindings()).items[0].status, 'stale');
});

test('text conflicts require a resolution and cannot spread to a different conflicted instance', async t => {
  const { flow, root, frames, proposal } = await fixture(t);
  frames['10:200'][0] = instance('10:201', 'Closed', 'Not today', 'Hold to confirm');
  await writeFile(path.join(root, 'design.fig'), 'conflict');
  const { pending } = await flow.propose({ proposals: [proposal] });
  assert.match(pending[0].issues[0], /Not today/);
  await assert.rejects(flow.review(pending[0].id, 'approve'), /resolution/);
  assert.equal((await flow.pending()).length, 1);
  await flow.review(pending[0].id, 'approve', { resolution: 'Use visible descendant text: Hold to confirm.' });
  assert.equal((await flow.bindings()).items[0].status, 'approved');
  frames['22:103'][0] = instance('22:250', 'Closed', 'Not today', 'Hold to confirm');
  await writeFile(path.join(root, 'design.fig'), 'another conflicted instance');
  const data = await flow.bindings({ file: 'design.fig', frame: '22:103' });
  assert.equal(data.items[0].status, 'unmatched');
  await flow.select('design.fig', '22:103');
  const next = await flow.propose({ proposals: [{ ...proposal, instance: '22:250' }] });
  await flow.review(next.pending[0].id, 'approve', { resolution: 'Use visible text for this second instance too.' });
  assert.equal((await flow.bindings()).items[0].status, 'approved');
  await flow.select('design.fig', '10:200');
  assert.equal((await flow.bindings()).items[0].status, 'approved');
});

test('approval rechecks design and code evidence after the review dialog', async t => {
  const { flow, root, proposal, frames } = await fixture(t);
  let { pending } = await flow.propose({ proposals: [proposal] });
  await writeFile(path.join(root, 'src/Card.vue'), 'changed implementation');
  await assert.rejects(flow.review(pending[0].id, 'approve'), /Code evidence changed/);
  ({ pending } = await flow.propose({ proposals: [proposal] }));
  frames['10:200'][0].children[0].characters = 'Changed text';
  await writeFile(path.join(root, 'design.fig'), 'new design');
  await assert.rejects(flow.review(pending[0].id, 'approve'), /Design evidence changed/);
});

test('editing a target clears previously reported checks; skip and supersede retain history', async t => {
  const { flow, proposal } = await fixture(t);
  let { pending } = await flow.propose({ proposals: [proposal] });
  await flow.review(pending[0].id, 'skip');
  ({ pending } = await flow.propose({ proposals: [proposal] }));
  await flow.review(pending[0].id, 'approve', { target: { ...proposal.target, example: '<Card><p>{{ label }}</p></Card>' } });
  assert.deepEqual((await flow.bindings()).items[0].approved[0].checks, []);
  ({ pending } = await flow.propose({ proposals: [proposal] }));
  await flow.review(pending[0].id, 'approve');
  assert.deepEqual((await flow.read()).state.mappings.map(entry => entry.status), ['skipped', 'superseded', 'approved']);
});

test('rejects traversal, external code symlinks, empty candidates and malformed selections', async t => {
  const { flow, root, proposal } = await fixture(t);
  await symlink(path.join(root, 'design.fig'), path.join(root, 'src/local-link'));
  await assert.rejects(flow.propose({ proposals: [{ ...proposal, target: { ...proposal.target, path: '../outside' } }] }));
  await symlink('/etc/hosts', path.join(root, 'src/external'));
  await assert.rejects(flow.propose({ proposals: [{ ...proposal, dependencies: ['src/external'] }] }), /inside the project/);
  await assert.rejects(flow.propose({ proposals: [] }), /1-20/);
  await assert.rejects(flow.bindings({ file: 'design.fig' }), /together/);
  await assert.rejects(flow.bindings({ offset: -1 }), /offset/);
});

test('preserves hand-written guide notes and refuses corrupt state or another writer', async t => {
  const { flow } = await fixture(t);
  const initial = await readFile(flow.guide, 'utf8');
  await writeFile(flow.guide, `Personal notes\n${initial}\nMore notes\n`);
  await flow.select('design.fig', '22:103');
  const saved = await readFile(flow.guide, 'utf8');
  assert.ok(saved.startsWith('Personal notes\n'));
  assert.ok(saved.endsWith('\nMore notes\n'));
  await mkdir(path.join(flow.directory, '.mapping-lock'));
  await assert.rejects(flow.select('design.fig', '10:200'), /Another mapping writer/);
  await rm(path.join(flow.directory, '.mapping-lock'), { recursive: true });
  await writeFile(flow.guide, saved.replace('"version": 1', '"version": 999'));
  await assert.rejects(flow.select('design.fig', '10:200'), /Unsupported/);
  assert.match(await readFile(flow.guide, 'utf8'), /999/);
});

test('refuses a symlinked guide instead of modifying its target', async t => {
  const { flow, root } = await fixture(t);
  const original = await readFile(flow.guide, 'utf8');
  await writeFile(path.join(root, 'notes.md'), original);
  await rm(flow.guide);
  await symlink(path.join(root, 'notes.md'), flow.guide);
  await assert.rejects(flow.select('design.fig', '22:103'), /symbolic link/);
  assert.equal(await readFile(path.join(root, 'notes.md'), 'utf8'), original);
});

test('paginates bounded context and omits hidden instances', async t => {
  const { flow, root, frames } = await fixture(t);
  frames['10:200'] = Array.from({ length: 60 }, (_, index) => instance(`1:${index}`));
  frames['10:200'][59].visible = false;
  await writeFile(path.join(root, 'design.fig'), 'many instances');
  const first = await flow.bindings({ limit: 5 });
  assert.equal(first.total, 59);
  assert.equal(first.items.length, 5);
  assert.equal(first.nextOffset, 5);
  const last = await flow.bindings({ offset: 55, limit: 5 });
  assert.equal(last.items.length, 4);
  assert.equal(last.nextOffset, null);
});
