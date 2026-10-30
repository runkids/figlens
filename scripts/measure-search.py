#!/usr/bin/env python3
"""Search and usages cost on one approved input: cold CLI, MCP first call and warm retrieval,
workspace builds, peak RSS.

Reports counts and timings, never names or text from the design, plus the caller's own
`--query`/`--node` values as metadata: treat a report on a private file as private and keep
it under the ignored `samples/`. Run it inside `docker compose run --rm dev` so MCP RSS
comes from Linux /proc.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from installation import environment, stop_process
from mcp_smoke import session
from importlib import import_module

baseline = import_module('measure-baseline')
ROOT = Path(__file__).resolve().parent.parent


def windows_memory(handle):
    """(WorkingSetSize, PeakWorkingSetSize); an exited process keeps them while its handle is open."""
    import ctypes
    counters = (ctypes.c_size_t * 10)()  # PROCESS_MEMORY_COUNTERS_EX; cb and PageFaultCount share slot 0
    counters[0] = ctypes.sizeof(counters)
    assert ctypes.windll.kernel32.K32GetProcessMemoryInfo(ctypes.c_void_p(int(handle)), counters, ctypes.sizeof(counters))
    return counters[2], counters[1]


def run(command, cwd, env):
    """Elapsed ms, the child's own peak RSS (wait4, or the peak working set on Windows) and stdout."""
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        start = time.perf_counter()
        process = subprocess.Popen(list(map(str, command)), cwd=cwd, env=env,
                                   stdout=stdout, stderr=stderr, start_new_session=True)
        deadline = time.monotonic() + 60
        try:
            if os.name == 'nt':
                code = process.wait(timeout=60)
                peak = windows_memory(process._handle)[1]
            else:
                while True:
                    pid, status, usage = os.wait4(process.pid, os.WNOHANG)
                    if pid:
                        break
                    if time.monotonic() >= deadline:
                        raise TimeoutError('command exceeded 60 seconds')
                    time.sleep(.001)
                code = os.waitstatus_to_exitcode(status)
                peak = usage.ru_maxrss * (1 if sys.platform == 'darwin' else 1024)
        except BaseException:
            stop_process(process)
            raise
        elapsed = (time.perf_counter() - start) * 1000
        stdout.seek(0)
        stderr.seek(0)
        assert code == 0, stderr.read().decode()
        return elapsed, peak, stdout.read().decode()


def shape(reply):
    """What a search reply says about its scope, without any design content."""
    coverage = reply['coverage']
    return {'results': len(reply['results']), 'hasMore': reply['hasMore'],
            'complete': coverage['complete'], 'scannedRecords': coverage['scannedRecords'],
            'indexedTextBytes': coverage['indexedTextBytes'],
            'reasons': [r['code'] for r in coverage['reasons']],
            'origins': sorted({r['origin'] for r in reply['results']})}


def usage_shape(reply):
    """What a usages reply says, without any design content."""
    return {'results': len(reply['results']), 'hasMore': reply['hasMore'],
            'complete': reply['coverage']['complete'],
            'relations': sorted({r['relation'] for r in reply['results']}),
            'members': len(reply.get('members', {}).get('nodeIds', []))}


def measure_usages(binary, fixture, node, targets, cold, warm, builds, work, env, cli, samples):
    """Stateless cold CLI and warm MCP usages, then cold workspace builds and warm workspace usages."""
    shapes = {}
    for _ in range(cold):
        for target in targets:
            shapes[target] = usage_shape(json.loads(cli('cliUsages', [binary, 'usages', fixture, target, '--scope', node])))
    memory = {}
    with session([binary, 'mcp'], work, env, timeout=60) as request:
        request('initialize', {'protocolVersion': '2025-06-18'})
        request('notifications/initialized', {})
        def call(arguments):
            target = arguments.pop('node')
            arguments['resource' if target.startswith(('VariableID:', 'StyleKey:', 'image:')) else 'node'] = target
            reply = request('tools/call', {'name': 'usages', 'arguments': arguments})
            assert not reply['isError'], reply
            return reply['structuredContent']
        for i in range(warm + 1):
            target = targets[i % len(targets)]
            start = time.perf_counter()
            reply = call({'file': str(fixture), 'node': target, 'scope': node})
            samples.setdefault('mcpWarmUsages' if i else 'mcpFirstUsages', []).append((time.perf_counter() - start) * 1000)
            assert usage_shape(reply) == shapes[target]
        memory['statelessAfterWarm'] = baseline.memory(request.pid)
        # Each build is a fresh generation of the whole scope: hash, parse, index, write, verify.
        cli('cliIndexPrepare', [binary, 'index', 'prepare', fixture, '--scope', node])
        for _ in range(builds - 1):
            cli('cliIndexRebuild', [binary, 'index', 'rebuild', '--file', fixture, '--scope', node])
        ws = json.loads(cli('cliIndexStatus', [binary, 'index', 'status', '--file', fixture]))['workspaceId']
        for i in range(warm):
            target = targets[i % len(targets)]
            start = time.perf_counter()
            reply = call({'workspaceId': ws, 'node': target, 'scope': node})
            samples.setdefault('mcpWorkspaceUsages', []).append((time.perf_counter() - start) * 1000)
            assert usage_shape(reply) == shapes[target], (target, reply)
        memory['workspaceAfterWarm'] = baseline.memory(request.pid)
        # Every page of each target: a late page must cost about what the first one does.
        pages = {}
        for target in targets:
            for lane, base in [('stateless', {'file': str(fixture)}), ('workspace', {'workspaceId': ws})]:
                cursor, times = None, []
                while True:
                    arguments = {**base, 'node': target, 'scope': node, 'limit': 50}
                    if cursor:
                        arguments['cursor'] = cursor
                    start = time.perf_counter()
                    reply = call(arguments)
                    times.append(round((time.perf_counter() - start) * 1000, 3))
                    cursor = reply['nextCursor']
                    if not cursor:
                        break
                pages.setdefault(lane, []).append({'pages': len(times), 'firstMs': times[0], 'lastMs': times[-1], 'maxMs': max(times)})
        memory['pages'] = pages
    for target in targets:
        cli('cliWorkspaceUsages', [binary, 'usages', '--file', fixture, target, '--scope', node])
    return shapes, memory


