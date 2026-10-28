#!/usr/bin/env python3
"""Elapsed time, child peak RSS and expansion shape of `figlens nodes` per scope."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


def run(binary, fixture, node):
    with tempfile.TemporaryFile() as out:
        start = time.perf_counter()
        process = subprocess.Popen([binary, 'nodes', fixture, node], stdout=out,
                                   stderr=subprocess.PIPE)
        # wait4 returns this child's own peak, unlike RUSAGE_CHILDREN's historical max.
        _, status, usage = os.wait4(process.pid, 0)
        elapsed = (time.perf_counter() - start) * 1000
        stderr = process.stderr.read().decode(errors='replace').strip()
        size = out.tell()
        out.seek(0)
        doc = json.load(out) if status == 0 else None
    result = {'node': node, 'exit': os.waitstatus_to_exitcode(status), 'elapsedMs': round(elapsed, 1),
              'peakRssKiB': usage.ru_maxrss, 'outputBytes': size}
    if doc is None:
        result['stderr'] = stderr[:300]
        return result
    total = expanded = nesting = 0
    stack = [doc['document']]
    while stack:
        n = stack.pop()
        total += 1
        if n['id'].startswith('I'):
            expanded += 1
            nesting = max(nesting, n['id'].count(';'))
        stack.extend(n.get('children', []))
    result.update({'nodes': total, 'expandedNodes': expanded, 'maxInstanceNesting': nesting,
                   'components': len(doc['components']), 'styles': len(doc['styles'])})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--fixture', required=True)
    parser.add_argument('nodes', nargs='+')
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve())
    for node in args.nodes:
        print(json.dumps(run(binary, args.fixture, node), ensure_ascii=False), flush=True)


if __name__ == '__main__':
    main()
