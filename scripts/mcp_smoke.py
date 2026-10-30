#!/usr/bin/env python3
"""Exercise a real JSON-lines stdio MCP child; no sockets or model calls."""
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import hashlib
import json
import os
import queue
import re
import shutil
import subprocess
import threading
import time
from pathlib import Path
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
        assert {'find', 'search', 'inspect', 'diff', 'nodes', 'bundle', 'export', 'workflow', 'index', 'jobs', 'usages'} <= {t['name'] for t in tools}
        found = request('tools/call', {'name': 'find', 'arguments': {'file': str(fixture), 'query': 'First success', 'type': 'FRAME'}})
        assert found['isError'] is False and found['structuredContent']['results'][0]['id'] == '0:1', found
        searched = request('tools/call', {'name': 'search', 'arguments': {'file': str(fixture), 'query': 'First success'}})
        assert searched['isError'] is False and searched['structuredContent']['results'][0]['ref']['nodeId'] == '0:1', searched
        assert searched['structuredContent']['coverage']['complete'] is True, searched
        # Optional local models through the same entry: listed and set up, never downloaded; without a
        # configured profile semantic fails mode_unavailable and hybrid answers with text alone.
        search_mode = next(t for t in tools if t['name'] == 'search')['inputSchema']['properties']['mode']['enum']
        assert {'text', 'semantic', 'hybrid'} <= set(search_mode), search_mode
        listed = request('tools/call', {'name': 'models', 'arguments': {'action': 'list'}})
        assert listed['isError'] is False and listed['structuredContent']['profiles'], listed
        status = request('tools/call', {'name': 'models', 'arguments': {'action': 'status'}})
        assert status['isError'] is False and status['structuredContent'].get('setup'), status
        assert isinstance(status['structuredContent'].get('configRoot'), str), status
        empty = Path(cwd) / 'no-model'
        empty.mkdir(exist_ok=True)
        refused = request('tools/call', {'name': 'models', 'arguments': {'action': 'configure', 'profile': 'bge-m3', 'modelDir': str(empty)}})
        assert refused['isError'] is True and ' is missing ' in json.dumps(refused) and 'no model runtime' not in json.dumps(refused), refused
        after = request('tools/call', {'name': 'models', 'arguments': {'action': 'status'}})
        assert after['structuredContent']['profiles'] == [], after
        semantic = request('tools/call', {'name': 'search', 'arguments': {'file': str(fixture), 'query': 'First success', 'mode': 'semantic'}})
        assert semantic['isError'] is True and semantic['_meta']['figlens']['code'] == 'mode_unavailable', semantic
        hybrid = request('tools/call', {'name': 'search', 'arguments': {'file': str(fixture), 'query': 'First success', 'mode': 'hybrid'}})
        assert hybrid['isError'] is False and hybrid['structuredContent']['query']['usedModes'] == ['text'], hybrid
        assert hybrid['structuredContent']['results'][0]['ref']['nodeId'] == '0:1', hybrid
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
        used = request('tools/call', {'name': 'usages', 'arguments': {'file': str(fixture), 'node': ref, 'relation': 'contains', 'direction': 'out'}})
        assert used['isError'] is False and used['structuredContent']['results'][0]['target']['ref']['nodeId'] == '0:10', used
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
    return {"recoveredAfterMissingNode": True, "searchedExactRef": True, "modelsWithoutProfile": True, "diffedTwoCopies": True, "numericPreflight": True, "strictRejectionAndRecovery": True, "stdout": "JSON-RPC only"}


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


