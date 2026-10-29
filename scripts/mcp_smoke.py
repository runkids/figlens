#!/usr/bin/env python3
"""Exercise a real JSON-lines stdio MCP child; no sockets or model calls."""
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import hashlib
import json
import os
import queue
import shutil
import subprocess
import threading
import time
from installation import run, stop_process


@contextmanager
def session(command, cwd, env, timeout=30):
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
        if method.startswith('notifications/'):
            process.stdin.write(json.dumps({'jsonrpc':'2.0','method':method,'params':params}) + '\n')
            process.stdin.flush()
            return None
        process.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': next_id, 'method': method, 'params': params}) + '\n')
        process.stdin.flush()
        try:
            line = lines.get(timeout=timeout)
        except queue.Empty as error:
            raise TimeoutError(f'MCP {method} did not respond within {timeout:g} seconds') from error
        assert line is not None, 'MCP exited before replying: ' + ''.join(errors)
        request.last_wire_bytes = len(line.encode('utf-8'))
        reply = json.loads(line)  # An installer/log line on stdout is a failure.
        assert reply.get('jsonrpc') == '2.0' and reply.get('id') == next_id, reply
        assert 'error' not in reply and 'result' in reply, reply
        return reply['result']
    request.pid = process.pid  # Optional Linux /proc observations for bounded benchmarks.
    try:
        yield request
    except BaseException:
        stop_process(process)
        raise
    finally:
        try:
            try:
                process.stdin.close()
            except BrokenPipeError:
                pass
            try:
                process.wait(timeout=min(timeout, 10))
            except subprocess.TimeoutExpired:
                stop_process(process)
        finally:
            for reader in readers:
                reader.join(timeout=5)
            process.stdout.close()
            process.stderr.close()
    assert process.returncode == 0, ''.join(errors)
    assert lines.get(timeout=5) is None, 'Unexpected extra stdout after final MCP response'


def check_mcp(command, cwd, env, fixture, version):
    with session(command, cwd, env) as request:
        initialized = request('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {},
                                             'clientInfo': {'name': 'figlens-install-smoke', 'version': '1'}})
        assert initialized['protocolVersion'] == '2025-06-18'
        assert initialized['serverInfo']['version'] == version, initialized
        request('notifications/initialized', {})
        tools = request('tools/list', {})['tools']
        assert {'find', 'search', 'inspect', 'diff', 'nodes', 'bundle', 'export', 'workflow', 'index', 'jobs'} <= {t['name'] for t in tools}
        found = request('tools/call', {'name': 'find', 'arguments': {'file': str(fixture), 'query': 'First success', 'type': 'FRAME'}})
        assert found['isError'] is False and found['structuredContent']['results'][0]['id'] == '0:1', found
        searched = request('tools/call', {'name': 'search', 'arguments': {'file': str(fixture), 'query': 'First success'}})
        assert searched['isError'] is False and searched['structuredContent']['results'][0]['ref']['nodeId'] == '0:1', searched
        assert searched['structuredContent']['coverage']['complete'] is True, searched
        ref = searched['structuredContent']['results'][0]['ref']['nodeId']
        opened = request('tools/call', {'name': 'inspect', 'arguments': {'file': str(fixture), 'node': ref}})
        assert opened['isError'] is False and opened['structuredContent']['node']['id'] == ref, opened
        # Wiring only: two copies of one file are two sources with no changes; the engine's
        # revised-pair semantics are tested in the crate.
        old = fixture.with_name('diff-old.fig')
        shutil.copyfile(fixture, old)
        try:
            diffed = request('tools/call', {'name': 'diff', 'arguments': {'old': str(old), 'file': str(fixture)}})
        finally:
            old.unlink()
        pair = diffed.get('structuredContent', {})
        assert diffed['isError'] is False and pair['coverage']['complete'] is True and pair['coverage']['changes'] == 0, diffed
        assert pair['source']['old']['sourceId'] != pair['source']['new']['sourceId'], diffed
        assert pair['source']['old']['snapshotId'] == pair['source']['new']['snapshotId'], diffed
        failed = request('tools/call', {'name': 'inspect', 'arguments': {'file': str(fixture), 'node': '9:9'}})
        assert failed['isError'] is True and failed['_meta']['figlens']['code'] == 'node_not_found', failed
        schema = next(t for t in tools if t['name'] == 'inspect')['inputSchema']
        assert schema['properties']['depth']['maximum'] == 10
        invalid = request('tools/call', {'name':'inspect', 'arguments':{'file':str(fixture),'node':'0:1','depth':1.5}})
        assert invalid['isError'] and invalid['_meta']['figlens']['code'] == 'invalid_arguments', invalid
        legacy = request('tools/call', {'name':'tree','arguments':{'file':str(fixture),'depth':'0'}})
        assert legacy['isError'] and legacy['_meta']['figlens']['code'] == 'invalid_arguments', legacy
        assert 'argumentMode' not in legacy['_meta']['figlens'], legacy
        assert request('ping', {}) == {}
        normal = request('tools/call', {'name': 'inspect', 'arguments': {'file': str(fixture), 'node': '0:1'}})
        assert normal['isError'] is False and normal['structuredContent']['node']['name'] == 'First success', normal
        assert normal['structuredContent']['node']['absoluteBoundingBox']['width'] == 320
        guide = request('tools/call', {'name': 'workflow', 'arguments': {'topic': 'build'}})
        assert guide['isError'] is False and 'Building a frame' in guide['content'][0]['text']
        workflow = next(t for t in tools if t['name'] == 'workflow')
        assert 'audit' in workflow['inputSchema']['properties']['topic']['enum']
        audit = request('tools/call', {'name': 'workflow', 'arguments': {'topic': 'audit'}})
        assert audit['isError'] is False and 'Static review is not visual acceptance' in audit['content'][0]['text']
        cli_audit = run([*command[:-1], 'workflow', 'audit'], cwd, env)
        assert '# Audit an existing UI' in cli_audit
        assert cli_audit.split('# Audit an existing UI', 1)[1].rstrip() == audit['content'][0]['text'].split('# Audit an existing UI', 1)[1].rstrip()
    return {"recoveredAfterMissingNode": True, "searchedExactRef": True, "diffedTwoCopies": True, "numericPreflight": True, "strictRejectionAndRecovery": True, "stdout": "JSON-RPC only"}


