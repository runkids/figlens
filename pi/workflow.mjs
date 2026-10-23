import { createHash, randomUUID } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { lstat, mkdir, readFile, realpath, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';

const START = '<!-- figlens:managed:start -->';
const END = '<!-- figlens:managed:end -->';
const MAX_STATE_BYTES = 4 * 1024 * 1024;
const MAX_RESULT_BYTES = 24000;

function ordered(value) {
  if (Array.isArray(value)) return value.map(ordered);
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().map(key => [key, ordered(value[key])]));
  }
  return value;
}

function digest(value) {
  return createHash('sha256').update(JSON.stringify(ordered(value))).digest('hex');
}

async function fileHash(file) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest('hex');
}

function text(value, name, max = 8000) {
  if (typeof value !== 'string' || !value.trim() || value.length > max) {
    throw new Error(`${name} must be non-empty text, at most ${max} characters.`);
  }
  return value;
}

function strings(value, name, max = 20) {
  if (!Array.isArray(value) || value.length > max) throw new Error(`${name} must be an array of at most ${max} strings.`);
  return value.map(item => text(item, name, 2000));
}

function emptyState() {
  return { version: 1, active: null, mappings: [] };
}

function parseState(content) {
  if (!content) return emptyState();
  const start = content.indexOf(START);
  const end = content.indexOf(END);
  if (start < 0 || end < start || content.indexOf(START, start + START.length) !== -1 || content.indexOf(END, end + END.length) !== -1) {
    throw new Error('Mapping guide has missing or duplicate managed markers; it was not overwritten.');
  }
  const block = content.slice(start + START.length, end);
  const match = block.match(/^```json\n([\s\S]*?)\n```$/m);
  if (!match) throw new Error('Mapping guide has no state block; it was not overwritten.');
  const state = JSON.parse(match[1]);
  if (state.version !== 1 || !Array.isArray(state.mappings) || !('active' in state)) throw new Error('Unsupported mapping guide state.');
  if (state.active !== null) {
    text(state.active.file, 'active.file', 4096);
    text(state.active.frame, 'active.frame', 200);
  }
  const ids = new Set();
  for (const entry of state.mappings) {
    text(entry.id, 'mapping.id', 100);
    if (ids.has(entry.id)) throw new Error('Duplicate mapping id.');
    ids.add(entry.id);
    if (!['pending', 'approved', 'skipped', 'superseded'].includes(entry.status)) throw new Error('Invalid mapping status.');
    text(entry.source.file, 'source.file', 4096);
    text(entry.source.frame, 'source.frame', 200);
    text(entry.source.instance, 'source.instance', 200);
    text(entry.source.name, 'source.name', 2000);
    text(entry.source.componentId, 'source.componentId', 200);
    text(entry.source.signature, 'source.signature', 100);
    text(entry.source.snapshot, 'source.snapshot', 100);
    text(entry.source.textHash, 'source.textHash', 100);
    text(entry.target.path, 'target.path', 4096);
    text(entry.target.export, 'target.export', 200);
    text(entry.target.import, 'target.import');
    text(entry.target.example, 'target.example');
    if (!entry.files || typeof entry.files !== 'object' || Array.isArray(entry.files) || !Object.keys(entry.files).length) throw new Error('Missing source file evidence.');
    for (const [file, hash] of Object.entries(entry.files)) {
      text(file, 'evidence file', 4096);
      text(hash, 'evidence hash', 100);
    }
    text(entry.rationale, 'rationale', 2000);
    strings(entry.checks, 'checks');
    strings(entry.issues, 'issues');
    if (entry.resolution !== undefined) text(entry.resolution, 'resolution', 2000);
    if (entry.status === 'approved' && (!entry.approvedAt || (entry.issues.length && !entry.resolution))) throw new Error('Approved mapping is missing its review evidence.');
  }
  return state;
}

function renderState(state) {
  const summary = state.mappings.map(entry =>
    `- ${entry.status}: ${entry.source.name.replace(/[\r\n]/g, ' ')} → ${entry.target.path.replace(/[\r\n]/g, ' ')} (${entry.id})`,
  ).join('\n') || 'No mappings yet.';
  return `${START}\n## Mappings\n\n${summary}\n\n## Extension state\n\n\`\`\`json\n${JSON.stringify(state, null, 2)}\n\`\`\`\n${END}`;
}

function properties(node) {
  return Object.fromEntries(Object.entries(node.componentProperties ?? {}).map(([key, prop]) => [key,
    prop.type === 'TEXT' ? { type: prop.type } : { type: prop.type, value: prop.value },
  ]));
}

function shape(node) {
  return {
    type: node.type,
    componentId: node.componentId ?? null,
    properties: properties(node),
    children: (node.children ?? []).map(shape),
  };
}

