#!/usr/bin/env python3
"""Native #88 performance acceptance of one packaged candidate on Linux, macOS or Windows.

Runs the exact candidate archive for this OS against the original synthetic workload from
`mapping-fixture --scale` (one source plus a copy with one text edit), using only the standard
library. Prints counts, timings and bytes, never names or text. Every attempt is kept. Cold means
a fresh process: the OS file cache stays warm, CI cannot drop it.

The owned process tree is every process running the candidate executable; on a CI runner nothing
else runs it. RSS is sampled every 100 ms, so a shorter spike can be missed; each process's OS
peak is used where it is exact (Linux VmHWM, Windows PeakWorkingSetSize).
"""
import argparse
from collections import Counter
import ctypes
import hashlib
from importlib import import_module
import json
import os
from pathlib import Path
import platform
import re
import shutil
import sqlite3
from stat import S_ISREG
import statistics
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import urllib.parse
import urllib.request
import zipfile
from installation import environment, run, stop_process

search = import_module('measure-search')
summarize = import_module('measure-baseline').summarize
TARGETS = {'linux': 'x86_64-unknown-linux-musl', 'darwin': 'aarch64-apple-darwin', 'win32': 'x86_64-pc-windows-gnu'}
AREA = '0:900000'  # the generator's first top-level area: the first prepared scope, under 10,000 records
MiB = 1 << 20
# #88 p95 targets (ms) and the samples each needs before a pass is claimed.
GATES = {'warmSearch': (200, 100), 'warmUsages': (200, 100), 'viewerWarmUsages': (200, 100),
         'indexedColdSearch': (1000, 20), 'prepare': (3000, 20), 'build': (10000, 20),
         'save': (2000, 20), 'cancel': (500, 20)}
# Resource limits root locked on 2026-10-30 before any measurement; never raised after a result.
LIMITS = {'mcpTreePeakRssBytes': 1024 * MiB, 'viewerTreePeakRssBytes': 1024 * MiB,
          'buildTreePeakRssBytes': 2048 * MiB, 'runnerPeakRssBytes': 768 * MiB, 'workerPeakRssBytes': 1536 * MiB,
          'pollingCpuMsPer60s': 300, 'idleCpuMsPer60s': 600, 'idle8HoldersCpuMsPer60s': 1200,
          'databaseBytes': 2048 * MiB, 'walBytes': 96 * MiB}


def verdict(name, samples):
    target, need = GATES[name]
    result = summarize(samples) if samples else {'samplesMs': [], 'p95Ms': None}
    result.update(n=len(samples), targetMs=target, required=need,
                  status='insufficient' if len(samples) < need else 'pass' if result['p95Ms'] <= target else 'fail')
    return result


def within(value, limit, exact=True):
    """An inexact value is a lower bound: over the limit it fails, under it proves nothing."""
    return {'value': value, 'limit': limit, 'status': 'unmeasured' if value is None else 'fail' if value > limit
            else 'pass' if exact else 'lower_bound'}


# --- the candidate's processes: {pid: {ppid, rss, peak?, cpuMs, readBytes, writeBytes}} ---
# readBytes/writeBytes are logical (Linux rchar/wchar, Windows transfer counts, macOS logical
# writes; macOS has no logical read count). Linux and macOS also give physical disk bytes.

if sys.platform == 'darwin':
    libproc = ctypes.CDLL('/usr/lib/libproc.dylib')

    class Timebase(ctypes.Structure):
        _fields_ = [('numer', ctypes.c_uint32), ('denom', ctypes.c_uint32)]
    _base = Timebase()
    ctypes.CDLL(None).mach_timebase_info(ctypes.byref(_base))

    def table(exe):
        count = libproc.proc_listallpids(None, 0)
        pids = (ctypes.c_int * (count * 2))()
        count = libproc.proc_listallpids(pids, ctypes.sizeof(pids))
        rows, path = {}, ctypes.create_string_buffer(4096)
        for pid in pids[:count]:
            if libproc.proc_pidpath(pid, path, 4096) <= 0 or os.fsdecode(path.value) != exe:
                continue
            info, usage = ctypes.create_string_buffer(136), ctypes.create_string_buffer(296)
            # PROC_PIDTBSDINFO: pbi_ppid at byte 16. RUSAGE_INFO_V4: CPU times in mach ticks.
            if libproc.proc_pidinfo(pid, 3, 0, info, 136) != 136 or libproc.proc_pid_rusage(pid, 4, usage):
                continue
            u = lambda at: int.from_bytes(usage.raw[at:at + 8], 'little')
            rows[pid] = {'ppid': int.from_bytes(info.raw[16:20], 'little'), 'rss': u(64),
                         'cpuMs': (u(16) + u(24)) * _base.numer // _base.denom // 1_000_000,
                         'childCpuMs': (u(96) + u(104)) * _base.numer // _base.denom // 1_000_000,
                         'readBytes': None, 'writeBytes': u(232), 'physReadBytes': u(144), 'physWriteBytes': u(152)}
        return rows