TERMINAL = {'succeeded', 'failed', 'cancelled', 'interrupted'}


def user_roots(base, env):
    """One user's per-user roots under base, as every host of that user sees them."""
    roots = {f'XDG_{kind}_HOME': str(base/kind.lower()) for kind in ['CONFIG', 'STATE', 'CACHE', 'DATA']}
    return dict(env, **roots, LOCALAPPDATA=str(base/'local'))


def check_reuse(command, base, env):
    """Another host's MCP command, for the same user, searches the index check_first_use enabled without preparing."""
    with session(command, base, user_roots(base, env)) as request:
        request('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'figlens-reuse', 'version': '1'}})
        status = request('tools/call', {'name': 'index', 'arguments': {'action': 'status', 'file': str(base/'design.fig')}})['structuredContent']
        assert status['state'] == 'initialized', status
        found = request('tools/call', {'name': 'search', 'arguments': {'workspaceId': status['workspaceId'], 'query': 'First success'}})
        assert found['structuredContent']['results'][0]['ref']['nodeId'] == '0:1', found
    return {'reusedWorkspace': status['workspaceId']}


def check_first_use(command, base, fixture, env):
    """First use from a clean per-user state through a real MCP command, as an agent host runs it:
    prepare, poll, search the published index, inspect, then restart, concurrency, cleanup and
    storage failures. `base` is a fresh directory holding every root and design copy."""
    base.mkdir()
    env = user_roots(base, env)
    # An absolute XDG_STATE_HOME takes precedence over %LOCALAPPDATA%, also on Windows.
    state = base/'state/figlens'
    design, other = base/'design.fig', base/'other.fig'
    for path in (design, other, base/'frame.fig'):
        shutil.copy2(fixture, path)
    digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    before = digest(design)
    def call(request, name, arguments, error=None):
        reply = request('tools/call', {'name': name, 'arguments': arguments})
        if error:
            assert reply['isError'] and reply['_meta']['figlens']['code'] == error, reply
            return reply['_meta']['figlens']
        assert reply['isError'] is False, reply
        return reply['structuredContent']
    def settle(request, job):
        deadline = time.monotonic() + 60
        while job['state'] not in TERMINAL and time.monotonic() < deadline:
            time.sleep(0.05)
            job = call(request, 'jobs', {'action': 'status', 'jobId': job['jobId']})
        assert job['state'] == 'succeeded', job
        return job
    def search(request, workspace):
        found = call(request, 'search', {'workspaceId': workspace, 'query': 'First success'})
        return found['sources'][0]['state'], found['results'][0]['ref']['nodeId'] if found['results'] else None
    prepare = {'action': 'prepare', 'file': str(design), 'scope': '0:0'}
    with session(command, base, env) as first, session(command, base, env) as second:
        for request in (first, second):
            request('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'figlens-first-use', 'version': '1'}})
        tools = {tool['name']: tool for tool in first('tools/list', {})['tools']}
        assert tools['index']['annotations']['destructiveHint'] is True and tools['index']['annotations']['readOnlyHint'] is False, tools['index']
        assert tools['jobs']['annotations']['readOnlyHint'] is False, tools['jobs']
        call(first, 'find', {'file': str(design), 'query': 'First success'})
        call(first, 'search', {'file': str(design), 'query': 'First success'})
        assert not state.exists(), 'connecting, listing, find or stateless search created figlens state'
        # Two hosts' first searches at once: one workspace, one database, both jobs succeed.
        with ThreadPoolExecutor(max_workers=2) as pool:
            replies = list(pool.map(lambda request: call(request, 'index', prepare), (first, second)))
        workspace = replies[0]['workspaceId']
        assert {reply['workspaceId'] for reply in replies} == {workspace}, replies
        jobs = [settle(request, reply['job']) for request, reply in zip((first, second), replies)]
        assert len(list((state/'workspaces').glob('*/index.sqlite'))) == 1
        assert search(first, workspace) == ('ready', '0:1')
        ref = call(first, 'search', {'workspaceId': workspace, 'query': 'First success'})['results'][0]['ref']['nodeId']
        assert call(first, 'inspect', {'file': str(design), 'node': ref})['node']['id'] == ref
        finished = call(first, 'jobs', {'action': 'cancel', 'jobId': jobs[0]['jobId']})
        assert finished['state'] == 'succeeded', finished
        prepared = call(first, 'index', dict(prepare, file=str(other)))
        other_workspace = prepared['workspaceId']
        settle(first, prepared['job'])
        # A small fixture may finish before the cancel lands; either way cancel reports a final state and the
        # source can be prepared again.
        stopped = call(second, 'jobs', {'action': 'cancel', 'jobId': call(second, 'index', dict(prepare, scope='0:1'))['job']['jobId']})
        assert stopped['state'] in {'cancelled', 'succeeded'}, stopped
        settle(second, call(second, 'index', dict(prepare, scope='0:1'))['job'])
        # A frame opt-in is searched at that scope; the default 0:0 is not prepared by it.
        framed = call(second, 'index', dict(prepare, file=str(base/'frame.fig'), scope='0:1'))
        settle(second, framed['job'])
        hit = call(second, 'search', {'workspaceId': framed['workspaceId'], 'node': '0:1', 'query': 'First success'})
        assert hit['coverage']['complete'] is True and hit['results'][0]['ref']['nodeId'] == '0:1', hit
        assert call(second, 'inspect', {'file': str(base/'frame.fig'), 'node': hit['results'][0]['ref']['nodeId']})['node']['id'] == '0:1'
    # A restarted host finds the published index without preparing again.
    with session(command, base, env) as request:
        request('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'figlens-first-use', 'version': '1'}})
        assert call(request, 'index', {'action': 'status', 'workspaceId': workspace})['state'] == 'initialized'
        assert search(request, workspace) == ('ready', '0:1')
        call(request, 'index', {'action': 'clear_cache', 'workspaceId': workspace})
        assert search(request, workspace) == ('ready', '0:1'), 'clear_cache removed the published index'
        for job in call(request, 'index', {'action': 'rebuild', 'workspaceId': workspace})['jobs']:
            settle(request, job)
        assert search(request, workspace) == ('ready', '0:1')
        incarnation = call(request, 'index', {'action': 'status', 'workspaceId': workspace})['incarnation']
        call(request, 'index', {'action': 'reset', 'workspaceId': workspace})
        assert call(request, 'index', {'action': 'status', 'workspaceId': workspace})['state'] == 'uninitialized'
        again = call(request, 'index', prepare)
        settle(request, again['job'])
        assert call(request, 'index', {'action': 'status', 'workspaceId': workspace})['incarnation'] != incarnation
        assert search(request, workspace) == ('ready', '0:1')
        assert call(request, 'index', {'action': 'status', 'workspaceId': other_workspace})['state'] == 'initialized', 'reset touched another workspace'
        # A remembered custom root that disappears is an error with recovery, never a second database.
        moved = base/'moved.fig'
        shutil.copy2(fixture, moved)
        custom = base/'custom'
        databases = len(list((state/'workspaces').glob('*/index.sqlite')))
        run([*command[:-1], 'index', 'prepare', moved, '--storage', custom], base, env)
        shutil.rmtree(custom)
        lost = call(request, 'index', {'action': 'status', 'file': str(moved)}, error='storage_unavailable')
        assert 'restore_storage' in json.dumps(lost['recovery']), lost
        assert len(list((state/'workspaces').glob('*/index.sqlite'))) == databases, 'a lost remembered root moved to the default state'
        result = {'concurrentFirstUseJoined': True, 'cancelled': True, 'frameScope': '0:1', 'restartReused': True, 'clearCacheKeptIndex': True, 'rebuild': True,
                  'resetReprepared': True, 'otherWorkspaceKept': True, 'missingRememberedRoot': 'storage_unavailable'}
        # chmod does not model a Windows ACL, and root ignores it.
        if os.name == 'nt' or os.geteuid() == 0:
            result['readOnlyStorage'] = 'skipped: needs a non-root POSIX run'
        else:
            locked = base/'locked'
            (locked/'state').mkdir(parents=True)
            (locked/'state').chmod(0o500)
            denied = base/'denied.fig'
            shutil.copy2(fixture, denied)
            try:
                env_locked = dict(env, XDG_STATE_HOME=str(locked/'state'))
                with session(command, base, env_locked) as blocked:
                    blocked('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'figlens-first-use', 'version': '1'}})
                    failed = call(blocked, 'index', {'action': 'prepare', 'file': str(denied), 'scope': '0:0'}, error='permission_denied')
            finally:
                (locked/'state').chmod(0o700)
            assert 'restore_access' in json.dumps(failed['recovery']), failed
            assert list((locked/'state').iterdir()) == [], 'a read-only state root was written'
            result['readOnlyStorage'] = 'permission_denied'
    assert digest(design) == before, 'indexing changed the source .fig'
    result['sourceUnchanged'] = True
    return result
