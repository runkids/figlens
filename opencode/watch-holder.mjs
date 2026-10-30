import { createInterface } from 'node:readline';

// Holds each `figlens watch run` lease this host started: one owned child per lease, whose first
// stdout line is the start reply and whose stdin EOF releases it. The runner it starts is detached
// and shared, so only the child itself is ever signalled, and only when it outlives `exitMs`.
const STDERR = 64 * 1024;

// Each child is its own process to figlens, so its per-process cap (16 leases) bounds none of them;
// this host bounds its own children to the same count, starting or held, until each confirmed exit.
const LIMIT = 16;

// `group`: each child leads its own process group (spawned detached), so a signal reaches the native
// `watch run` under the npm launcher too; figlens starts the shared runner in a group of its own.
export function createHolder(spawnRun, { firstLineMs = 30000, exitMs = 10000, limit = LIMIT, group = false } = {}) {
  const held = new Map();
  // requestId -> its start, while that start is pending or its lease is held here. Once released,
  // a replay reaches figlens again, which reports the ended lease and never restarts it.
  const requests = new Map();
  const owned = new Set();  // every child's watch, from spawn until it exits
  let closing = false;

  function watch(child) {
    const next = [];
    const waiting = [];
    let stderr = '';
    const exited = new Promise(resolve => {
      // A spawned child's error (such as a failed kill) is not an exit: only close is. Only a
      // spawn that never made a process resolves on error. The listener stays, so no later
      // error can crash the host.
      child.on('error', error => { if (child.pid === undefined) resolve({ error }); });
      // close, not exit: stdout is drained by then, so an end line is never lost.
      child.once('close', code => resolve({ code }));
    });
    let done = false;
    exited.then(() => { done = true; while (waiting.length) waiting.shift()(null); });
    // Only a start and an end line are read, so a few unread lines are kept and the rest dropped.
    if (child.stdout) createInterface({ input: child.stdout }).on('line', line => (waiting.length ? waiting.shift()(line) : next.length < 4 && next.push(line)));
    // Only its start failure is read from stderr, so no more than its head is kept.
    child.stderr?.on('data', chunk => { if (stderr.length < STDERR) stderr = (stderr + chunk).slice(0, STDERR); });
    const line = () => next.length ? Promise.resolve(next.shift())
      : done ? Promise.resolve(null) : new Promise(resolve => waiting.push(resolve));
    const out = { child, line, exited, stderr: () => stderr };
    owned.add(out);
    exited.then(() => owned.delete(out));
    return out;
  }

  const within = (promise, ms) => Promise.race([promise, new Promise(resolve => { const t = setTimeout(resolve, ms, 'timeout'); t.unref?.(); })]);

  // Closes stdin, then signals only this child, each step bounded. True once it exited.
  async function shutdown(out) {
    try { out.child.stdin?.end(); } catch {}
    for (const signal of [null, 'SIGTERM', 'SIGKILL']) {
      if (signal) try { group ? process.kill(-out.child.pid, signal) : out.child.kill(signal); } catch {}
      if (await within(out.exited, exitMs) !== 'timeout') return true;
    }
    return false;
  }

  function unconfirmed(leaseId, exited) {
    return exited
      ? { leaseId, state: 'unknown', reason: 'release_unconfirmed',
        recovery: 'the lease lapses within its 30 s TTL; check watch status before starting it again' }
      // Its child may still renew the lease, so no TTL is promised; it stays held for a retry.
      : { leaseId, state: 'unknown', reason: 'holder_not_exited',
        recovery: 'its holder process has not exited and may still renew the lease; stop it again, or end that process, then check watch status' };
  }

  // A requestId this session already started replays that start instead of spawning again: figlens
  // refuses a live lease's requestId from any other process, this host's own next child included.
  async function start(args, signal) {
    if (closing) throw new Error('figlens watch holder is shutting down');
    signal?.throwIfAborted();
    const at = args.indexOf('--request-id');
    if (at < 0) return begin(args, signal);
    // figlens keeps a requestId per workspace, so it is keyed with the arguments that pick one.
    const picks = args.filter((arg, i) => !['--source', '--scope'].includes(arg) && !['--source', '--scope'].includes(args[i - 1]));
    const id = JSON.stringify(picks), key = JSON.stringify(args);
    let request = requests.get(id);
    if (request && request.key !== key) throw new Error(`requestId ${args[at + 1]} names a different watch; start with a new requestId`);
    if (!request) {
      // The first start owns the child, so its cancel waits for begin to release it.
      request = { key, signal, reply: begin(args, signal) };
      requests.set(id, request);
      request.reply.then(reply => { if (held.has(reply.leaseId)) held.get(reply.leaseId).request = id; else requests.delete(id); },
        () => requests.delete(id));
      return request.reply;
    }
    // A replay's own cancel leaves the start it shares running.
    let cancel;
    const cancelled = new Promise((_, reject) => { cancel = () => reject(signal.reason); signal?.addEventListener('abort', cancel, { once: true }); });
    try { return await Promise.race([request.reply, cancelled]); }
    catch (error) {
      // The start it shared was cancelled by its own caller, not this one: start afresh.
      if (request.signal?.aborted && !signal?.aborted) return start(args, signal);
      throw error;
    } finally { signal?.removeEventListener('abort', cancel); }
  }

  // `signal` cancels a start still waiting for its reply: its child is released and awaited.
  async function begin(args, signal) {
    if (owned.size >= limit) throw new Error(`this session already holds or is starting ${limit} watch leases; stop one with figlens_watch stop first`);
    let out;
    try { out = watch(spawnRun(['watch', 'run', ...args])); }
    catch (error) { throw new Error(`figlens watch run could not start: ${error.message}`); }
    let cancel;
    const cancelled = new Promise(resolve => { cancel = () => resolve('cancelled'); signal?.addEventListener('abort', cancel, { once: true }); });
    const first = await Promise.race([within(out.line(), firstLineMs), cancelled]);
    signal?.removeEventListener('abort', cancel);
    let reply;
    try {
      if (first === 'cancelled') throw signal.reason;
      if (first === 'timeout') throw new Error('figlens watch run gave no start reply in time');
      if (first === null) {
        const { error, code } = await out.exited;
        const message = out.stderr().split('\n').find(line => line.startsWith('figlens: '));
        throw new Error(error ? `figlens watch run could not start: ${error.message}`
          : (message ?? `figlens watch run exited ${code}`).replace(/^figlens: /, ''));
      }
      reply = JSON.parse(first);
      if (closing && reply.state === 'active') throw new Error('figlens watch holder is shutting down');
      signal?.throwIfAborted();
    } catch (error) {
      await shutdown(out);
      throw error;
    }
    if (reply.state !== 'active') {
      await shutdown(out);
      return reply;
    }
    // Ended elsewhere (stopped, untracked, removed, expired): its end line, then exit.
    const end = out.line().then(line => { try { return line && JSON.parse(line); } catch { return null; } });
    const lease = { out, end };
    held.set(reply.leaseId, lease);
    out.exited.then(() => drop(reply.leaseId, lease));
    return reply;
  }

  function drop(leaseId, lease) {
    if (held.get(leaseId) !== lease) return;
    held.delete(leaseId);
    if (lease.request) requests.delete(lease.request);
  }

  // The end reply of a lease this host holds, once its child released it; unconfirmed when the
  // child exited without saying so or had to be signalled.
  async function stop(leaseId) {
    const lease = held.get(leaseId);
    if (!lease) return null;
    const exited = await shutdown(lease.out);
    if (exited) drop(leaseId, lease);
    const end = exited ? await within(lease.end, exitMs) : null;
    return end && end !== 'timeout' ? end : unconfirmed(leaseId, exited);
  }

  // Releases every lease and stops every child still starting; no start admits a lease afterwards.
  // Returns each held lease's end reply, so an unconfirmed release is never silent.
  async function stopAll() {
    closing = true;
    const leases = [...held.values()].map(l => l.out);
    const starting = Promise.all([...owned].filter(out => !leases.includes(out)).map(shutdown));
    const ends = await Promise.all([...held.keys()].map(stop));
    await starting;
    return ends;
  }

  return { start, stop, stopAll, get leases() { return [...held.keys()]; } };
}