elif os.name == 'nt':
    from ctypes import wintypes
    kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel32.OpenProcess.restype = wintypes.HANDLE

    class Entry(ctypes.Structure):
        _fields_ = [('size', wintypes.DWORD), ('usage', wintypes.DWORD), ('pid', wintypes.DWORD),
                    ('heap', ctypes.c_size_t), ('module', wintypes.DWORD), ('threads', wintypes.DWORD),
                    ('ppid', wintypes.DWORD), ('priority', ctypes.c_long), ('flags', wintypes.DWORD),
                    ('exe', ctypes.c_wchar * 260)]

    def table(exe):
        snap, entry, rows = wintypes.HANDLE(kernel32.CreateToolhelp32Snapshot(2, 0)), Entry(), {}
        entry.size = ctypes.sizeof(Entry)
        ok = kernel32.Process32FirstW(snap, ctypes.byref(entry))
        while ok:
            # PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ
            handle = entry.exe.lower() == os.path.basename(exe).lower() and kernel32.OpenProcess(0x1010, False, entry.pid)
            if handle:
                handle = wintypes.HANDLE(handle)
                path, size = ctypes.create_unicode_buffer(32768), wintypes.DWORD(32768)
                times, io = (ctypes.c_uint64 * 4)(), (ctypes.c_uint64 * 6)()
                if (kernel32.QueryFullProcessImageNameW(handle, 0, path, ctypes.byref(size))
                        and os.path.normcase(path.value) == os.path.normcase(exe)
                        and kernel32.GetProcessTimes(handle, *[ctypes.byref(times, 8 * i) for i in range(4)])
                        and kernel32.GetProcessIoCounters(handle, io)):
                    rss, peak = search.windows_memory(handle.value)
                    rows[entry.pid] = {'ppid': entry.ppid, 'rss': rss, 'peak': peak,
                                       'cpuMs': (times[2] + times[3]) // 10_000, 'childCpuMs': None,
                                       'readBytes': io[3], 'writeBytes': io[4]}
                kernel32.CloseHandle(handle)
            ok = kernel32.Process32NextW(snap, ctypes.byref(entry))
        kernel32.CloseHandle(snap)
        return rows

    class Job:
        """A job object without limits for the idle holders. Its accounting is cumulative over
        every process ever in it, ended ones included. It forbids breakaway, so a holder's
        launcher starts the runner `in_host_job`, inside it, and the runner's workers inherit it."""
        def __init__(self):
            kernel32.CreateJobObjectW.restype = wintypes.HANDLE
            kernel32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
            kernel32.QueryInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p,
                                                           wintypes.DWORD, ctypes.c_void_p]
            self.handle = kernel32.CreateJobObjectW(None, None)
            if not self.handle:
                raise ctypes.WinError(ctypes.get_last_error())

        def start(self, command, **options):
            """Start `command` suspended, put it in the job before it can start a child, then resume it."""
            process = subprocess.Popen(command, creationflags=0x4, **options)  # CREATE_SUSPENDED
            try:
                if not kernel32.AssignProcessToJobObject(self.handle, int(process._handle)):
                    raise ctypes.WinError(ctypes.get_last_error())
                resume = ctypes.WinDLL('ntdll').NtResumeProcess  # Popen keeps no thread handle
                resume.argtypes, resume.restype = [wintypes.HANDLE], ctypes.c_long
                if resume(int(process._handle)):
                    raise OSError('NtResumeProcess failed')
            except BaseException:
                process.kill()
                process.wait()
                raise
            return process

        def counters(self):
            """JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION: (cpuMs, readBytes, writeBytes, processes, active)."""
            info = (ctypes.c_uint64 * 12)()
            if not kernel32.QueryInformationJobObject(self.handle, 8, info, ctypes.sizeof(info), None):
                raise ctypes.WinError(ctypes.get_last_error())
            return (info[0] + info[1]) // 10_000, info[9], info[10], info[4] >> 32, info[5] & 0xFFFFFFFF

        def close(self):
            kernel32.CloseHandle(wintypes.HANDLE(self.handle))