def measure(binary, fixture, node, queries, cold, warm, targets=(), builds=0):
    samples, peaks, shapes = {}, {}, {}
    with tempfile.TemporaryDirectory(prefix='search-', dir=ROOT/'samples') as directory:
        work = Path(directory)
        env = environment(work, sys.executable)
        def cli(key, command):
            elapsed, peak, out = run(command, work, env)
            samples.setdefault(key, []).append(elapsed)
            peaks.setdefault(key, []).append(peak)
            return out
        for _ in range(cold):
            # Decode alone, for comparison: tree reads the same bytes without hashing or indexing.
            cli('cliTreeDepth0', [binary, 'tree', fixture, node, '--depth', '0'])
            for query in queries:
                shapes[query] = shape(json.loads(cli('cliSearch', [binary, 'search', fixture, query, '--node', node])))
        with session([binary, 'mcp'], work, env, timeout=60) as request:
            request('initialize', {'protocolVersion': '2025-06-18'})
            request('notifications/initialized', {})
            def call(query):
                reply = request('tools/call', {'name': 'search', 'arguments': {'file': str(fixture), 'query': query, 'node': node}})
                assert not reply['isError'], reply
                return reply['structuredContent']
            start = time.perf_counter()
            first = call(queries[0])
            samples['mcpFirstSearch'] = [(time.perf_counter() - start) * 1000]
            assert not first['source']['freshness']['reusedIndex']
            after_first = baseline.memory(request.pid)
            for i in range(warm):
                query = queries[i % len(queries)]
                start = time.perf_counter()
                reply = call(query)
                samples.setdefault('mcpWarmSearch', []).append((time.perf_counter() - start) * 1000)
                assert reply['source']['freshness']['reusedIndex']
                assert shape(reply) == shapes[query]
            after_warm = baseline.memory(request.pid)
        usages = measure_usages(binary, fixture, node, targets, cold, warm, builds, work, env, cli, samples) if targets else None
    return {
        'version': run([binary, '--version'], ROOT, os.environ)[2].strip(),
        'node': node, 'queries': queries, 'fixtureBytes': Path(fixture).stat().st_size,
        'timings': {k: baseline.summarize(v) for k, v in samples.items()},
        'cliPeakRssBytes': {k: max(v) for k, v in peaks.items()},
        'mcpMemory': {'afterFirst': after_first, 'afterWarm': after_warm},
        'container': baseline.container_memory(),
        'shapes': shapes,
        'usages': usages and {'targets': len(targets), 'shapes': list(usages[0].values()), 'mcpMemory': usages[1]},
        'costBoundaries': 'CLI includes spawn and up to 1 ms wait4 polling; cliSearch is a cold process: stage, SHA-256, decode, scope expansion, indexing and retrieval. MCP first call is the same in a server; warm calls reuse the index (stamp check, open check, retrieval, serialization, IPC). Usages: cliUsages is a cold stateless process like cliSearch; mcpWarmUsages reuse the process index; cliIndexPrepare/Rebuild wait for a whole build and their RSS includes the waited worker; mcpWorkspaceUsages read the published generation from SQLite. The MCP server spawns no children, so its RSS is the process tree.',
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--fixture', required=True, type=Path)
    parser.add_argument('--node', default='0:0')
    parser.add_argument('--query', action='append', required=True)
    parser.add_argument('--cold', type=int, default=5)
    parser.add_argument('--warm', type=int, default=100)
    parser.add_argument('--usages', action='append', default=[], help='node or resource id; repeatable')
    parser.add_argument('--builds', type=int, default=20)
    args = parser.parse_args()
    print(json.dumps(measure(args.binary.resolve(), args.fixture.resolve(), args.node, args.query, args.cold, args.warm, args.usages, args.builds), indent=2))


if __name__ == '__main__':
    main()