def check_library(command, base, mapping, env):
    """An agent names a library reference from tool output, then asks where it is used: stateless
    and indexed outgoing usages of an instance agree on its cached copy's reference, and the
    library question finds every instance in both registered copies. `mapping` is the synthetic
    fixture generated with `--library`."""
    base.mkdir()
    env = user_roots(base, env)
    project = base/'project'
    project.mkdir()
    design, other = project/'mapping.fig', project/'mapping-copy.fig'
    shutil.copy2(mapping, design)
    shutil.copy2(mapping, other)
    with session(command, base, env) as request:
        request('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'figlens-library', 'version': '1'}})
        def call(name, arguments):
            reply = request('tools/call', {'name': name, 'arguments': arguments})
            assert reply['isError'] is False, reply
            return reply['structuredContent']
        def named(reply):
            return next((r['library'] for r in reply['results'] if 'library' in r), None)
        stateless = named(call('usages', {'file': str(design), 'node': '0:2', 'direction': 'out'}))
        assert stateless and stateless['namespace'] and stateless['key'], stateless
        workspace, first = None, None
        for f in (design, other):
            job = call('index', {'action': 'prepare', 'workspace': str(project), 'file': str(f), 'scope': '0:0'})
            assert workspace in (None, job['workspaceId']), job
            first = first or job['job']['sourceId']
            workspace, job = job['workspaceId'], job['job']
            deadline = time.monotonic() + 60
            while job['state'] not in TERMINAL and time.monotonic() < deadline:
                time.sleep(0.05)
                job = call('jobs', {'action': 'status', 'jobId': job['jobId']})
            assert job['state'] == 'succeeded', job
        indexed = named(call('usages', {'workspaceId': workspace, 'sourceId': first, 'node': '0:2', 'direction': 'out'}))
        assert indexed == stateless, (indexed, stateless)
        asked = {k: stateless[k] for k in ('kind', 'namespace', 'key')}
        used = call('usages', {'workspaceId': workspace, 'library': asked})
        refs = sorted((r['ref']['sourceId'], r['ref']['nodeId']) for r in used['results'])
        sources = sorted({s for s, _ in refs})
        assert len(sources) == 2 and refs == [(s, n) for s in sources for n in ('0:2', '0:3')], used
        assert used['coverage']['complete'] is True and used['definition']['state'] == 'unresolved', used
    return {'named': stateless, 'usedBy': [n for _, n in refs], 'sources': len(sources)}


DIRECTORY_FILES = 11  # one page more than an add registers, so the cursor is followed once


def check_directory(command, base, fixture, env):
    """From clean roots, an agent prepares one design of a project, then registers the project's
    design directory through the same `index` tool, following nextCursor until none is left; the
    replies' sources are exactly the registered ones, at their canonical paths."""
    base.mkdir()
    env = user_roots(base, env)
    designs = base/'project/designs'
    designs.mkdir(parents=True)
    files = [designs/f'd{n:02}.fig' for n in range(1, DIRECTORY_FILES + 1)]
    for path in files:
        shutil.copy2(fixture, path)
    with session(command, base, env) as request:
        request('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'figlens-directory', 'version': '1'}})
        def call(name, arguments):
            reply = request('tools/call', {'name': name, 'arguments': arguments})
            assert reply['isError'] is False, reply
            return reply['structuredContent']
        prepared = call('index', {'action': 'prepare', 'workspace': str(base/'project'), 'file': str(files[0]), 'scope': '0:0'})
        workspace, job = prepared['workspaceId'], prepared['job']
        deadline = time.monotonic() + 60
        while job['state'] not in TERMINAL and time.monotonic() < deadline:
            time.sleep(0.05)
            job = call('jobs', {'action': 'status', 'jobId': job['jobId']})
        assert job['state'] == 'succeeded', job
        pages, cursor = [], None
        while len(pages) < 3:
            add = {'action': 'add', 'workspaceId': workspace, 'file': str(designs)}
            pages.append(call('index', dict(add, cursor=cursor) if cursor else add)['directory'])
            cursor = pages[-1]['nextCursor']
            if cursor is None:
                break
        status = call('index', {'action': 'status', 'workspaceId': workspace})
    listed = [s for page in pages for s in page['existing'] + page['added']]
    assert cursor is None and [len(p['existing'] + p['added']) for p in pages] == [10, 1], pages
    # The prepared design is found registered with its id, never added twice.
    assert [s['sourceId'] for s in pages[0]['existing']] == [prepared['job']['sourceId']], pages[0]
    # samefile, since Windows reports canonical paths with a \\?\ prefix realpath leaves off.
    assert len(listed) == len(files) and all(os.path.samefile(s['path'], f) for s, f in zip(listed, files)), listed
    assert sorted((s['sourceId'], s['path']) for s in status['sources']) == sorted((s['sourceId'], s['path']) for s in listed), status
    return {'workspaceId': workspace, 'pages': [len(p['added']) for p in pages], 'sources': len(status['sources'])}


def check_question(command, base, mapping, other, env):
    """A saved text question over a fixed set of two sources: subscribe reads both sources' exact refs,
    `changes` returns that first evaluation, and after one member's design is replaced and refreshed,
    `evaluate` names only that member and the ref that left with it. `other` lacks the frame."""
    base.mkdir()
    env = user_roots(base, env)
    project = base/'project'
    project.mkdir()
    a, b = project/'a.fig', project/'b.fig'
    shutil.copy2(mapping, a)
    shutil.copy2(mapping, b)
    with session(command, base, env) as request:
        request('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'figlens-question', 'version': '1'}})
        def call(name, arguments):
            reply = request('tools/call', {'name': name, 'arguments': arguments})
            assert reply['isError'] is False, reply
            return reply['structuredContent']
        def settle(job):
            deadline = time.monotonic() + 60
            while job['state'] not in TERMINAL and time.monotonic() < deadline:
                time.sleep(0.05)
                job = call('jobs', {'action': 'status', 'jobId': job['jobId']})
            assert job['state'] == 'succeeded', job
        sources = []
        for f in (a, b):
            prepared = call('index', {'action': 'prepare', 'workspace': str(project), 'file': str(f), 'scope': '0:0'})
            settle(prepared['job'])
            workspace = prepared['workspaceId']
            sources.append(prepared['job']['sourceId'])
        saved = call('watch', {'action': 'subscribe', 'workspaceId': workspace, 'consumer': 'host-question',
                               'sources': ','.join(sources), 'query': 'Mapping review'})
        subscription = saved['subscription']['subscriptionId']
        first = call('changes', {'workspaceId': workspace, 'subscriptionId': subscription})
        shutil.copy2(other, b)
        for job in call('index', {'action': 'refresh', 'workspaceId': workspace, 'scope': '0:0'})['jobs']:
            settle(job)
        evaluated = call('watch', {'action': 'evaluate', 'workspaceId': workspace, 'subscriptionId': subscription})['evaluation']
    frame = lambda refs: sorted(r['sourceId'] for r in refs if r['nodeId'] == '0:1')
    assert saved['subscription']['sources'] == sorted(sources) and saved['subscription']['sourceId'] is None, saved
    assert first['evaluations'] and frame(first['evaluations'][0]['delta']['enteredTopK']) == sorted(sources), first
    assert 'source_changed' in evaluated['causes'] and evaluated['delta']['changedSourceIds'] == [sources[1]], evaluated
    assert frame(evaluated['delta']['leftTopK']) == [sources[1]] and frame(evaluated['delta']['enteredTopK']) == [], evaluated
    # The ref that left is exactly the one that entered first, its old snapshot included.
    exact = lambda refs: [r for r in refs if r['nodeId'] == '0:1' and r['sourceId'] == sources[1]]
    assert exact(evaluated['delta']['leftTopK']) == exact(first['evaluations'][0]['delta']['enteredTopK']), evaluated
    return {'workspaceId': workspace, 'sources': len(sources), 'changed': evaluated['delta']['changedSourceIds'], 'causes': evaluated['causes']}


def check_first_use(command, base, fixture, env):
    """First use from a clean per-user state through a real MCP command, as an agent host runs it:
    prepare, poll, search the published index, inspect, a watch lease its input end releases, then
    restart, concurrency, cleanup and storage failures. `base` is a fresh directory holding every root and design copy."""
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
        assert tools['changes']['annotations']['readOnlyHint'] is True, tools['changes']
        assert tools['watch']['annotations']['destructiveHint'] is True, tools['watch']
        call(first, 'find', {'file': str(design), 'query': 'First success'})
        call(first, 'search', {'file': str(design), 'query': 'First success'})
        contains = {'node': '0:1', 'relation': 'contains', 'direction': 'out'}
        call(first, 'usages', {'file': str(design), **contains})
        assert not state.exists(), 'connecting, listing, find, stateless search or usages created figlens state'
        # Two hosts' first searches at once: one workspace, one database, both jobs succeed.
        with ThreadPoolExecutor(max_workers=2) as pool:
            replies = list(pool.map(lambda request: call(request, 'index', prepare), (first, second)))
        workspace = replies[0]['workspaceId']
        assert {reply['workspaceId'] for reply in replies} == {workspace}, replies
        jobs = [settle(request, reply['job']) for request, reply in zip((first, second), replies)]
        assert len(list((state/'workspaces').glob('*/index.sqlite'))) == 1
        assert search(first, workspace) == ('ready', '0:1')
        used = call(first, 'usages', {'workspaceId': workspace, **contains})
        assert used['coverage']['complete'] and used['results'][0]['target']['ref']['nodeId'] == '0:10', used
        # Wiring only: the fixture records no library reference, so the question is unresolved and has no users.
        library = {'kind': 'component', 'namespace': 'lib', 'key': 'k1'}
        answer = call(first, 'usages', {'workspaceId': workspace, 'library': library})
        assert answer['library'] == library and answer['results'] == [], answer
        assert answer['definition'] == {'state': 'unresolved', 'reason': 'library_source_unproven'}, answer
        assert answer['coverage']['complete'] is True and [s['state'] for s in answer['coverage']['sources']] == ['ready'], answer
        # Tracking records a baseline; a consumer that acks it reads nothing new after it.
        for job in call(first, 'index', {'action': 'track', 'workspaceId': workspace, 'scope': '0:0'})['jobs']:
            settle(first, job)
        polled = call(second, 'changes', {'workspaceId': workspace, 'consumer': 'smoke'})
        assert [revision['kind'] for revision in polled['revisions']] == ['baseline'], polled
        assert call(second, 'watch', {'action': 'ack', 'workspaceId': workspace, 'consumer': 'smoke', 'cursor': polled['nextCursor']})['advanced'] is True
        assert call(first, 'changes', {'workspaceId': workspace, 'consumer': 'smoke'})['revisions'] == []
        # A saved query stores its first evaluation; evaluate is current until a publication, and changes reads it.
        source = polled['tracking'][0]['sourceId']
        saved = call(second, 'watch', {'action': 'subscribe', 'workspaceId': workspace, 'consumer': 'smoke',
                                       'sourceId': source, 'query': 'First success'})
        assert saved['evaluation']['state'] == 'evaluated' and saved['evaluation']['causes'] == ['initial'], saved
        subscription = saved['subscription']['subscriptionId']
        assert call(first, 'watch', {'action': 'evaluate', 'workspaceId': workspace, 'subscriptionId': subscription})['evaluation']['state'] == 'current'
        read = call(first, 'changes', {'workspaceId': workspace, 'subscriptionId': subscription})
        assert len(read['evaluations']) == 1 and read['evaluation']['state'] == 'current', read
        ref = call(first, 'search', {'workspaceId': workspace, 'query': 'First success'})['results'][0]['ref']['nodeId']
        assert call(first, 'inspect', {'file': str(design), 'node': ref})['node']['id'] == ref
        # A lease this host holds, which only its own input end releases.
        lease = call(first, 'watch', {'action': 'start', 'workspaceId': workspace, 'scope': '0:0'})
        assert lease['state'] == 'active', lease
        assert [l['mine'] for l in call(second, 'watch', {'action': 'status', 'workspaceId': workspace})['leases']
                if l['leaseId'] == lease['leaseId']] == [False], 'another host saw the lease as its own'
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
        ended = [l for l in call(request, 'watch', {'action': 'status', 'workspaceId': workspace})['leases'] if l['leaseId'] == lease['leaseId']]
        assert [(l['state'], l.get('reason')) for l in ended] == [('ended', 'released')], ended
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
        result = {'concurrentFirstUseJoined': True, 'cancelled': True, 'trackedBaselineAcked': True, 'watchLeaseReleasedOnInputEnd': True, 'frameScope': '0:1', 'restartReused': True, 'clearCacheKeptIndex': True, 'rebuild': True,
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