else:
    def table(exe):
        rows, tick = {}, os.sysconf('SC_CLK_TCK')
        for pid in filter(str.isdigit, os.listdir('/proc')):
            try:
                if os.readlink(f'/proc/{pid}/exe') != exe:
                    continue
                stat = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
                status = {k: v.split()[0] for k, v in (l.split(':', 1) for l in Path(f'/proc/{pid}/status').read_text().splitlines() if l.split(':', 1)[1].strip())}
                io = {k: int(v) for k, v in (l.split(': ') for l in Path(f'/proc/{pid}/io').read_text().splitlines())}
                rows[int(pid)] = {'ppid': int(stat[1]), 'rss': int(status['VmRSS']) * 1024, 'peak': int(status['VmHWM']) * 1024,
                                  'cpuMs': (int(stat[11]) + int(stat[12])) * 1000 // tick,
                                  'childCpuMs': (int(stat[13]) + int(stat[14])) * 1000 // tick,
                                  'readBytes': io['rchar'], 'writeBytes': io['wchar'],
                                  'physReadBytes': io['read_bytes'], 'physWriteBytes': io['write_bytes']}
            except (OSError, ValueError, KeyError, IndexError):
                pass  # exited between listing and reading
        return rows


def roles(rows, holders):
    """(runners, workers) among `rows`. A holder starts a short-lived `__watch-launch`, which
    starts the detached runner (macOS reparents it at once, Windows keeps the launcher's pid);
    a runner starts the workers. The harness's own direct children are holders and CLI calls."""
    launchers = {p for p, r in rows.items() if r['ppid'] in holders}
    runners = {p for p, r in rows.items() if p not in holders | launchers and r['ppid'] != os.getpid()
               and (r['ppid'] in launchers or r['ppid'] not in rows)}
    return runners, {p for p, r in rows.items() if r['ppid'] in runners}


def sizes(paths):
    """(name, bytes) of the regular files among `paths`; a file removed meanwhile (a checkpointed
    WAL, a replaced temporary) is skipped."""
    files = []
    for path in paths:
        try:
            info = path.stat()
        except FileNotFoundError:
            continue
        if S_ISREG(info.st_mode):
            files.append((path.name, info.st_size))
    return files


class Tree:
    """While the block runs: peak RSS of all candidate processes together and of each one, and
    with `state`, the largest state-root, database and WAL sizes seen."""
    def __init__(self, exe, state=None):
        self.exe, self.state, self.done, self.error = exe, state, threading.Event(), None
        self.peak, self.procs, self.samples, self.disk = 0, {}, 0, {'stateBytes': 0, 'databaseBytes': 0, 'walBytes': 0}

    def loop(self):
        try:
            self.sample()
        except BaseException as error:  # re-raised by __exit__, so a dead sampler cannot pass silently
            self.error = error

    def sample(self):
        while not self.done.is_set():
            rows = table(self.exe)
            self.peak = max(self.peak, sum(r['rss'] for r in rows.values()))
            for pid, r in rows.items():
                seen = self.procs.setdefault(pid, {'ppid': r['ppid'], 'peak': 0})  # the parent at start
                seen['peak'] = max(seen['peak'], r['rss'], r.get('peak', 0))
            if self.state and self.samples % 10 == 0:
                files = sizes(self.state.rglob('*'))
                now = {'stateBytes': sum(s for _, s in files),
                       'databaseBytes': sum(s for n, s in files if n.endswith('.sqlite')),
                       'walBytes': sum(s for n, s in files if n.endswith('.sqlite-wal'))}
                self.disk = {k: max(v, now[k]) for k, v in self.disk.items()}
            self.samples += 1
            self.done.wait(0.1)

    def __enter__(self):
        self.thread = threading.Thread(target=self.loop, daemon=True)
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.done.set()
        self.thread.join()
        if self.error:
            raise self.error

    def roles(self, holders):
        """Peak RSS of the runners the holders started and of the workers those runners started."""
        runners, workers = roles(self.procs, holders)
        peak = lambda pids: max((self.procs[p]['peak'] for p in pids), default=None)
        return {'runners': len(runners), 'runnerPeakRss': within(peak(runners), LIMITS['runnerPeakRssBytes']),
                'workers': len(workers), 'workerPeakRss': within(peak(workers), LIMITS['workerPeakRssBytes'])}

    def result(self, limit, pids=None):
        """The limit applies to the tree, or with `pids` to the largest of those processes."""
        peak = self.peak if pids is None else max((self.procs[p]['peak'] for p in pids if p in self.procs), default=None)
        return {'treePeakSampledBytes': self.peak, 'samples': self.samples,
                'largestProcessPeakBytes': max((r['peak'] for r in self.procs.values()), default=None),
                'limited': 'tree' if pids is None else 'process', 'peakRss': within(peak, LIMITS[limit])}


