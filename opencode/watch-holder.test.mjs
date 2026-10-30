import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import test from 'node:test';
import { createHolder } from './watch-holder.mjs';

// A stand-in for one `figlens watch run` child: `script` drives it as the real one would.
function child(script) {
  const c = new EventEmitter();
  Object.assign(c, { pid: 1, stdout: new PassThrough(), stderr: new PassThrough(), stdin: new PassThrough(), exitCode: null, signals: [] });
  c.exit = code => { if (c.exitCode === null) { c.exitCode = code; c.stdout.end(); c.emit('exit', code); setImmediate(() => c.emit('close', code)); } };
  c.kill = signal => { c.signals.push(signal); if (script.kill) script.kill(c, signal); else if (!script.ignores?.includes(signal)) c.exit(null); };
  c.say = value => c.stdout.write(`${JSON.stringify(value)}\n`);
  c.stdin.on('finish', () => script.onStdinEnd?.(c));
  queueMicrotask(() => script.start(c));
  return c;
}
const active = { workspaceId: 'ws_1', leaseId: 'l1', sourceId: 's', scope: '0:0', state: 'active' };

test('a started lease is held until stop closes stdin, which ends it released', async () => {
  let spawned;
  const holder = createHolder(args => (spawned = { args, c: child({
    start: c => c.say(active),
    onStdinEnd: c => { c.say({ leaseId: 'l1', state: 'ended', reason: 'released' }); c.exit(0); },
  }) }).c);
  const reply = await holder.start(['--id', 'ws_1', '--scope', '0:0']);
  const held = holder.leases;
  const end = await holder.stop('l1');
  assert.deepEqual([spawned.args, reply.state, held, end.reason, holder.leases, spawned.c.signals],
    [['watch', 'run', '--id', 'ws_1', '--scope', '0:0'], 'active', ['l1'], 'released', [], []]);
});

test('a lease ended elsewhere is dropped and stop reports nothing held', async () => {
  let c;
  const holder = createHolder(() => (c = child({ start: c => c.say(active) })));
  await holder.start([]);
  c.say({ leaseId: 'l1', state: 'ended', reason: 'untracked' });
  c.exit(0);
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual([holder.leases, await holder.stop('l1')], [[], null]);
});

test('a replayed ended lease is returned as is and nothing is held', async () => {
  const ended = { leaseId: 'l0', state: 'ended', reason: 'stopped', nextStep: {} };
  const holder = createHolder(() => child({ start: c => { c.say(ended); c.exit(1); } }));
  assert.deepEqual([await holder.start([]), holder.leases], [ended, []]);
});

test('a start refused before any lease fails with figlens\'s message', async () => {
  const holder = createHolder(() => child({ start: c => { c.stderr.write('figlens: scope 0:0 is not tracked\n'); c.stderr.end(); setImmediate(() => c.exit(1)); } }));
  await assert.rejects(holder.start([]), /^Error: scope 0:0 is not tracked$/);
});

test('no start reply in time closes stdin and kills only a child that will not exit', async () => {
  let c;
  const holder = createHolder(() => (c = child({ start: () => {} })), { firstLineMs: 10, exitMs: 10 });
  await assert.rejects(holder.start([]), /no start reply in time/);
  assert.deepEqual([c.stdin.writableEnded, c.signals], [true, ['SIGTERM']]);
});

test('stopAll releases every held lease', async () => {
  let n = 0;
  const holder = createHolder(() => {
    const id = `l${++n}`;
    return child({ start: c => c.say({ ...active, leaseId: id }), onStdinEnd: c => c.exit(0) });
  });
  await holder.start([]);
  await holder.start([]);
  await holder.stopAll();
  assert.deepEqual(holder.leases, []);
});

test('a malformed start reply stops the child it spawned', async () => {
  let c;
  const holder = createHolder(() => (c = child({ start: c => c.stdout.write('not json\n'), onStdinEnd: c => c.exit(1) })));
  await assert.rejects(holder.start([]), SyntaxError);
  assert.deepEqual([c.exitCode, holder.leases], [1, []]);
});

test('a spawn that only emits error is refused, not left pending', async () => {
  const holder = createHolder(() => child({ start: c => { c.pid = undefined; c.emit('error', new Error('ENOENT')); } }), { exitMs: 10 });
  await assert.rejects(holder.start([]), /could not start: ENOENT/);
});

test('cleanup while a start awaits its reply returns only after that child closed', async () => {
  let c, closed = false;
  const holder = createHolder(() => (c = child({ start: c => c.once('close', () => { closed = true; }),
    onStdinEnd: c => setTimeout(() => c.exit(0), 20) })));
  const starting = holder.start([]);
  await new Promise(resolve => setImmediate(resolve));
  await holder.stopAll();
  const atReturn = closed;
  await assert.rejects(starting);
  assert.deepEqual([atReturn, holder.leases], [true, []]);
});

