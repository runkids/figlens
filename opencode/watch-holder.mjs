import { createInterface } from 'node:readline';

// Holds each `figlens watch run` lease this host started: one owned child per lease, whose first
// stdout line is the start reply and whose stdin EOF releases it. The runner it starts is detached
// and shared, so only the child itself is ever killed, and only when it outlives `exitMs`.
export function createHolder(spawnRun, { firstLineMs = 30000, exitMs = 10000 } = {}) {
  const held = new Map();

  function watch(child) {
    const lines = createInterface({ input: child.stdout });
    const next = [];
    const waiting = [];
    lines.on('line', line => (waiting.length ? waiting.shift()(line) : next.push(line)));
    let stderr = '';
    child.stderr?.on('data', chunk => { stderr += chunk; });
    const exited = new Promise(resolve => {
      if (child.exitCode !== null && child.exitCode !== undefined) resolve(child.exitCode);
      else child.once('exit', code => resolve(code));
    });
    const line = () => next.length ? Promise.resolve(next.shift())
      : new Promise(resolve => { waiting.push(resolve); exited.then(() => setImmediate(() => resolve(next.shift() ?? null))); });
    return { line, exited, stderr: () => stderr };
  }

  async function ended(child, exited) {
    const timer = new Promise(resolve => { const t = setTimeout(resolve, exitMs, 'timeout'); t.unref?.(); });
    if (await Promise.race([exited, timer]) === 'timeout') {
      child.kill();
      await exited;
    }
  }

  async function start(args) {
    const child = spawnRun(['watch', 'run', ...args]);
    const out = watch(child);
    const timer = new Promise(resolve => { const t = setTimeout(resolve, firstLineMs, 'timeout'); t.unref?.(); });
    const first = await Promise.race([out.line(), timer]);
    if (first === 'timeout' || first === null) {
      child.stdin.end();
      await ended(child, out.exited);
      const message = out.stderr().split('\n').find(line => line.startsWith('figlens: '));
      throw new Error(first === 'timeout' ? 'figlens watch run gave no start reply in time'
        : (message ?? `figlens watch run exited ${await out.exited}`).replace(/^figlens: /, ''));
    }
    const reply = JSON.parse(first);
    if (reply.state !== 'active') {
      await ended(child, out.exited);
      return reply;
    }
    // Ended elsewhere (stopped, untracked, removed, expired): its end line, then exit.
    const end = out.line().then(line => line && JSON.parse(line));
    const lease = { child, exited: out.exited, end };
    held.set(reply.leaseId, lease);
    out.exited.then(() => { if (held.get(reply.leaseId) === lease) held.delete(reply.leaseId); });
    return reply;
  }

  // The end reply of a lease this host holds, after its child released it and exited.
  async function stop(leaseId) {
    const lease = held.get(leaseId);
    if (!lease) return null;
    held.delete(leaseId);
    lease.child.stdin.end();
    await ended(lease.child, lease.exited);
    return (await lease.end) ?? { leaseId, state: 'ended', reason: 'released' };
  }

  async function stopAll() {
    await Promise.all([...held.keys()].map(stop));
  }

  return { start, stop, stopAll, get leases() { return [...held.keys()]; } };
}