def gone(pid):
    """A callable that reads True once process `pid` (None: no process) has exited."""
    if pid is None:
        return lambda: True
    if os.name == 'nt':
        handle = kernel32.OpenProcess(0x00100000, False, pid)  # SYNCHRONIZE
        return (lambda: True) if not handle else (lambda: kernel32.WaitForSingleObject(wintypes.HANDLE(handle), 0) == 0)

    def check():
        try:
            os.kill(pid, 0)
            return False
        except ProcessLookupError:
            return True
    return check


def classify(in_flight, job, final, timed_out):
    """One cancel attempt; only `cancelled_in_flight` enters the percentile."""
    if timed_out:
        return 'timed_out'
    if not job:
        return 'no_job'
    if final == 'succeeded':
        return 'completed_before_cancel'
    if not in_flight:
        return 'not_proven_in_flight'
    return 'cancelled_in_flight' if final == 'cancelled' else f'ended_{final}'


def settle(exe, limit=30):
    """Wait until no candidate process runs, so a later step starts its own runner; False if one
    is still running after `limit` seconds."""
    start = time.monotonic()
    while table(exe):
        if time.monotonic() - start > limit:
            return False
        time.sleep(0.05)
    return True


def windowed(series, size=60):
    """The largest sum over `size` consecutive one-second samples."""
    return max(sum(series[i:i + size]) for i in range(len(series) - size + 1))


IO = ['readBytes', 'writeBytes', 'physReadBytes', 'physWriteBytes']


def step(before, now):
    """One second of a watched, unchanged source from two process tables. A process started
    within it counts from zero. One that ended was reaped by its candidate parent, whose
    reaped-children CPU then holds all of the child's CPU, so the part already counted from the
    child is taken off. No reaped-children I/O counter exists, so a second in which a child ended
    has I/O as a lower bound (`ended`). Windows uses `Job` accounting instead."""
    ended = [r for p, r in before.items() if p not in now]
    base = lambda p, k: (before.get(p) or {}).get(k) or 0
    cpu = sum(r['cpuMs'] + (r['childCpuMs'] or 0) - base(p, 'cpuMs') - base(p, 'childCpuMs') for p, r in now.items()) \
        - sum(r['cpuMs'] + r['childCpuMs'] for r in ended if r['childCpuMs'] is not None)
    reaped = any((r['childCpuMs'] or 0) > base(p, 'childCpuMs') for p, r in now.items())
    io = {k: None if any(r.get(k) is None for r in now.values()) else sum(r[k] - base(p, k) for p, r in now.items()) for k in IO}
    return {'cpuMs': cpu, **io, 'ended': bool(ended) or reaped}


def idle_cost(series, cpu_exact):
    """Per-second `step`s → per-60-s costs. Full idle is the worst 60-s window, which holds a
    scheduled content verification; polling only leaves out the seconds whose CPU stands out
    (those verifications)."""
    cpu = [s['cpuMs'] for s in series]
    floor = statistics.median(cpu) * 3 + 10
    quiet = [c for c in cpu if c <= floor]
    io = lambda k: None if any(s[k] is None for s in series) else windowed([s[k] for s in series])
    return {'seconds': len(series), 'verificationSeconds': len(cpu) - len(quiet), 'cpuMsPer60s': windowed(cpu),
            'pollingCpuMsPer60s': round(sum(quiet) * 60 / len(quiet)), 'cpuExact': cpu_exact,
            'ioExact': cpu_exact and not any(s['ended'] for s in series),
            **{f'{k}Per60s': io(k) for k in IO}}