test('an error on SIGTERM is not taken as exit: the child is killed and stays owned until it closes', async () => {
  let c;
  const holder = createHolder(() => (c = child({ start: c => c.say(active),
    kill: (c, signal) => signal === 'SIGTERM' ? c.emit('error', new Error('EPERM')) : c.exit(null) })), { exitMs: 10 });
  await holder.start([]);
  const end = await holder.stop('l1');
  assert.deepEqual([c.signals, end.reason, holder.leases], [['SIGTERM', 'SIGKILL'], 'release_unconfirmed', []]);
});

test('a child that never closes keeps its lease held, with no TTL promised', async () => {
  const holder = createHolder(() => child({ start: c => c.say(active), kill: () => {} }), { exitMs: 10 });
  await holder.start([]);
  const end = await holder.stop('l1');
  assert.deepEqual([end.reason, /TTL/.test(end.recovery), holder.leases], ['holder_not_exited', false, ['l1']]);
});

test('a child ignoring stdin and SIGTERM is killed and its release reported unconfirmed', async () => {
  let c;
  const holder = createHolder(() => (c = child({ start: c => c.say(active), ignores: ['SIGTERM'] })), { exitMs: 10 });
  await holder.start([]);
  const end = await holder.stop('l1');
  assert.deepEqual([c.signals, end.state, end.reason], [['SIGTERM', 'SIGKILL'], 'unknown', 'release_unconfirmed']);
});

test('a child that exits without an end reply is not reported released', async () => {
  const holder = createHolder(() => child({ start: c => c.say(active), onStdinEnd: c => c.exit(0) }), { exitMs: 10 });
  await holder.start([]);
  assert.equal((await holder.stop('l1')).reason, 'release_unconfirmed');
});

test('cancelling a start before its reply releases and awaits its child, and holds nothing', async () => {
  let c, ended = false;
  const holder = createHolder(() => (c = child({ start: () => {}, onStdinEnd: c => { ended = true; c.exit(0); } })));
  const controller = new AbortController();
  const starting = holder.start([], controller.signal);
  await new Promise(resolve => setImmediate(resolve));
  controller.abort();
  await assert.rejects(starting, { name: 'AbortError' });
  assert.deepEqual([ended, c.exitCode, holder.leases], [true, 0, []]);
});

test('a requestId replay shares its start: one child, a changed watch refused, and a replay cancel leaves it running', async () => {
  let spawned = 0;
  let reply;
  const holder = createHolder(() => (spawned++, child({ start: c => { reply = () => c.say(active); } })));
  const args = ['--id', 'ws_1', '--scope', '0:0', '--request-id', 'r'];
  const first = holder.start(args);
  const replay = new AbortController();
  const cancelled = holder.start(args, replay.signal);
  await new Promise(resolve => setImmediate(resolve));
  replay.abort(new Error('replay cancelled'));
  await assert.rejects(cancelled, /replay cancelled/);
  reply();
  const leases = [await first, await holder.start(args)];
  await assert.rejects(holder.start(['--id', 'ws_1', '--scope', '0:1', '--request-id', 'r']), /names a different watch/);
  assert.deepEqual([spawned, leases, holder.leases], [1, [active, active], ['l1']]);
});

test('starts past the limit fail without spawning, and a confirmed exit frees a place', async () => {
  let spawned = 0;
  const holder = createHolder(() => { const leaseId = `l${++spawned}`; return child({
    start: c => c.say({ ...active, leaseId }),
    onStdinEnd: c => c.exit(0),
  }); }, { limit: 2 });
  const results = await Promise.allSettled([holder.start([]), holder.start([]), holder.start([])]);
  const refused = results.filter(result => result.status === 'rejected').map(result => result.reason.message);
  await holder.stop('l1');
  await holder.start([]);
  assert.deepEqual([refused.length, /already holds or is starting 2/.test(refused[0]), spawned], [1, true, 3]);
});

test('a replay whose shared start its owner cancelled starts afresh', async () => {
  let spawned = 0;
  const holder = createHolder(() => { spawned++; return child({ start: c => { if (spawned > 1) c.say(active); }, onStdinEnd: c => c.exit(0) }); });
  const args = ['--request-id', 'r'];
  const owner = new AbortController();
  const first = holder.start(args, owner.signal);
  const replay = holder.start(args);
  await new Promise(resolve => setImmediate(resolve));
  owner.abort(new Error('owner cancelled'));
  await assert.rejects(first, /owner cancelled/);
  assert.deepEqual([await replay, spawned], [active, 2]);
});

test('one requestId in two workspaces is two watches, as figlens keeps it per workspace', async () => {
  let spawned = 0;
  const holder = createHolder(() => { const leaseId = `l${++spawned}`; return child({ start: c => c.say({ ...active, leaseId }) }); });
  const [a, b] = [await holder.start(['--id', 'ws_1', '--scope', '0:0', '--request-id', 'r']), await holder.start(['--id', 'ws_2', '--scope', '0:0', '--request-id', 'r'])];
  assert.deepEqual([a.leaseId, b.leaseId, spawned], ['l1', 'l2', 2]);
});