function instances(document) {
  const found = [];
  function walk(node, visible = true) {
    visible &&= node.visible !== false;
    if (visible && node.type === 'INSTANCE') found.push(node);
    for (const child of node.children ?? []) walk(child, visible);
  }
  walk(document);
  return found;
}

function hasTextConflict(entry) {
  return entry.issues.some(issue => issue.startsWith('TEXT property '));
}

function summarizeInstance(node, doc) {
  const component = doc.components?.[node.componentId] ?? {};
  const set = doc.componentSets?.[component.componentSetId] ?? {};
  const texts = [];
  function walk(child, visible = true) {
    visible &&= child.visible !== false;
    if (visible && typeof child.characters === 'string') texts.push(child.characters);
    for (const next of child.children ?? []) walk(next, visible);
  }
  walk(node);
  const issues = Object.entries(node.componentProperties ?? {}).filter(([, prop]) =>
    prop.type === 'TEXT' && typeof prop.value === 'string' && prop.value.trim() && !texts.includes(prop.value),
  ).slice(0, 10).map(([key, prop]) => `TEXT property ${key}=${JSON.stringify(prop.value).slice(0, 1500)} is not present verbatim in visible descendant text. Inspect before choosing the implementation text.`);
  return {
    id: node.id, name: set.name || node.name, componentId: node.componentId ?? null,
    componentSetId: component.componentSetId ?? null, componentKey: component.key ?? null,
    properties: node.componentProperties ?? {}, texts, issues,
    signature: digest({ componentId: node.componentId, key: component.key ?? null, set: component.componentSetId ?? null, shape: shape(node) }),
  };
}

export class Workflow {
  constructor(root, run) {
    this.root = path.resolve(root);
    this.run = run;
    this.directory = path.join(this.root, '.figlens');
    this.guide = path.join(this.directory, 'component-mappings.md');
  }

  async read() {
    for (const file of [this.directory, this.guide]) {
      try {
        if ((await lstat(file)).isSymbolicLink()) throw new Error(`${file} must not be a symbolic link.`);
      } catch (error) {
        if (error.code !== 'ENOENT') throw error;
      }
    }
    let content = '';
    try { content = await readFile(this.guide, 'utf8'); } catch (error) { if (error.code !== 'ENOENT') throw error; }
    if (Buffer.byteLength(content) > MAX_STATE_BYTES) throw new Error('Mapping guide exceeds 4 MiB.');
    return { content, state: parseState(content) };
  }

  async mutate(change) {
    await mkdir(this.directory, { recursive: true });
    await this.read();
    const lock = path.join(this.directory, '.mapping-lock');
    try { await mkdir(lock); } catch (error) {
      if (error.code === 'EEXIST') throw new Error('Another mapping writer holds .figlens/.mapping-lock. Retry after it finishes; remove the lock only if that process has stopped.');
      throw error;
    }
    const temporary = path.join(this.directory, `.mapping-${randomUUID()}.tmp`);
    try {
      const { content, state } = await this.read();
      const result = await change(state);
      const managed = renderState(state);
      const output = content ? content.slice(0, content.indexOf(START)) + managed + content.slice(content.indexOf(END) + END.length)
        : `# figlens component mappings\n\nReviewed mappings for this project. Keep project notes outside the managed markers.\nThe JSON block is extension-owned state, not a public interchange format.\n\n${managed}\n`;
      if (Buffer.byteLength(output) > MAX_STATE_BYTES) throw new Error('Mapping guide would exceed 4 MiB.');
      await writeFile(temporary, output, { flag: 'wx', mode: 0o600 });
      if ((await this.read()).content !== content) throw new Error('Mapping guide changed during the operation; retry.');
      await rename(temporary, this.guide);
      return result;
    } finally {
      await rm(temporary, { force: true });
      await rm(lock, { recursive: true, force: true });
    }
  }

