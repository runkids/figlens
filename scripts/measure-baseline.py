#!/usr/bin/env python3
"""Bounded local timings and Linux RSS observations; no cloud or cache eviction."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import tempfile
import time
import zipfile
from installation import environment, stop_process
from mcp_smoke import session

ROOT = Path(__file__).resolve().parent.parent


def summarize(samples):
    ordered = sorted(samples)
    return {'samplesMs': samples, 'medianMs': statistics.median(samples),
            'p95Ms': ordered[math.ceil(len(ordered) * .95) - 1]}


def memory(pid):
    """Live RSS and lifetime high-water, not allocations attributable to a request."""
    try:
        lines = Path(f'/proc/{pid}/status').read_text().splitlines()
    except (FileNotFoundError, PermissionError):
        return None
    return {name: int(line.split()[1]) * 1024 for line in lines
            for name, field in [('rssBytes', 'VmRSS:'), ('highWaterBytes', 'VmHWM:')]
            if line.startswith(field)}


def container_memory():
    values = {}
    for field in ['memory.current', 'memory.peak', 'memory.max', 'cpu.max']:
        try:
            values[field] = Path('/sys/fs/cgroup', field).read_text().strip()
        except (FileNotFoundError, PermissionError):
            values[field] = None
    return values


def entries(path):
    with zipfile.ZipFile(path) as archive:
        result = {name: archive.read(name) for name in archive.namelist()}
    manifest = json.loads(result['manifest.json'])
    manifest.pop('createdAt')
    for item in manifest['files']:
        assert hashlib.sha256(result[item['path']]).hexdigest() == item['sha256']
    result['manifest.json'] = json.dumps(manifest, sort_keys=True).encode()
    return result


def measure(binary, repetitions, fixture, node, alternate=None, frames=()):
    samples, observations, sizes = {}, {}, {}
    container_before = container_memory()
    def timed(key, action):
        start = time.perf_counter()
        value = action()
        samples.setdefault(key, []).append((time.perf_counter() - start) * 1000)
        return value
    def run(command, cwd, env):
        # wait4 returns this child's own peak, unlike RUSAGE_CHILDREN's historical max.
        with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
            process = subprocess.Popen(list(map(str, command)), cwd=cwd, env=env,
                                       stdout=stdout, stderr=stderr, start_new_session=True)
            deadline = time.monotonic() + 60
            try:
                while True:
                    pid, status, usage = os.wait4(process.pid, os.WNOHANG)
                    if pid:
                        process.returncode = os.waitstatus_to_exitcode(status)
                        break
                    if time.monotonic() >= deadline:
                        raise TimeoutError('command exceeded 60 seconds')
                    time.sleep(.001)
            except BaseException:
                stop_process(process)
                raise
            stdout.seek(0)
            stderr.seek(0)
            assert process.returncode == 0, stderr.read().decode()
            key = str(command[1]) + 'Process'
            observations.setdefault(key, []).append({'arguments': list(map(str, command[1:])), 'peakRssBytes': int(usage.ru_maxrss * (1 if sys.platform == 'darwin' else 1024))})
            return stdout.read().decode()

    def call(request, tool, file, selected):
        args = {'file': str(file), 'node': selected}
        if tool == 'tree':
            args.update(depth=1, json=True)
        reply = request('tools/call', {'name': tool, 'arguments': args})
        assert not reply['isError'], reply
        return reply
    (ROOT/'samples').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='baseline-', dir=ROOT/'samples') as directory:
        work = Path(directory)
        env = environment(work, sys.executable)
        version = run([binary, '--version'], work, env).strip()
        alternate_expected = json.loads(run([binary, 'nodes', *alternate], work, env)) if alternate else None
        expected = None
        for _ in range(repetitions):
            value = json.loads(timed('nodesProcess', lambda: run([binary, 'nodes', fixture, node], work, env)))
            if expected is None:
                expected = value
            assert value == expected
            start = time.perf_counter()
            with session([binary, 'mcp'], work, env, timeout=60) as request:
                request('initialize', {'protocolVersion': '2025-06-18'})
                samples.setdefault('mcpStartInitialize', []).append((time.perf_counter()-start)*1000)
                request('notifications/initialized', {})
                reply = timed('mcpFirstNodes', lambda: call(request, 'nodes', fixture, node))
                assert json.loads(reply['content'][0]['text']) == expected
                observations.setdefault('mcpFirstNodes', []).append(memory(request.pid))
                for tool in ['tree', 'inspect', 'nodes']:
                    key = 'mcpWarm' + tool.title()
                    reference = call(request, tool, fixture, node)
                    sizes[key] = len(json.dumps(reference).encode())
                    observations.setdefault(key, []).append({'before': memory(request.pid)})
                    for _ in range(10):
                        assert timed(key, lambda: call(request, tool, fixture, node)) == reference
                    observations[key][-1]['after'] = memory(request.pid)
            if alternate:
                with session([binary, 'mcp'], work, env, timeout=60) as request:
                    request('initialize', {'protocolVersion': '2025-06-18'})
                    request('notifications/initialized', {})
                    assert alternate_expected != expected, 'A/B must have different node content'
                    for label, file, selected in [('A', fixture, node), ('B', *alternate), ('Aagain', fixture, node)]:
                        key = 'alternate' + label
                        reply = timed(key, lambda: call(request, 'nodes', file, selected))
                        observations.setdefault(key, []).append(memory(request.pid))
                        assert json.loads(reply['content'][0]['text']) == (alternate_expected if label == 'B' else expected)
        if frames:
            reference = {}
            frame_list = work/'frames.txt'
            frame_list.write_text('\n'.join(frames))
            for iteration in range(3):
                for mode in ['sequential', '1', '2', 'default', 'mcp']:
                    output = work/f'{mode}-{iteration}'
                    output.mkdir()
                    def bundle():
                        if mode == 'sequential':
                            for frame in frames:
                                run([binary, 'bundle', fixture, frame, output/(frame.replace(':', '-')+'.zip')], work, env)
                        elif mode == 'mcp':
                            with session([binary, 'mcp'], work, env, timeout=60) as request:
                                request('initialize', {'protocolVersion': '2025-06-18'})
                                request('notifications/initialized', {})
                                for frame in frames:
                                    reply = request('tools/call', {'name': 'bundle', 'arguments': {'file': str(fixture), 'node': frame, 'out': str(output/(frame.replace(':', '-')+'.zip'))}})
                                    assert not reply['isError'], reply
                                observations.setdefault('bundleMcp', []).append(memory(request.pid))
                        else:
                            args = [] if mode == 'default' else ['--jobs', mode]
                            rows = run([binary, 'bundle', fixture, '--frames', frame_list, '--out-dir', output, *args], work, env)
                            assert all('error' not in json.loads(row) for row in rows.splitlines())
                    timed('bundle'+mode.title(), bundle)
                    for frame in frames:
                        value = entries(output/(frame.replace(':', '-')+'.zip'))
                        if frame not in reference:
                            reference[frame] = value
                        assert value == reference[frame], (mode, frame)
    files = [fixture] + ([alternate[0]] if alternate else [])
    def digest(path):
        return hashlib.sha256(path.read_bytes()).hexdigest()
    return {'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
            'dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT)),
            'platform': platform.platform(), 'machine': platform.machine(), 'python': platform.python_version(),
            'binary': str(binary), 'binaryVersion': version, 'binarySha256': digest(binary),
            'fixtures': [{'path': str(p), 'bytes': p.stat().st_size, 'sha256': digest(p)} for p in files],
            'node': node, 'frames': frames, 'repetitions': repetitions,
            'conditions': 'isolated HOME/cache; OS cache not flushed; no GCS; 60s command timeout',
            'costBoundaries': 'CLI includes spawn and up to 1ms wait4 polling; MCP includes IPC and serialization. Warm loops are 10 per server. RSS is Linux boundary observations and lifetime high-water, not per-stage allocation. Missing RSS is null. Bundle modes include startup and verification is outside timing.',
            'outputBytes': sizes, 'memory': observations,
            'container': {'before': container_before, 'after': container_memory(),
                          'scope': 'cgroup lifetime includes Python harness, page cache and all children; not a per-stage allocation'},
            'timings': {key: summarize(values) for key, values in samples.items()}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--repetitions', type=int, default=5)
    parser.add_argument('--fixture', type=Path, default=ROOT/'examples/first-success/design.fig')
    parser.add_argument('--node', default='0:1')
    parser.add_argument('--alternate', type=Path)
    parser.add_argument('--alternate-node', default='0:1')
    parser.add_argument('--frames', nargs=3, default=())
    args = parser.parse_args()
    if not 3 <= args.repetitions <= 30:
        parser.error('repetitions must be 3-30')
    alternate = (args.alternate.resolve(), args.alternate_node) if args.alternate else None
    print(json.dumps(measure(args.binary.resolve(), args.repetitions, args.fixture.resolve(), args.node, alternate, args.frames), indent=2))


if __name__ == '__main__':
    main()
