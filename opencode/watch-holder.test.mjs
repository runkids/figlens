import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import test from 'node:test';
import { createHolder } from './watch-holder.mjs';

// A stand-in for one `figlens watch run` child: `script` drives it as the real one would.
function child(script) {
  const c = new EventEmitter();
  Object.assign(c, { stdout: new PassThrough(), stderr: new PassThrough(), stdin: new PassThrough(), exitCode: null, killed: false });
  c.exit = code => { if (c.exitCode === null) { c.exitCode = code; c.stdout.end(); c.emit('exit', code); } };
  c.kill = () => { c.killed = true; c.exit(null); };
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
  assert.deepEqual([spawned.args, reply.state, held, end.reason, holder.leases, spawned.c.killed],
    [['watch', 'run', '--id', 'ws_1', '--scope', '0:0'], 'active', ['l1'], 'released', [], false]);
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
  assert.deepEqual([c.stdin.writableEnded, c.killed], [true, true]);
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