  async source(file) {
    const absolute = await realpath(path.resolve(this.root, text(file, 'file', 4096)));
    if (path.extname(absolute).toLowerCase() !== '.fig' || !(await lstat(absolute)).isFile()) throw new Error('Select a local .fig file.');
    const relative = path.relative(await realpath(this.root), absolute);
    return { absolute, file: relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative) ? absolute : relative.split(path.sep).join('/') };
  }

  async frames(file, signal, query) {
    const source = await this.source(file);
    const found = await this.run(['find', source.absolute, text(query, 'screen name', 4096), '--type', 'FRAME', '--limit', '200'], signal);
    return { file: source.file, truncated: found.truncated, frames: found.results.map(frame => ({
      id: frame.id, name: frame.path.slice(1).map(node => node.name).join(' / '), size: frame.size,
    })) };
  }

  async design(selection, signal) {
    if (!selection) throw new Error('Supply file and frame for the design in the current request. Use the figlens MCP find/inspect tools to discover its frame. A saved /figlens open selection is optional.');
    const source = await this.source(selection.file);
    const snapshot = await fileHash(source.absolute);
    const frame = text(selection.frame, 'frame', 200);
    if (this.cached?.file === source.file && this.cached.frame === frame && this.cached.snapshot === snapshot) return this.cached;
    const doc = await this.run(['nodes', source.absolute, frame], signal);
    if (!doc.document?.id) throw new Error('figlens returned no document. Check the frame id and installed figlens version.');
    if (await fileHash(source.absolute) !== snapshot) throw new Error('Design changed while reading; retry after the save finishes.');
    const result = {
      ...source, frame: doc.document.id, name: doc.document.name, snapshot,
      instances: instances(doc.document).map(node => summarizeInstance(node, doc)),
    };
    this.cached = result;
    return result;
  }

  async select(file, frame, signal) {
    const design = await this.design({ file, frame }, signal);
    await this.mutate(state => { state.active = { file: design.file, frame: design.frame }; });
    return design;
  }

  async close() {
    await this.mutate(state => { state.active = null; });
  }

  async selection(options = {}) {
    if (options.file !== undefined || options.frame !== undefined) {
      if (!options.file || !options.frame) throw new Error('Supply file and frame together.');
      return { file: options.file, frame: options.frame };
    }
    return (await this.read()).state.active;
  }

  async codeFiles(target, dependencies = []) {
    text(target.path, 'target.path', 4096);
    text(target.export, 'target.export', 200);
    text(target.import, 'target.import');
    text(target.example, 'target.example');
    strings(dependencies, 'dependencies');
    const root = await realpath(this.root);
    const files = {};
    for (const file of new Set([target.path, ...dependencies])) {
      if (path.isAbsolute(file)) throw new Error('Code evidence paths must be relative to the project.');
      const absolute = await realpath(path.resolve(root, file));
      const relative = path.relative(root, absolute);
      if (!relative || relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) throw new Error('Code evidence must stay inside the project.');
      files[relative.split(path.sep).join('/')] = await fileHash(absolute);
    }
    return files;
  }

  async freshness(entry) {
    try {
      const files = await this.codeFiles(entry.target, Object.keys(entry.files));
      return digest(files) === digest(entry.files) ? [] : ['Recorded code files changed.'];
    } catch (error) { return [`Recorded code evidence is unavailable: ${error.message}`]; }
  }

  async bindings(options = {}, signal) {
    const { state } = await this.read();
    const design = await this.design(await this.selection(options), signal);
    const offset = options.offset ?? 0;
    const limit = options.limit ?? 20;
    if (!Number.isInteger(offset) || offset < 0 || !Number.isInteger(limit) || limit < 1 || limit > 50) throw new Error('offset must be non-negative; limit must be 1-50.');
    const items = [];
    let bytes = 0;
    for (const instance of design.instances.slice(offset, offset + limit)) {
      const related = state.mappings.filter(entry => entry.source.file === design.file &&
        (entry.source.componentId === instance.componentId || (instance.componentSetId && entry.source.componentSetId === instance.componentSetId)) &&
        entry.status !== 'skipped' && entry.status !== 'superseded');
      const exact = related.filter(entry => entry.source.signature === instance.signature);
      const approved = [];
      const stale = [];
      for (const entry of exact.filter(entry => entry.status === 'approved')) {
        if (hasTextConflict(entry) && entry.source.instance !== instance.id) continue;
        const reasons = await this.freshness(entry);
        // A conflict resolution belongs to the exact observed instance and text, not every sibling.
        if ((instance.issues.length && !hasTextConflict(entry)) || (hasTextConflict(entry) && entry.source.textHash !== digest({ properties: instance.properties, texts: instance.texts }))) {
          reasons.push('Current text conflict has not been reviewed for this instance.');
        }
        if (reasons.length) stale.push({ id: entry.id, reasons });
        else approved.push({ id: entry.id, target: entry.target, rationale: entry.rationale, checks: entry.checks, resolution: entry.resolution });
      }
      const pending = exact.filter(entry => entry.status === 'pending' && entry.source.instance === instance.id).map(entry => ({ id: entry.id, target: entry.target, issues: entry.issues }));
      const item = {
        ...instance, status: approved.length === 1 ? 'approved' : approved.length > 1 ? 'ambiguous' : pending.length ? 'pending' : stale.length ? 'stale' : 'unmatched',
        approved, pending, stale,
        needsReview: related.length > 0 && exact.length === 0 ? ['Unreviewed component variant or structure.'] : [],
      };
      const size = Buffer.byteLength(JSON.stringify(item));
      if (items.length && bytes + size > MAX_RESULT_BYTES) break;
      // A single large instance still needs a bounded result; full design data is available through MCP.
      if (size > MAX_RESULT_BYTES) {
        items.push({ id: instance.id, name: instance.name, status: 'needs-inspection', reason: 'Instance details exceed the result budget; inspect this node with figlens MCP.' });
      } else items.push(item);
      bytes += size;
    }
    return {
      file: design.file, absoluteFile: design.absolute, frame: design.frame, name: design.name, snapshot: design.snapshot,
      total: design.instances.length, offset, nextOffset: offset + items.length < design.instances.length ? offset + items.length : null,
      items, guide: this.guide,
      verification: 'Approval records a human mapping decision. Checks are agent-reported; hashes check recorded files only, not import correctness, transitive dependencies or visual equivalence. Use current instance text, not example literals.',
    };
  }

  async propose(options, signal) {
    if (!Array.isArray(options.proposals) || options.proposals.length < 1 || options.proposals.length > 20) throw new Error('Submit 1-20 proposals at a time.');
    const design = await this.design(await this.selection(options), signal);
    const entries = [];
    for (const proposal of options.proposals) {
      const instance = design.instances.find(node => node.id === proposal.instance);
      if (!instance?.componentId) throw new Error(`Instance ${proposal.instance} is not a visible, component-backed instance in this frame.`);
      const target = proposal.target;
      const files = await this.codeFiles(target, proposal.dependencies);
      entries.push({
        id: randomUUID(), status: 'pending',
        source: {
          file: design.file, frame: design.frame, instance: instance.id, name: instance.name,
          componentId: instance.componentId, componentSetId: instance.componentSetId, componentKey: instance.componentKey,
          signature: instance.signature, snapshot: design.snapshot,
          textHash: digest({ properties: instance.properties, texts: instance.texts }),
          properties: instance.properties, texts: instance.texts,
        },
        target: { path: target.path, export: target.export, import: target.import, example: target.example }, files,
        rationale: text(proposal.rationale, 'rationale', 2000), checks: strings(proposal.checks ?? [], 'checks'),
        issues: [...instance.issues, ...strings(proposal.issues ?? [], 'issues', 10)],
      });
    }
    return this.mutate(state => {
      const ids = new Set(entries.map(entry => entry.source.instance));
      if (ids.size !== entries.length) throw new Error('Submit only one proposal per instance in each call.');
      state.mappings = state.mappings.filter(entry => !(entry.status === 'pending' && entry.source.file === design.file && entry.source.frame === design.frame && ids.has(entry.source.instance)));
      state.mappings.push(...entries);
      return { file: design.file, frame: design.frame, pending: entries.map(entry => ({ id: entry.id, instance: entry.source.instance, issues: entry.issues })), next: 'Run /figlens review to accept, edit or skip these proposals. No proposal is approved automatically.' };
    });
  }

  async pending() {
    const { state } = await this.read();
    return state.mappings.filter(entry => entry.status === 'pending');
  }

  async review(id, action, edit = {}, signal) {
    if (!['approve', 'skip'].includes(action)) throw new Error('Review action must be approve or skip.');
    return this.mutate(async state => {
      const entry = state.mappings.find(entry => entry.id === id && entry.status === 'pending');
      if (!entry) throw new Error('Proposal changed or was already reviewed; reload the review list.');
      if (action === 'skip') { entry.status = 'skipped'; return entry; }
      const design = await this.design({ file: entry.source.file, frame: entry.source.frame }, signal);
      const instance = design.instances.find(node => node.id === entry.source.instance);
      if (!instance || instance.signature !== entry.source.signature || digest({ properties: instance.properties, texts: instance.texts }) !== entry.source.textHash) throw new Error('Design evidence changed since proposal; ask the agent to submit a new proposal.');
      if ((await this.freshness(entry)).length) throw new Error('Code evidence changed since proposal; ask the agent to submit a new proposal.');
      if (edit.target) {
        entry.target = edit.target;
        entry.files = await this.codeFiles(entry.target, Object.keys(entry.files));
        entry.checks = [];
      }
      if (entry.issues.length) entry.resolution = text(edit.resolution, 'Review resolution', 2000);
      entry.status = 'approved';
      entry.approvedAt = new Date().toISOString();
      entry.source.snapshot = design.snapshot;
      for (const other of state.mappings) {
        if (other.id !== entry.id && other.status === 'approved' && other.source.file === entry.source.file && other.source.signature === entry.source.signature &&
          (hasTextConflict(other) === hasTextConflict(entry)) && (!hasTextConflict(entry) || other.source.instance === entry.source.instance)) other.status = 'superseded';
      }
      return entry;
    });
  }
}
