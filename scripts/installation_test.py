import csv
import json
import os
from pathlib import Path
import runpy
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch
from installation import environment, run, toolchain

ROOT = Path(__file__).resolve().parent.parent


class InstallationHarnessTest(unittest.TestCase):
    def test_cli_timeout_stops_the_wrapper_child(self):
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
                with self.assertRaises(subprocess.TimeoutExpired):
                    run([sys.executable, '-c', wrapper, child, heartbeat, pidfile],
                        root, os.environ.copy(), timeout=1)
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

    def test_mcp_rejects_log_lines_on_stdout(self):
        check = runpy.run_path(str(ROOT/'scripts/mcp-smoke.py'))['check_mcp']
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            env = environment(root, sys.executable)
            with self.assertRaises(ValueError):
                check([sys.executable, '-c', 'import sys; sys.stdin.readline(); print("installer progress", flush=True)'], root, env, root/'unused.fig', '0.0.0')

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
