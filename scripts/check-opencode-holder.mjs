#!/usr/bin/env node
// The installed OpenCode adapter's CLI-path watch holders on this OS, with no host: each lease is held by
// the cached native `watch run` itself (never the npm launcher, which a stop could not end on Windows),
// closing one session ends only its holder while the shared runner keeps serving the other, and dispose
// releases the rest.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { promisify } from 'node:util';

const [installed, binary, fixture] = process.argv.slice(2);
assert.ok(installed && binary && fixture, 'Usage: node scripts/check-opencode-holder.mjs <installed-package> <cached-binary> <design.fig>');
const execute = promisify(execFile);
const base = await mkdtemp(path.join(tmpdir(), 'figlens-holder-'));
for (const kind of ['CONFIG', 'STATE', 'DATA']) process.env[`XDG_${kind}_HOME`] = path.join(base, kind.toLowerCase());
delete process.env.FIGLENS_BIN;
const cli = async (...args) => JSON.parse((await execute(binary, args, { cwd: base })).stdout);
await cli('index', 'prepare', fixture, '--scope', '0:0');
await cli('index', 'track', '--file', fixture, '--scope', '0:0');
const { workspaceId } = await cli('index', 'status', '--file', fixture);

// This process's own children running `watch run`, from the OS process list.
async function holders() {
  const rows = process.platform === 'win32'
    ? [JSON.parse((await execute('powershell.exe', ['-NoProfile', '-Command',
        `Get-CimInstance Win32_Process -Filter "ParentProcessId=${process.pid}" | Select-Object ProcessId,CommandLine | ConvertTo-Json -Compress`])).stdout || '[]')]
        .flat().map(row => ({ pid: row.ProcessId, command: row.CommandLine ?? '' }))
    : (await execute('ps', ['-axo', 'pid=,ppid=,command='])).stdout.split('\n').map(line => line.trim().split(/\s+/))
        .filter(([, ppid]) => Number(ppid) === process.pid).map(([pid, , ...command]) => ({ pid: Number(pid), command: command.join(' ') }));
  return rows.filter(row => / watch run /.test(` ${row.command} `));
}
const until = async (what, done) => {
  for (const start = Date.now(); !(await done()); await new Promise(resolve => setTimeout(resolve, 100)))
    assert.ok(Date.now() - start < 20000, `timed out waiting for ${what}`);
};
const leases = async () => (await cli('watch', 'status', '--id', workspaceId)).leases;

const { createAdapter } = await import(pathToFileURL(path.join(installed, 'opencode/adapter.mjs')).href);
const ctx = { location: { directory: base },
  session: { get: async () => ({ location: { directory: base } }), wait: async () => {}, prompt: async () => {} },
  mcp: { list: async () => ({ data: [] }) }, tool: { list: async () => [] }, permission: { assert: async () => {} } };
const adapter = createAdapter(ctx);
try {
  const start = async session => adapter.cli(await adapter.scope(session), 'watch', { action: 'start', workspaceId, scope: '0:0' });
  const [a, b] = [await start('a'), await start('b')];
  assert.deepEqual([a.state, b.state], ['active', 'active']);
  const held = await holders();
  assert.equal(held.length, 2, JSON.stringify(held));
  for (const { command } of held) assert.ok(!command.includes('figlens.js') && command.includes(path.basename(binary)), `a holder is not the native binary: ${command}`);
  await adapter.close('a');
  await until("a's holder to exit", async () => (await holders()).length === 1);
  const after = Object.fromEntries((await leases()).map(lease => [lease.leaseId, [lease.state, lease.reason ?? null]]));
  assert.deepEqual([after[a.leaseId], after[b.leaseId]], [['ended', 'released'], ['active', null]]);
  assert.equal((await cli('watch', 'status', '--id', workspaceId)).runner.state, 'running', 'closing one session stopped the shared runner');
  await adapter.dispose();
  await until('every holder to exit', async () => (await holders()).length === 0);
  const ended = (await leases()).find(lease => lease.leaseId === b.leaseId);
  assert.deepEqual([ended.state, ended.reason], ['ended', 'released']);
  console.log(JSON.stringify({ platform: process.platform, nativeHolders: held.length, sessionCloseKeptRunner: true, disposeReleased: true }));
} finally {
  await adapter.dispose();
}
