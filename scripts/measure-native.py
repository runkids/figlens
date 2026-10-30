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


def within(value, limit):
    return {'value': value, 'limit': limit,
            'status': 'unmeasured' if value is None else 'pass' if value <= limit else 'fail'}


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
                                       'cpuMs': (times[2] + times[3]) // 10_000, 'readBytes': io[3], 'writeBytes': io[4]}
                kernel32.CloseHandle(handle)
            ok = kernel32.Process32NextW(snap, ctypes.byref(entry))
        kernel32.CloseHandle(snap)
        return rows
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
                                  'readBytes': io['rchar'], 'writeBytes': io['wchar'],
                                  'physReadBytes': io['read_bytes'], 'physWriteBytes': io['write_bytes']}
            except (OSError, ValueError, KeyError, IndexError):
                pass  # exited between listing and reading
        return rows


def roles(rows, holders):
    """(runners, workers) among `rows`: a runner was started by a holder, and may already be
    detached from it (macOS reparents it at once); a worker was started by a runner. The
    harness's own direct children are holders and CLI calls."""
    runners = {p for p, r in rows.items() if p not in holders and r['ppid'] != os.getpid()
               and (r['ppid'] in holders or r['ppid'] not in rows)}
    return runners, {p for p, r in rows.items() if r['ppid'] in runners}


class Tree:
    """While the block runs: peak RSS of all candidate processes together and of each one, and
    with `state`, the largest state-root, database and WAL sizes seen."""
    def __init__(self, exe, state=None):
        self.exe, self.state, self.done = exe, state, threading.Event()
        self.peak, self.procs, self.samples, self.disk = 0, {}, 0, {'stateBytes': 0, 'databaseBytes': 0, 'walBytes': 0}

    def loop(self):
        while not self.done.is_set():
            rows = table(self.exe)
            self.peak = max(self.peak, sum(r['rss'] for r in rows.values()))
            for pid, r in rows.items():
                seen = self.procs.setdefault(pid, {'ppid': r['ppid'], 'peak': 0})  # the parent at start
                seen['peak'] = max(seen['peak'], r['rss'], r.get('peak', 0))
            if self.state and self.samples % 10 == 0:
                files = [(p.name, p.stat().st_size) for p in self.state.rglob('*') if p.is_file()]
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

    def roles(self, holders):
        """Peak RSS of the runners the holders started and of the workers those runners started."""
        runners, workers = roles(self.procs, holders)
        peak = lambda pids: max((self.procs[p]['peak'] for p in pids), default=None)
        return {'runners': len(runners), 'runnerPeakRss': within(peak(runners), LIMITS['runnerPeakRssBytes']),
                'workers': len(workers), 'workerPeakRss': within(peak(workers), LIMITS['workerPeakRssBytes'])}

    def result(self, limit):
        return {'treePeakSampledBytes': self.peak, 'samples': self.samples,
                'largestProcessPeakBytes': max((r['peak'] for r in self.procs.values()), default=None),
                'treePeakRss': within(self.peak, LIMITS[limit])}


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


def idle_cost(series):
    """Per-second (cpuMs, readBytes, writeBytes, physReadBytes, physWriteBytes) of a watched,
    unchanged source → per-60-s costs.
    Full idle is the worst 60-s window, which holds a scheduled content verification; polling only
    leaves out the seconds whose CPU stands out (those verifications)."""
    cpu = [s[0] for s in series]
    floor = statistics.median(cpu) * 3 + 10
    quiet = [c for c in cpu if c <= floor]
    io = lambda k: None if any(s[k] is None for s in series) else windowed([s[k] for s in series])
    return {'seconds': len(series), 'verificationSeconds': len(cpu) - len(quiet), 'cpuMsPer60s': windowed(cpu),
            'pollingCpuMsPer60s': round(sum(quiet) * 60 / len(quiet)), 'readBytesPer60s': io(1), 'writeBytesPer60s': io(2),
            'physReadBytesPer60s': io(3), 'physWriteBytesPer60s': io(4)}


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

    def hold(self):
        holder = subprocess.Popen([self.binary, 'watch', 'run', '--file', str(self.source), '--scope', '0:0'],
                                  cwd=self.base, env=self.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, encoding='utf-8')
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
    """`holders` leases on the unchanged source for `seconds`, after 5 s to settle."""
    processes = [ws.hold() for _ in range(holders)]
    try:
        time.sleep(5)
        series, before = [], table(exe)
        for _ in range(seconds):
            time.sleep(1)
            now = table(exe)
            delta = lambda k: None if any(r.get(k) is None for r in now.values()) else \
                sum(r[k] - before[p][k] for p, r in now.items() if p in before)
            series.append(tuple(map(delta, ['cpuMs', 'readBytes', 'writeBytes', 'physReadBytes', 'physWriteBytes'])))
            before = now
    finally:
        ws.release(*processes)
    return {pid.pid for pid in processes}, idle_cost(series)


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
            ws.save(copies[i % 2])
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
                time.sleep(0.002)
            settled = ws.release(holder)
            final = state(job) if job else None
            conn.close()
            cancels.append({'i': i, 'outcome': classify(in_flight, job, final, len(done) < 3), 'job': final,
                            'workerSeen': bool(worker), 'workerCpuMsIn100ms': cpu, 'runnerSeen': bool(runner),
                            'settled': settled, **{f'{k}EndMs': round(v, 1) for k, v in done.items()}})
    valid = [x for x in cancels if x['outcome'] == 'cancelled_in_flight']
    io = lambda cost: {'read': within(cost['readBytesPer60s'], 3 * size + MiB), 'write': within(cost['writeBytesPer60s'], size + 2 * MiB)}
    return {
        'save': {'attempts': saves, **verdict('save', [x['ms'] for x in saves if x['recorded']])},
        'cancel': {'attempts': cancels, 'outcomes': Counter(x['outcome'] for x in cancels),
                   **verdict('cancel', [max(x['jobEndMs'], x['workerEndMs'], x['runnerEndMs']) for x in valid])},
        'idleOneHolder': {**one, 'cpu': within(one['cpuMsPer60s'], LIMITS['idleCpuMsPer60s']),
                          'polling': within(one['pollingCpuMsPer60s'], LIMITS['pollingCpuMsPer60s']), **io(one)},
        'idleEightHolders': {**eight, 'cpu': within(eight['cpuMsPer60s'], LIMITS['idle8HoldersCpuMsPer60s']), **io(eight)},
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
            'indexedColdSearch': verdict('indexedColdSearch', cold), 'tree': tree.result('buildTreePeakRssBytes')}


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


