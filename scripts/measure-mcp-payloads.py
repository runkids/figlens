#!/usr/bin/env python3
"""Measure complete stdio replies on explicit local scopes; results belong under samples/."""
import argparse
import json
import os
from pathlib import Path
import tempfile

from mcp_smoke import session


def sizes(result, wire_bytes):
    def encoded(value):
        return len(json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode())
    content = result.get('content', [])
    return {
        'wireBytes': wire_bytes,
        'textUtf8Bytes': sum(len(block.get('text', '').encode()) for block in content),
        'structuredJsonBytes': encoded(result['structuredContent']) if 'structuredContent' in result else 0,
        'imageBase64Bytes': sum(len(block.get('data', '')) for block in content if block.get('type') == 'image'),
    }


def main():
    assert sizes({'content': [{'type': 'image', 'data': 'YWJj'}]}, 100)['imageBase64Bytes'] == 4
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('scopes', nargs='+', help='pairs of local .fig path and exact node ID')
    args = parser.parse_args()
    if len(args.scopes) % 2:
        parser.error('scopes must be file/node pairs')
    binary = args.binary.resolve(strict=True)
    rows = []
    with tempfile.TemporaryDirectory(prefix='payload-', dir='samples') as temporary:
        with session([binary, 'mcp'], Path.cwd(), os.environ.copy(), timeout=120) as request:
            version = request('initialize', {'protocolVersion': '2025-06-18'})['serverInfo']['version']
            for index in range(0, len(args.scopes), 2):
                file, node = args.scopes[index:index + 2]
                for tool, extra in [('inspect', {'depth': 0}), ('nodes', {}),
                                    ('export', {'out': str(Path(temporary) / f'{index}.png'), 'maxSize': 400})]:
                    result = request('tools/call', {'name': tool, 'arguments': {'file': file, 'node': node, **extra}})
                    assert not result.get('isError'), result
                    row = {'file': file, 'node': node, 'tool': tool, **sizes(result, request.last_wire_bytes)}
                    if 'structuredContent' in result and tool in ('inspect', 'nodes'):
                        assert json.loads(result['content'][0]['text']) == result['structuredContent']
                    assert request('ping', {}) == {}
                    row['subsequentPing'] = True
                    rows.append(row)
    print(json.dumps({'version': version, 'rows': rows}, indent=2))


if __name__ == '__main__':
    main()
