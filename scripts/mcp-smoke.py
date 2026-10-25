#!/usr/bin/env python3
"""Exercise a real JSON-lines stdio MCP child; no sockets or model calls."""
import json
import os
import queue
import signal
import subprocess
import threading


def check_mcp(command, cwd, env, fixture, version):
    process = subprocess.Popen(list(map(str, command)), cwd=cwd, env=env, text=True,
                               encoding='utf-8', stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=os.name != 'nt')
    lines, errors = queue.Queue(), []
    def read_stdout():
        for line in process.stdout:
            lines.put(line)
        lines.put(None)
    def read_stderr():
        errors.extend(process.stderr)
    readers = [threading.Thread(target=read_stdout, daemon=True), threading.Thread(target=read_stderr, daemon=True)]
    for reader in readers:
        reader.start()
    next_id = 0
    def request(method, params):
        nonlocal next_id
        next_id += 1
        process.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': next_id, 'method': method, 'params': params}) + '\n')
        process.stdin.flush()
        try:
            line = lines.get(timeout=30)
        except queue.Empty as error:
            raise TimeoutError(f'MCP {method} did not respond within 30 seconds') from error
        assert line is not None, 'MCP exited before replying: ' + ''.join(errors)
        reply = json.loads(line)  # An installer/log line on stdout is a failure.
        assert reply.get('jsonrpc') == '2.0' and reply.get('id') == next_id, reply
        assert 'error' not in reply and 'result' in reply, reply
        return reply['result']
    try:
        initialized = request('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {},
                                             'clientInfo': {'name': 'figlens-install-smoke', 'version': '1'}})
        assert initialized['protocolVersion'] == '2025-06-18'
        assert initialized['serverInfo']['version'] == version, initialized
        process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
        process.stdin.flush()
        tools = request('tools/list', {})['tools']
        assert {'find', 'inspect', 'nodes', 'bundle', 'export', 'workflow'} <= {t['name'] for t in tools}
        found = request('tools/call', {'name': 'find', 'arguments': {'file': str(fixture), 'query': 'First success', 'type': 'FRAME'}})
        assert found['isError'] is False and found['structuredContent']['results'][0]['id'] == '0:1', found
        failed = request('tools/call', {'name': 'inspect', 'arguments': {'file': str(fixture), 'node': '9:9'}})
        assert failed['isError'] is True and 'not found' in failed['content'][0]['text'], failed
        assert request('ping', {}) == {}
        normal = request('tools/call', {'name': 'inspect', 'arguments': {'file': str(fixture), 'node': '0:1'}})
        assert normal['isError'] is False and normal['structuredContent']['node']['name'] == 'First success', normal
        assert normal['structuredContent']['node']['absoluteBoundingBox']['width'] == 320
        guide = request('tools/call', {'name': 'workflow', 'arguments': {'topic': 'build'}})
        assert guide['isError'] is False and 'Building a frame' in guide['content'][0]['text']
    finally:
        try:
            process.stdin.close()
        except BrokenPipeError:
            pass
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            if os.name == 'nt':
                subprocess.run(['taskkill', '/PID', str(process.pid), '/T', '/F'], capture_output=True, timeout=10)
            else:
                os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=10)
        for reader in readers:
            reader.join(timeout=5)
        process.stdout.close()
        process.stderr.close()
    assert process.returncode == 0, ''.join(errors)
    assert lines.get(timeout=5) is None, 'Unexpected extra stdout after final MCP response'
    return {'requests': next_id, 'recoveredAfterMissingNode': True, 'stdout': 'JSON-RPC only'}
