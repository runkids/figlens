#!/usr/bin/env python3
"""Verify GCS snapshot behavior with an owned gcloud stand-in; never contacts Google."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tempfile
import zipfile
import zlib
from installation import environment, run

ROOT = Path(__file__).resolve().parent.parent
session = runpy.run_path(str(ROOT/'scripts/mcp-smoke.py'))['session']


def check_gcs(binary, fixture):
    with tempfile.TemporaryDirectory(prefix='figlens-gcs-mock-') as directory:
        work = Path(directory)
        env = environment(work, sys.executable)
        mock = work/'bin'
        mock.mkdir()
        state, events = work/'state.json', work/'events.jsonl'
        state.write_text(json.dumps({'generation':'101','fixture':str(fixture),'fail':False}))
        script = mock/'gcloud'
        script.write_text(f'#!{sys.executable}\n' + '''import json, os, shutil, sys
from pathlib import Path
state = json.loads(Path(os.environ['FIGLENS_MOCK_STATE']).read_text())
with open(os.environ['FIGLENS_MOCK_EVENTS'], 'a') as out:
    out.write(json.dumps(sys.argv[1:]) + '\\n')
if sys.argv[1:4] == ['storage','objects','describe']:
    if state['fail']:
        sys.exit('test-only error https://invalid.test/?signature=never-echo-this')
    print(state['generation'])
elif sys.argv[1:3] == ['storage','cp']:
    shutil.copyfile(state['fixture'], sys.argv[-1])
else:
    sys.exit('Unexpected mock command')
''', encoding='utf-8')
        script.chmod(0o700)
        if os.name == 'nt':
            (mock/'gcloud.cmd').write_text(f'@"{sys.executable}" "{script}" %*\n')
        env.update(PATH=str(mock)+os.pathsep+env['PATH'], FIGLENS_MOCK_STATE=str(state), FIGLENS_MOCK_EVENTS=str(events))
        assert Path(shutil.which('gcloud.cmd' if os.name == 'nt' else 'gcloud', path=env['PATH'])).resolve() == (mock/('gcloud.cmd' if os.name == 'nt' else 'gcloud')).resolve()
        uri = 'gs://example-design-handoff/designs/first-success.fig'
        request_args = {'file':uri, 'node':'0:1'}
        def calls():
            return [json.loads(line) for line in events.read_text().splitlines()] if events.exists() else []
        def change(**values):
            current = json.loads(state.read_text())
            state.write_text(json.dumps(dict(current, **values)))
        with session([binary, 'mcp'], work, env) as request:
            request('initialize', {'protocolVersion':'2025-06-18'})
            request('notifications/initialized', {})
            for _ in range(2):
                reply = request('tools/call', {'name':'inspect','arguments':request_args})
                assert not reply['isError'] and reply['structuredContent']['node']['name'] == 'First success', reply
            assert sum(c[:2] == ['storage','cp'] for c in calls()) == 1
            assert sum(c[:3] == ['storage','objects','describe'] for c in calls()) == 2
            cache = work/'cache/figlens'/hashlib.sha256(uri.encode()).digest()[:16].hex()
            assert (cache/'101.fig').read_bytes() == fixture.read_bytes()
            # Mutate only the known fixture's same-length frame label, not a general .fig writer.
            fresh = work/'fresh.fig'
            with zipfile.ZipFile(fixture) as source, zipfile.ZipFile(fresh, 'w') as target:
                for entry in source.infolist():
                    data = source.read(entry.filename)
                    if entry.filename == 'canvas.fig':
                        schema_end = 16 + int.from_bytes(data[12:16], 'little')
                        end = schema_end + 4 + int.from_bytes(data[schema_end:schema_end+4], 'little')
                        message = zlib.decompress(data[schema_end+4:end], -15)
                        assert message.count(b'First success') == 1
                        compressor = zlib.compressobj(wbits=-15)
                        encoded = compressor.compress(message.replace(b'First success', b'Fresh success')) + compressor.flush()
                        data = data[:schema_end] + len(encoded).to_bytes(4, 'little') + encoded + data[end:]
                    target.writestr(entry, data)
            change(generation='102', fixture=str(fresh))
            reply = request('tools/call', {'name':'inspect','arguments':request_args})
            assert not reply['isError'] and reply['structuredContent']['node']['name'] == 'Fresh success', reply
            assert not (cache/'101.fig').exists() and (cache/'102.fig').read_bytes() == fresh.read_bytes()
            change(fail=True)
            reply = request('tools/call', {'name':'inspect','arguments':request_args})
            assert reply['isError'] and reply['_meta']['figlens']['code'] == 'source_unreadable', reply
            assert 'never-echo-this' not in json.dumps(reply), reply
            cli = subprocess.run([str(binary), 'tree', uri, '--depth', '0'], cwd=work, env=env,
                                 text=True, capture_output=True, timeout=30)
            assert cli.returncode != 0 and 'gcloud' in cli.stderr
            assert 'never-echo-this' not in cli.stdout + cli.stderr
            assert request('ping', {}) == {}
            change(fail=False)
            assert not request('tools/call', {'name':'inspect','arguments':request_args})['isError']
        before = calls()
        with session([binary, 'mcp', '--root', work], work, env) as request:
            reply = request('tools/call', {'name':'inspect','arguments':request_args})
            assert reply['isError'] and reply['_meta']['figlens']['code'] == 'permission_denied', reply
            assert request('ping', {}) == {}
        assert calls() == before, '--root rejection must not invoke gcloud'
        assert sum(c[:2] == ['storage','cp'] for c in calls()) == 2
        assert sum(c[:3] == ['storage','objects','describe'] for c in calls()) == 6
        return {'transport':'owned gcloud mock, no network', 'metadataChecks':6, 'downloads':2,
                'generationChange':True, 'updatedContent':True, 'metadataFailureDespiteCache':True, 'rootRefusedBeforeGcloud':True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(check_gcs(args.binary.resolve(), ROOT/'examples/first-success/design.fig'), indent=2))


if __name__ == '__main__':
    try:
        main()
    except (AssertionError, OSError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(f'GCS mock smoke failed: {error}')