# --- one isolated state root per scenario ---

class Workspace:
    def __init__(self, binary, fixture, parent):
        self.binary, self.base = binary, Path(tempfile.mkdtemp(prefix='ws-', dir=parent))
        self.env = dict(environment(self.base, sys.executable),
                        **{f'XDG_{k}_HOME': str(self.base/k.lower()) for k in ['CONFIG', 'STATE', 'DATA']})
        self.source = self.base/'design.fig'
        shutil.copy2(fixture, self.source)

    def cli(self, *args):
        return json.loads(run([self.binary, *args], self.base, self.env, timeout=120))

    def timed(self, *args):
        elapsed, _, out = search.run([self.binary, *args], self.base, self.env)
        return elapsed, json.loads(out)

    def save(self, copy):
        """A real save: the other revision replaces the source atomically."""
        shutil.copyfile(copy, self.base/'next.fig')
        os.replace(self.base/'next.fig', self.source)

    def hold(self, job=None):
        holder = (job.start if job else subprocess.Popen)(
            [self.binary, 'watch', 'run', '--file', str(self.source), '--scope', '0:0'], cwd=self.base, env=self.env,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, encoding='utf-8')
        lease = json.loads(holder.stdout.readline())
        assert lease['state'] == 'active', lease
        return holder

    def release(self, *holders):
        """End the leases, then wait for the runner too; False if a candidate process outlived 30 s."""
        for holder in holders:
            holder.stdin.close()
        for holder in holders:
            holder.wait(timeout=30)
        return settle(os.path.realpath(self.binary))


def changes(ws):
    return sum(r['kind'] == 'changes' for r in ws.cli('changes', '--file', ws.source, '--scope', '0:0', '--limit', '50')['revisions'])


def idle(ws, exe, holders, seconds):
    """`holders` leases on the unchanged source for `seconds`, after 5 s to settle. On Windows the
    holders run in a `Job`, whose cumulative accounting includes processes that ended."""
    job = Job() if os.name == 'nt' else None
    processes = []
    try:
        processes += [ws.hold(job) for _ in range(holders)]
        time.sleep(5)
        series, before = [], job.counters() if job else table(exe)
        for _ in range(seconds):
            time.sleep(1)
            now = job.counters() if job else table(exe)
            series.append({'cpuMs': now[0] - before[0], 'readBytes': now[1] - before[1], 'writeBytes': now[2] - before[2],
                           'physReadBytes': None, 'physWriteBytes': None, 'ended': False} if job else step(before, now))
            before = now
    finally:
        ws.release(*processes)
        if job:
            job.close()  # without a kill-on-close limit, closing ends no process
    return {pid.pid for pid in processes}, idle_cost(series, cpu_exact=True)


