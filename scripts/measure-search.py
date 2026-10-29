#!/usr/bin/env python3
"""Search cost on one approved input: cold CLI, MCP first call and warm retrieval, peak RSS.

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


def run(command, cwd, env):
    """Elapsed ms, the child's own peak RSS (wait4) and stdout."""
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        start = time.perf_counter()
        process = subprocess.Popen(list(map(str, command)), cwd=cwd, env=env,
                                   stdout=stdout, stderr=stderr, start_new_session=True)
        deadline = time.monotonic() + 60
        try:
            while True:
                pid, status, usage = os.wait4(process.pid, os.WNOHANG)
                if pid:
                    break
                if time.monotonic() >= deadline:
                    raise TimeoutError('command exceeded 60 seconds')
                time.sleep(.001)
        except BaseException:
            stop_process(process)
            raise
        elapsed = (time.perf_counter() - start) * 1000
        stdout.seek(0)
        stderr.seek(0)
        assert os.waitstatus_to_exitcode(status) == 0, stderr.read().decode()
        peak = usage.ru_maxrss * (1 if sys.platform == 'darwin' else 1024)
        return elapsed, peak, stdout.read().decode()


def shape(reply):
    """What a search reply says about its scope, without any design content."""
    coverage = reply['coverage']
    return {'results': len(reply['results']), 'hasMore': reply['hasMore'],
            'complete': coverage['complete'], 'scannedRecords': coverage['scannedRecords'],
            'indexedTextBytes': coverage['indexedTextBytes'],
            'reasons': [r['code'] for r in coverage['reasons']],
            'origins': sorted({r['origin'] for r in reply['results']})}


def measure(binary, fixture, node, queries, cold, warm):
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
    return {
        'version': run([binary, '--version'], ROOT, os.environ)[2].strip(),
        'node': node, 'queries': queries, 'fixtureBytes': Path(fixture).stat().st_size,
        'timings': {k: baseline.summarize(v) for k, v in samples.items()},
        'cliPeakRssBytes': {k: max(v) for k, v in peaks.items()},
        'mcpMemory': {'afterFirst': after_first, 'afterWarm': after_warm},
        'container': baseline.container_memory(),
        'shapes': shapes,
        'costBoundaries': 'CLI includes spawn and up to 1 ms wait4 polling; cliSearch is a cold process: stage, SHA-256, decode, scope expansion, indexing and retrieval. MCP first call is the same in a server; warm calls reuse the index (stamp check, open check, retrieval, serialization, IPC). The MCP server spawns no children, so its RSS is the process tree.',
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--fixture', required=True, type=Path)
    parser.add_argument('--node', default='0:0')
    parser.add_argument('--query', action='append', required=True)
    parser.add_argument('--cold', type=int, default=5)
    parser.add_argument('--warm', type=int, default=100)
    args = parser.parse_args()
    print(json.dumps(measure(args.binary.resolve(), args.fixture.resolve(), args.node, args.query, args.cold, args.warm), indent=2))


if __name__ == '__main__':
    main()
