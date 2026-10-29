import csv
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import Mock, patch
from installation import environment, run, stop_process, toolchain
from mcp_smoke import check_mcp, session

ROOT = Path(__file__).resolve().parent.parent


class InstallationHarnessTest(unittest.TestCase):
    def test_host_profiles_require_both_runtime_and_sdk_paths(self):
        for argument in ['--opencode-bin', '--omp-sdk']:
            with self.subTest(argument=argument):
                result = subprocess.run([sys.executable, '-B', str(ROOT/'scripts/check-installation.py'),
                    '--artifacts', 'missing', '--commit', 'unused', argument, 'missing'],
                    text=True, capture_output=True, timeout=10)
                self.assertEqual(result.returncode, 2)
                self.assertIn('requires both', result.stderr)

    def test_cli_timeout_stops_the_wrapper_child(self):
        self.check_stops_wrapper_child(
            lambda command, cwd, env: run(command, cwd, env, timeout=1),
            subprocess.TimeoutExpired)

    def test_mcp_timeout_stops_the_wrapper_child(self):
        def check(command, cwd, env):
            with session(command, cwd, env, timeout=1) as request:
                request('ping', {})
        self.check_stops_wrapper_child(check, TimeoutError)

    def test_mcp_shutdown_timeout_stops_the_wrapper_child(self):
        def check(command, cwd, env):
            with session(command, cwd, env, timeout=1):
                self.wait_for_child(cwd)
        self.check_stops_wrapper_child(check, AssertionError)

    def test_mcp_body_failure_stops_the_wrapper_child(self):
        def check(command, cwd, env):
            with session(command, cwd, env):
                self.wait_for_child(cwd)
                raise ValueError('test-only body failure')
        self.check_stops_wrapper_child(check, ValueError)

    def wait_for_child(self, cwd):
        deadline = time.monotonic() + 2
        while not (cwd/'child.pid').exists() and time.monotonic() < deadline:
            time.sleep(.02)
        self.assertTrue((cwd/'child.pid').is_file(), 'Child did not start before cleanup')

    def check_stops_wrapper_child(self, action, expected_error):
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            heartbeat, pidfile = root/'heartbeat', root/'child.pid'
            child = '''import os, sys, time
from pathlib import Path
heartbeat, pidfile = map(Path, sys.argv[1:])
heartbeat.write_text('ready')
pidfile.write_text(str(os.getpid()))
end = time.monotonic() + 8
while time.monotonic() < end:
    heartbeat.write_text(str(time.monotonic_ns()))
    time.sleep(.02)
'''
            wrapper = '''import subprocess, sys
subprocess.run([sys.executable, '-c', sys.argv[1], *sys.argv[2:]], check=True)
'''
            started = time.monotonic()
            cleanup_needed = True
            try:
                with self.assertRaises(expected_error):
                    action([sys.executable, '-c', wrapper, child, heartbeat, pidfile],
                           root, os.environ.copy())
                self.assertTrue(pidfile.is_file(), 'Child did not start before timeout')
                self.assertLess(time.monotonic() - started, 4, 'Timeout waited for the orphan to exit')
                stopped = heartbeat.read_text()
                for _ in range(5):
                    time.sleep(.05)
                    self.assertEqual(heartbeat.read_text(), stopped, 'Child survived the wrapper timeout')
                cleanup_needed = False
                pid = pidfile.read_text()
                if os.name == 'nt':
                    tasks = subprocess.run(['tasklist', '/FI', f'PID eq {pid}', '/FO', 'CSV', '/NH'],
                                           capture_output=True, text=True, encoding='utf-8',
                                           errors='replace', check=True, timeout=5)
                    running = any(len(row) > 1 and row[1] == pid for row in csv.reader(tasks.stdout.splitlines()))
                else:
                    state = subprocess.run(['ps', '-p', pid, '-o', 'stat='],
                                           capture_output=True, text=True, timeout=5)
                    self.assertIn(state.returncode, [0, 1], state.stderr)
                    running = state.returncode == 0 and not state.stdout.lstrip().startswith('Z')
                self.assertFalse(running, 'Child is still running after timeout cleanup')
            except BaseException:
                # Only this test's finite child; the fixed helper already cleans successful runs.
                if cleanup_needed and pidfile.is_file() and time.monotonic() - started < 8:
                    try:
                        os.kill(int(pidfile.read_text()), signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                raise

    def test_cli_output_and_nonzero_exit_are_preserved(self):
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            self.assertEqual(run([sys.executable, '-c', 'print("ready")'], root, os.environ.copy()), 'ready\n')
            with self.assertRaisesRegex(RuntimeError, 'exited 7\noutput\n\nproblem'):
                run([sys.executable, '-c', 'import sys; print("output"); print("problem", file=sys.stderr); sys.exit(7)'],
                    root, os.environ.copy())

    def test_consumer_environment_drops_user_configuration_and_credentials(self):
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            with patch.dict(os.environ, {'OPENAI_API_KEY': 'test-only', 'FIGLENS_BIN': 'global',
                                         'NODE_PATH': 'repo/node_modules', 'NODE_OPTIONS': 'user-preload'}):
                env = environment(root, '/selected/node')
            for key in ['OPENAI_API_KEY', 'FIGLENS_BIN', 'NODE_PATH', 'NODE_OPTIONS']:
                self.assertNotIn(key, env)
            self.assertEqual(env['HOME'], str(root/'home'))
            self.assertEqual(env['XDG_CACHE_HOME'], str(root/'cache'))
            self.assertTrue(Path(env['npm_config_userconfig']).is_file())

    def test_wrong_commit_fails_before_install_or_download(self):
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            (root/'candidate.json').write_text(json.dumps({'commit': 'other'}))
            result = subprocess.run([sys.executable, '-B', str(ROOT/'scripts/check-installation.py'),
                                     '--artifacts', str(root), '--commit', 'expected'],
                                    text=True, capture_output=True, timeout=10)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('another commit', result.stderr)

    def test_windows_cleanup_reaps_child(self):
        for exited_during_taskkill in [False, True]:
            with self.subTest(exited_during_taskkill=exited_during_taskkill):
                process = Mock(pid=123)
                process.poll.side_effect = [None, 0, 0]
                failure = subprocess.CalledProcessError(1, 'taskkill') if exited_during_taskkill else None
                with patch('installation.os.name', 'nt'), patch('installation.subprocess.run', side_effect=failure) as taskkill:
                    stop_process(process)
                taskkill.assert_called_once_with(
                    ['taskkill', '/PID', '123', '/T', '/F'], stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL, check=True, timeout=10)
                process.kill.assert_not_called()
                process.wait.assert_called_once_with(timeout=10)

    def test_windows_cleanup_failure_still_reaps_child(self):
        process = Mock(pid=123)
        process.poll.return_value = None
        with patch('installation.os.name', 'nt'), patch('installation.subprocess.run', side_effect=subprocess.CalledProcessError(1, 'taskkill')):
            with self.assertRaises(subprocess.CalledProcessError):
                stop_process(process)
        process.kill.assert_called_once_with()
        process.wait.assert_called_once_with(timeout=10)

    def test_mcp_clean_shutdown(self):
        child = """import json, sys
for line in sys.stdin:
    request = json.loads(line)
    if 'id' in request:
        print(json.dumps({'jsonrpc': '2.0', 'id': request['id'],
                          'result': request['params']}), flush=True)
"""
        with tempfile.TemporaryDirectory() as work:
            with session([sys.executable, '-c', child], work, os.environ.copy(), timeout=1) as request:
                self.assertEqual(request('ping', {}), {})
                self.assertIsNone(request('notifications/initialized', {}))
                self.assertEqual(request('echo', {'ok': True}), {'ok': True})

    def test_mcp_rejects_extra_stdout_at_shutdown(self):
        child = "import sys; sys.stdin.read(); print('unexpected stdout', flush=True)"
        with tempfile.TemporaryDirectory() as work:
            with self.assertRaisesRegex(AssertionError, 'Unexpected extra stdout'):
                with session([sys.executable, '-c', child], work, os.environ.copy(), timeout=1):
                    pass

    def test_mcp_audit_cli_uses_bounded_runner(self):
        guide = '# Audit an existing UI\nStatic review is not visual acceptance'
        tools = [{'name': name} for name in ['find', 'search', 'diff', 'nodes', 'bundle', 'export', 'index', 'jobs', 'usages']]
        tools.extend([
            {'name': 'inspect', 'inputSchema': {'properties': {'depth': {'maximum': 10}}}},
            {'name': 'workflow', 'inputSchema': {'properties': {'topic': {'enum': ['audit']}}}},
        ])
        request = Mock(side_effect=[
            {'protocolVersion': '2025-06-18', 'serverInfo': {'version': '0.0.0'}},
            None,
            {'tools': tools},
            {'isError': False, 'structuredContent': {'results': [{'id': '0:1'}]}},
            {'isError': False, 'structuredContent': {
                'results': [{'ref': {'nodeId': '0:1'}}], 'coverage': {'complete': True}}},
            {'isError': False, 'structuredContent': {'node': {'id': '0:1'}}},
            {'isError': False, 'structuredContent': {'results': [{'target': {'ref': {'nodeId': '0:10'}}}]}},
            {'isError': False, 'structuredContent': {'coverage': {'complete': True, 'changes': 0}, 'source': {
                'old': {'sourceId': 'local:a', 'snapshotId': 's'}, 'new': {'sourceId': 'local:b', 'snapshotId': 's'}}}},
            {'isError': True, '_meta': {'figlens': {'code': 'node_not_found'}}},
            {'isError': True, '_meta': {'figlens': {'code': 'invalid_arguments'}}},
            {'isError': True, '_meta': {'figlens': {'code': 'invalid_arguments'}}},
            {},
            {'isError': False, 'structuredContent': {'node': {
                'name': 'First success', 'absoluteBoundingBox': {'width': 320}}}},
            {'isError': False, 'content': [{'text': 'Building a frame'}]},
            {'isError': False, 'content': [{'text': guide}]},
        ])
        command = [sys.executable, 'wrapper.py', 'mcp']
        with tempfile.TemporaryDirectory() as work:
            root, env = Path(work), os.environ.copy()
            with patch('mcp_smoke.session') as smoke, patch('mcp_smoke.run') as runner, \
                    patch('mcp_smoke.subprocess.check_output', return_value=guide) as unbounded:
                smoke.return_value.__enter__.return_value = request
                runner.return_value = guide
                (root/'design.fig').write_bytes(b'fig')
                result = check_mcp(command, root, env, root/'design.fig', '0.0.0')
                self.assertTrue(result['recoveredAfterMissingNode'])
                self.assertFalse((root/'diff-old.fig').exists())
                runner.assert_called_once_with([sys.executable, 'wrapper.py', 'workflow', 'audit'], root, env)
                unbounded.assert_not_called()

    def test_mcp_rejects_log_lines_on_stdout(self):
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            env = environment(root, sys.executable)
            with self.assertRaises(ValueError):
                check_mcp([sys.executable, '-c', 'import sys; sys.stdin.readline(); print("installer progress", flush=True)'], root, env, root/'unused.fig', '0.0.0')

    def test_candidate_transport_cannot_fall_back_to_another_release(self):
        node, _npm = toolchain()
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            feed = root/'feed.json'
            events = root/'events.jsonl'
            feed.write_text(json.dumps({'version': '1.2.3', 'archive': 'candidate.tar.gz',
                                        'directory': work, 'events': str(events)}))
            env = environment(root, node)
            env['FIGLENS_TEST_FEED'] = str(feed)
            result = subprocess.run([node, '--require', str(ROOT/'scripts/candidate-fetch.cjs'), '-e',
                                     'fetch("https://github.com/runkids/figlens/releases/download/v0.0.0/SHA256SUMS").then(()=>process.exit(9),()=>console.log("blocked"))'],
                                    env=env, text=True, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), 'blocked')
            self.assertFalse(events.exists())


if __name__ == '__main__':
    unittest.main()