def watch(binary, design, revised, n, seconds, parent):
    """Save→committed `changes` (N), idle CPU/I/O with one and eight holders, in-flight cancel
    (N attempts), runner and worker RSS, and the state root's disk use."""
    exe = os.path.realpath(binary)
    ws = Workspace(binary, design, parent)
    copies, size = [revised, design], design.stat().st_size
    ws.cli('index', 'prepare', ws.source, '--scope', '0:0')
    ws.cli('index', 'track', '--file', ws.source, '--scope', '0:0')
    db = next(ws.base.rglob('index.sqlite'))
    saves, cancels, holders = [], [], set()
    with Tree(exe, ws.base/'state') as tree:
        holder, seen = ws.hold(), changes(ws)
        holders.add(holder.pid)
        try:
            for i in range(n):
                time.sleep(1.5)  # let the previous turn finish and the detector settle
                ws.save(copies[i % 2])
                start = time.monotonic()
                while (now := changes(ws)) == seen and time.monotonic() - start < 30:
                    time.sleep(0.02)
                ms = (time.monotonic() - start) * 1000
                saves.append({'i': i, 'ms': round(ms, 1), 'recorded': now > seen})
                seen = now
        finally:
            ws.release(holder)
        pids, one = idle(ws, exe, 1, seconds)
        holders |= pids
        pids, eight = idle(ws, exe, 8, seconds)
        holders |= pids
        for i in range(n):
            # A cancelled attempt leaves its save unbuilt; build it first, so this save is the only change.
            ws.cli('index', 'refresh', '--file', ws.source, '--scope', '0:0')
            holder = ws.hold()
            holders.add(holder.pid)
            conn = sqlite3.connect(f'file:{db}?mode=ro', uri=True)
            state = lambda job: conn.execute('SELECT state FROM jobs WHERE job_id = ?', (job,)).fetchone()[0]
            jobs = conn.execute('SELECT coalesce(max(seq), 0) FROM jobs').fetchone()[0]
            ws.save(copies[(n + i) % 2])  # the save loop left copies[(n - 1) % 2]
            start, job, runner, worker = time.monotonic(), None, None, None
            # This save's job, run by the holder's runner in a worker child; then the worker's CPU must grow.
            while time.monotonic() - start < 30 and worker is None:
                row = conn.execute('SELECT job_id, state FROM jobs WHERE seq > ? ORDER BY seq DESC LIMIT 1', (jobs,)).fetchone()
                if row:
                    job = row[0]
                    if row[1] not in ('queued', 'running'):
                        break
                    rows = table(exe)
                    runners, workers = roles(rows, {holder.pid})
                    runner = next(iter(runners), runner)
                    worker = next(((p, rows[p]['cpuMs']) for p in workers), None)
                time.sleep(0.02)
            in_flight, cpu = False, None
            if worker:
                time.sleep(0.1)
                again = table(exe).get(worker[0])
                cpu = again and again['cpuMs'] - worker[1]
                in_flight = bool(again) and cpu > 0 and state(job) == 'running'
            exits = {'worker': gone(worker and worker[0]), 'runner': gone(runner)}
            t0, done = time.monotonic(), {}
            holder.stdin.close()  # the last lease ends: the runner must cancel the job nobody joined
            while time.monotonic() - t0 < 30 and len(done) < 3:
                if 'job' not in done and (not job or state(job) not in ('queued', 'running')):
                    done['job'] = (time.monotonic() - t0) * 1000
                for k, check in exits.items():
                    if k not in done and check():
                        done[k] = (time.monotonic() - t0) * 1000
                time.sleep(0.01)
            settled = ws.release(holder)
            final = state(job) if job else None
            conn.close()
            cancels.append({'i': i, 'outcome': classify(in_flight, job, final, len(done) < 3), 'job': final, 'inFlight': in_flight,
                            'workerSeen': bool(worker), 'workerCpuMsIn100ms': cpu, 'runnerSeen': bool(runner),
                            'settled': settled, **{f'{k}EndMs': round(v, 1) for k, v in done.items()}})
    # A timed-out cancel of a job proven in flight counts as 30 s, a lower bound.
    valid = [max(x['jobEndMs'], x['workerEndMs'], x['runnerEndMs']) if x['outcome'] == 'cancelled_in_flight' else 30000
             for x in cancels if x['outcome'] == 'cancelled_in_flight' or (x['outcome'] == 'timed_out' and x['inFlight'])]
    io = lambda cost: {'read': within(cost['readBytesPer60s'], 3 * size + MiB, cost['ioExact']),
                       'write': within(cost['writeBytesPer60s'], size + 2 * MiB, cost['ioExact'])}
    return {
        # An unrecorded save counts as its 30 s wait, a lower bound, so it fails the gate instead of thinning N.
        'save': {'attempts': saves, **verdict('save', [x['ms'] if x['recorded'] else 30000 for x in saves])},
        'cancel': {'attempts': cancels, 'outcomes': Counter(x['outcome'] for x in cancels),
                   **verdict('cancel', valid)},
        'idleOneHolder': {**one, 'cpu': within(one['cpuMsPer60s'], LIMITS['idleCpuMsPer60s'], one['cpuExact']),
                          'polling': within(one['pollingCpuMsPer60s'], LIMITS['pollingCpuMsPer60s'], one['cpuExact']), **io(one)},
        'idleEightHolders': {**eight, 'cpu': within(eight['cpuMsPer60s'], LIMITS['idle8HoldersCpuMsPer60s'], eight['cpuExact']), **io(eight)},
        **tree.roles(holders),
        'disk': {'sourceBytes': size, **tree.disk, 'database': within(tree.disk['databaseBytes'], LIMITS['databaseBytes']),
                 'wal': within(tree.disk['walBytes'], LIMITS['walBytes']),
                 'staging': 'in memory: one source copy per active build, no staging file'},
    }


