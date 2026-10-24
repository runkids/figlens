#!/usr/bin/env python3
"""Repeat a read-only Pi planning case, waiting for actual model completion."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time


def hashes(directory):
    return {
        str(p.relative_to(directory)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(directory.rglob('*')) if p.is_file()
    }


def main():
    if len(sys.argv) != 3:
        raise SystemExit('Usage: check-pi-plan-model.py <previous-case> <new-output>')
    source, output = [Path(p).resolve() for p in sys.argv[1:]]
    root = Path(__file__).resolve().parent.parent
    previous = json.loads((source / 'manifest.json').read_text())
    profile = json.loads((source / 'agent/models.json').read_text())
    provider, model = previous['model'].split('/', 1)
    assert any(m['id'] == model for m in profile['providers'][provider]['models'])
    output.mkdir()  # Preserve earlier runs, including incomplete ones.
    shutil.copytree(source / 'project', output / 'project')
    (output / 'agent').mkdir()
    (output / 'agent/models.json').write_text(json.dumps(profile))
    request = (source / 'request.txt').read_text()
    (output / 'request.txt').write_text(request)
    before = hashes(output / 'project')
    manifest = dict(previous)
    manifest.update(sourceHashes={
        p: hashlib.sha256((root / p).read_bytes()).hexdigest()
        for p in previous['sourceHashes']
    }, fixtureHashes=before, repeatedFrom=str(source))
    env = dict(os.environ, PI_CODING_AGENT_DIR=str(output / 'agent'))
    binary = Path(env.get('FIGLENS_BIN', root / 'samples/release-v0.4.2/native/figlens')).resolve()
    env['FIGLENS_BIN'] = str(binary)
    manifest['binary'] = {'path': str(binary), 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest()}
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    command = [
        'pi', '--offline', '--no-extensions', '-e', 'builtin:mcp',
        '-e', str(root / 'pi/index.ts'), '--no-context-files',
        '--no-prompt-templates', '--no-themes', '--skill', str(root / 'skills/figlens'),
        '--provider', provider, '--model', model, '--no-session', '--tools',
        'read,grep,find,ls,mcp__figlens__tree,mcp__figlens__find,mcp__figlens__inspect,mcp__figlens__nodes,mcp__figlens__variables',
        '--mode', 'rpc',
    ]
    started = time.time()
    settled = active = False
    turns = calls = 0
    final = {}
    with (output / 'stderr.log').open('w') as errors, (output / 'events.jsonl').open('w') as log:
        process = subprocess.Popen(command, cwd=output / 'project', env=env,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=errors, text=True, bufsize=1)
        (output / 'process.json').write_text(json.dumps({
            'pid': process.pid, 'startedAt': started, 'argv': command,
        }, indent=2) + '\n')
        process.stdin.write(json.dumps({
            'id': 'plan-regression', 'type': 'prompt',
            'message': '/figlens plan ' + request,
        }) + '\n')
        process.stdin.flush()
        for line in process.stdout:
            log.write(line)
            log.flush()
            event = json.loads(line)
            kind = event.get('type')
            active |= kind == 'agent_start'
            calls += kind == 'tool_execution_start'
            message = event.get('message', {})
            if kind == 'message_end' and message.get('role') == 'assistant':
                turns += 1
                final = message
            if active and kind == 'agent_settled':
                settled = True
                break
        process.stdin.close()
        process.wait()
        process.stdout.close()
    plan = '\n'.join(c.get('text', '') for c in final.get('content', []) if c.get('type') == 'text')
    (output / 'plan.md').write_text(plan)
    after = hashes(output / 'project')
    result = {
        'status': 'completed' if settled and plan.strip() and final.get('stopReason') == 'stop' and process.returncode == 0 else 'incomplete-or-error',
        'settled': settled, 'exitCode': process.returncode,
        'elapsedSeconds': round(time.time() - started, 2), 'model': previous['model'],
        'toolCalls': calls, 'assistantTurns': turns, 'stopReason': final.get('stopReason'),
        'finalPlanCharacters': len(plan),
        'fixtureFilesChanged': [p for p in sorted(before.keys() | after.keys()) if before.get(p) != after.get(p)],
    }
    (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))
    return 0 if result['status'] == 'completed' and not result['fixtureFilesChanged'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
