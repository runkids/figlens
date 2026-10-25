import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from installation import environment, toolchain

ROOT = Path(__file__).resolve().parent.parent


class InstallationHarnessTest(unittest.TestCase):
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