def builds(binary, design, n, parent):
    """First prepare of the area scope in a fresh state root (N), full rebuilds of the whole source
    (N) and indexed cold-process workspace search of the area scope (N)."""
    exe, prepare, build, cold = os.path.realpath(binary), [], [], []
    with Tree(exe) as tree:
        for _ in range(n):
            ws = Workspace(binary, design, parent)
            prepare.append(ws.timed('index', 'prepare', ws.source, '--scope', AREA)[0])
            shutil.rmtree(ws.base)
        ws = Workspace(binary, design, parent)
        tree.state = ws.base/'state'  # whole-source rebuilds are where the database and WAL peak
        ws.cli('index', 'prepare', ws.source, '--scope', '0:0')
        for _ in range(n):
            build.append(ws.timed('index', 'rebuild', '--file', ws.source, '--scope', '0:0')[0])
    ws = Workspace(binary, design, parent)
    ws.cli('index', 'prepare', ws.source, '--scope', AREA)
    for _ in range(n):
        elapsed, reply = ws.timed('search', '--file', ws.source, 'checkout', '--node', AREA)
        assert reply['results'], 'indexed search returned nothing'
        cold.append(elapsed)
    return {'prepare': verdict('prepare', prepare), 'build': verdict('build', build),
            'indexedColdSearch': verdict('indexedColdSearch', cold), 'tree': tree.result('buildTreePeakRssBytes'),
            'disk': {**tree.disk, 'database': within(tree.disk['databaseBytes'], LIMITS['databaseBytes']),
                     'wal': within(tree.disk['walBytes'], LIMITS['walBytes'])}}


def viewer(binary, design, n, parent):
    """`preview` routes on the area scope: catalog, then first and warm usages of the main component."""
    ws = Workspace(binary, design, parent)
    with Tree(os.path.realpath(binary)) as tree:
        proc = subprocess.Popen([binary, 'preview', str(ws.source), AREA], cwd=ws.base, env=ws.env,
                                stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True, encoding='utf-8')
        try:
            base = re.search(r'(http://\S+/)', proc.stderr.readline()).group(1)
            def get(route):
                start = time.perf_counter()
                body = urllib.request.urlopen(base + route, timeout=120).read()
                return (time.perf_counter() - start) * 1000, body
            catalog_ms, catalog = get('catalog')
            route = f'usages/{json.loads(catalog)["generation"]}?' + urllib.parse.urlencode({'target': '0:10'})
            first_ms, body = get(route)
            results = len(json.loads(body)['results'])
            assert results, 'viewer usages are empty'
            warm = [get(route)[0] for _ in range(n)]
        finally:
            stop_process(proc)
    return {'catalogMs': round(catalog_ms, 1), 'firstUsagesMs': round(first_ms, 1), 'firstPageResults': results,
            'viewerWarmUsages': verdict('viewerWarmUsages', warm), 'tree': tree.result('viewerTreePeakRssBytes')}


def workload(binary, design, revised, parent):
    """What the source holds, as figlens reports it: records and text bytes from search coverage;
    stored nodes, depth, the widest fan-out and instances from `tree` (instance children expanded
    from their component are records but not tree nodes). Nothing in it is rendered."""
    ws = Workspace(binary, design, parent)
    def coverage(node):
        reply = ws.cli('search', ws.source, 'checkout', '--node', node)['coverage']
        return {'records': reply['scannedRecords'], 'textBytes': reply['indexedTextBytes']}
    def walk(node, depth=0):
        yield depth, len(node.get('children') or []), node['type']
        for child in node.get('children') or []:
            yield from walk(child, depth + 1)
    nodes = list(walk(ws.cli('tree', ws.source, '0:0', '--depth', '64', '--json')))
    facts = {'bytes': design.stat().st_size, 'revisedBytes': revised.stat().st_size,
             'whole': coverage('0:0'), 'firstScope': coverage(AREA), 'storedNodes': len(nodes),
             'maxDepth': max(d for d, _, _ in nodes), 'maxFanout': max(f for _, f, _ in nodes),
             'instances': sum(t == 'INSTANCE' for _, _, t in nodes), 'renderedPixels': 0}
    shutil.rmtree(ws.base)
    return facts