def workload(binary, design, revised, screens):
    """What the source holds, as figlens counts it, plus the generator's fixed shape."""
    def coverage(node):
        reply = json.loads(run([binary, 'search', design, 'checkout', '--node', node], None, None, timeout=120))
        return {'records': reply['coverage']['scannedRecords'], 'textBytes': reply['coverage']['indexedTextBytes']}
    return {'bytes': design.stat().st_size, 'revisedBytes': revised.stat().st_size, 'screens': screens,
            'whole': coverage('0:0'), 'firstScope': coverage(AREA),
            # mapping-fixture --scale: document > canvas > area > screen > section > group > instance > surface.
            'maxDepth': 7, 'componentInstances': 2 + 4 * screens, 'firstScopeComponentInstances': 4 * min(screens, 800),
            'renderedPixels': 0}


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
    parser.add_argument('--screens', type=int, default=8000, help='the --scale the design was made with')
    parser.add_argument('--warm', type=int, default=100)
    parser.add_argument('--cold', type=int, default=20)
    parser.add_argument('--idle-seconds', type=int, default=130, help='over two 60 s content verifications')
    args = parser.parse_args()
    design, revised = args.design.resolve(), args.revised.resolve()
    (Path(__file__).resolve().parent.parent/'samples').mkdir(exist_ok=True)  # measure-search works under it
    with tempfile.TemporaryDirectory(prefix='figlens-native-') as temp:
        binary, provenance = candidate(args.artifacts.resolve(), Path(temp))
        with Tree(os.path.realpath(binary)) as tree:
            # Cold stateless CLI and warm MCP search/usages of the area scope, stateless and workspace.
            cost = search.measure(binary, design, AREA, ['checkout', 'button'], args.cold, args.warm, ['0:10'], 1)
        timings = cost['timings']
        report = {
            'candidate': provenance, 'os': f'{platform.system()} {platform.release()} {platform.machine()}',
            'cpus': os.cpu_count(), 'workload': workload(binary, design, revised, args.screens),
            'gates': GATES, 'limits': LIMITS,
            'searchShapes': list(cost['shapes'].values()), 'usageShapes': cost['usages']['shapes'],
            'timings': {k: {'n': len(v['samplesMs']), 'medianMs': v['medianMs'], 'p95Ms': v['p95Ms']} for k, v in timings.items()},
            'warmSearch': verdict('warmSearch', timings['mcpWarmSearch']['samplesMs']),
            'warmUsages': verdict('warmUsages', timings['mcpWorkspaceUsages']['samplesMs']),
            'warmStatelessUsages': verdict('warmUsages', timings['mcpWarmUsages']['samplesMs']),
            'cliPeakRssBytes': cost['cliPeakRssBytes'], 'usagePages': cost['usages']['mcpMemory']['pages'],
            'mcpTree': tree.result('mcpTreePeakRssBytes'),
            'builds': builds(binary, design, args.cold, Path(temp)),
            'viewer': viewer(binary, design, args.warm, Path(temp)),
            'watch': watch(binary, design, revised, args.cold, args.idle_seconds, Path(temp)),
            'costBoundaries': 'Cold = a new process with the OS file cache warm. ' + cost['costBoundaries'],
        }
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
