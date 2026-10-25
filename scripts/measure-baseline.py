#!/usr/bin/env python3
"""Bounded local public-fixture timings; no cloud access or filesystem cache eviction."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import tempfile
import time
from installation import environment, run
from mcp_smoke import session

ROOT = Path(__file__).resolve().parent.parent


def summarize(samples):
    ordered = sorted(samples)
    return {'samplesMs':samples, 'medianMs':statistics.median(samples),
            'p95Ms':ordered[math.ceil(len(ordered)*.95)-1]}


def measure(binary, repetitions):
    fixture = ROOT/'examples/first-success/design.fig'
    samples = {key:[] for key in ['versionProcess', 'nodesProcess', 'mcpStartInitialize', 'mcpFirstNodes', 'mcpWarmNodes']}
    def timed(key, action):
        start = time.perf_counter()
        value = action()
        samples[key].append((time.perf_counter()-start)*1000)
        return value
    with tempfile.TemporaryDirectory(prefix='figlens-baseline-') as directory:
        work = Path(directory)
        env = environment(work, sys.executable)
        version = run([binary,'--version'], work, env).strip()
        for _ in range(repetitions):
            assert timed('versionProcess', lambda: run([binary,'--version'], work, env)).strip() == version
            nodes = json.loads(timed('nodesProcess', lambda: run([binary,'nodes',fixture,'0:1'], work, env)))
            assert nodes['document']['id'] == '0:1'
            start = time.perf_counter()
            with session([binary,'mcp'], work, env) as request:
                request('initialize', {'protocolVersion':'2025-06-18'})
                samples['mcpStartInitialize'].append((time.perf_counter()-start)*1000)
                request('notifications/initialized', {})
                for key in ['mcpFirstNodes','mcpWarmNodes']:
                    reply = timed(key, lambda: request('tools/call', {'name':'nodes','arguments':{'file':str(fixture),'node':'0:1'}}))
                    assert not reply['isError'] and json.loads(reply['content'][0]['text'])['document']['id'] == '0:1'
    return {'commit':subprocess.check_output(['git','rev-parse','HEAD'], cwd=ROOT, text=True).strip(),
            'dirty':bool(subprocess.check_output(['git','status','--porcelain','--untracked-files=normal'], cwd=ROOT)),
            'platform':platform.platform(), 'machine':platform.machine(), 'python':platform.python_version(),
            'binary':str(binary), 'binaryVersion':version, 'binarySha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
            'fixture':str(fixture.relative_to(ROOT)), 'fixtureBytes':fixture.stat().st_size,
            'fixtureSha256':hashlib.sha256(fixture.read_bytes()).hexdigest(), 'repetitions':repetitions,
            'conditions':'fresh process except warm MCP nodes; isolated HOME/cache; OS file cache not flushed; no GCS',
            'costBoundaries':'version includes spawn; nodesProcess includes spawn/parse/tool; first MCP nodes includes parse/tool/IPC, warm includes tool/IPC. Differences do not isolate pure parsing.',
            'timings':{key:summarize(values) for key,values in samples.items()}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--repetitions', type=int, default=10)
    args = parser.parse_args()
    if not 3 <= args.repetitions <= 30:
        parser.error('repetitions must be 3-30')
    print(json.dumps(measure(args.binary.resolve(), args.repetitions), indent=2))


if __name__ == '__main__':
    main()