def candidate(artifacts, out):
    """The exact candidate binary for this OS, checked against candidate.json."""
    metadata = json.loads((artifacts/'candidate.json').read_text())
    name = f'figlens-{metadata["version"]}-{TARGETS[sys.platform]}' + ('.zip' if os.name == 'nt' else '.tar.gz')
    data = (artifacts/name).read_bytes()
    assert hashlib.sha256(data).hexdigest() == metadata['archives'][name]['sha256'], 'candidate archive hash differs'
    if os.name == 'nt':
        with zipfile.ZipFile(artifacts/name) as archive:
            data = archive.read('figlens.exe')
    else:
        with tarfile.open(artifacts/name) as archive:
            data = archive.extractfile('figlens').read()
    assert hashlib.sha256(data).hexdigest() == metadata['archives'][name]['binarySha256']
    binary = out/('figlens.exe' if os.name == 'nt' else 'figlens')
    binary.write_bytes(data)
    binary.chmod(0o755)
    return binary, {'commit': metadata['commit'], 'archive': name, 'sha256': metadata['archives'][name]['sha256']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', required=True, type=Path, help='candidate directory with candidate.json')
    parser.add_argument('--design', required=True, type=Path, help='mapping-fixture --scale output')
    parser.add_argument('--revised', required=True, type=Path, help='the same with --revised')
    parser.add_argument('--commit', required=True, help='the checkout the candidate must be built from')
    parser.add_argument('--warm', type=int, default=100)
    parser.add_argument('--cold', type=int, default=20)
    parser.add_argument('--idle-seconds', type=int, default=130, help='over two 60 s content verifications')
    args = parser.parse_args()
    design, revised = args.design.resolve(), args.revised.resolve()
    (Path(__file__).resolve().parent.parent/'samples').mkdir(exist_ok=True)  # measure-search works under it
    # A runner that outlived its attempt may still hold files on Windows; the report matters more.
    with tempfile.TemporaryDirectory(prefix='figlens-native-', ignore_cleanup_errors=True) as temp:
        binary, provenance = candidate(args.artifacts.resolve(), Path(temp))
        assert provenance['commit'] == args.commit, 'candidate is from another commit'
        with Tree(os.path.realpath(binary)) as tree:
            # Cold stateless CLI and warm MCP search/usages of the area scope, stateless and workspace.
            cost = search.measure(binary, design, AREA, ['checkout', 'button'], args.cold, args.warm, ['0:10'], 1)
        timings = cost['timings']
        assert all(s['results'] for s in cost['usages']['shapes']), 'usages are empty'
        report = {
            'candidate': provenance, 'os': f'{platform.system()} {platform.release()} {platform.machine()}',
            'cpus': os.cpu_count(), 'workload': workload(binary, design, revised, Path(temp)),
            'gates': GATES, 'limits': LIMITS,
            'searchShapes': list(cost['shapes'].values()), 'usageShapes': cost['usages']['shapes'],
            'timings': {k: {'n': len(v['samplesMs']), 'medianMs': v['medianMs'], 'p95Ms': v['p95Ms']} for k, v in timings.items()},
            'warmSearch': verdict('warmSearch', timings['mcpWarmSearch']['samplesMs']),
            'warmUsages': verdict('warmUsages', timings['mcpWorkspaceUsages']['samplesMs']),
            'warmStatelessUsages': verdict('warmUsages', timings['mcpWarmUsages']['samplesMs']),
            'cliPeakRssBytes': cost['cliPeakRssBytes'], 'usagePages': cost['usages']['mcpMemory']['pages'],
            # The MCP server spawns no children, so its own peak is its tree's.
            'mcpTree': tree.result('mcpTreePeakRssBytes', cost['mcpPids']),
            'builds': builds(binary, design, args.cold, Path(temp)),
            'viewer': viewer(binary, design, args.warm, Path(temp)),
            'watch': watch(binary, design, revised, args.cold, args.idle_seconds, Path(temp)),
            'costBoundaries': 'Cold = a new process with the OS file cache warm. A sampler lists every process 10 times a second '
                              'during all timings, as the cost of RSS evidence. ' + cost['costBoundaries'],
        }
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
