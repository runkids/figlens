import importlib.util
from contextlib import redirect_stdout
import io
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('plan_model', Path(__file__).with_name('check-pi-plan-model.py'))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class PlanningCompletionTest(unittest.TestCase):
    def test_actual_completion_is_required_and_project_changes_are_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'
            (source / 'project').mkdir(parents=True)
            (source / 'project/index.html').write_text('unchanged')
            (source / 'agent').mkdir()
            (source / 'agent/models.json').write_text(json.dumps({
                'providers': {'test': {'models': [{'id': 'fake'}]}},
            }))
            (source / 'manifest.json').write_text(json.dumps({'model': 'test/fake', 'sourceHashes': {}}))
            (source / 'request.txt').write_text('Plan only the icon.')
            fake = root / 'pi'
            binary = root / 'figlens'
            binary.write_bytes(b'fixture-binary')
            cases = [
                ('handled-only', [{'type': 'response', 'success': True}, {'type': 'agent_settled'}], False, 1),
                ('completed', [{'type': 'agent_start'}, {'type': 'message_end', 'message': {
                    'role': 'assistant', 'stopReason': 'stop', 'content': [{'type': 'text', 'text': 'Plan'}],
                }}, {'type': 'agent_settled'}], False, 0),
                ('changed', [{'type': 'agent_start'}, {'type': 'message_end', 'message': {
                    'role': 'assistant', 'stopReason': 'stop', 'content': [],
                }}, {'type': 'agent_settled'}], True, 1),
            ]
            for name, events, change, expected in cases:
                with self.subTest(name=name):
                    fake.write_text('#!/usr/bin/env python3\nimport json,sys\nfrom pathlib import Path\nsys.stdin.readline()\n' +
                                    ('Path("index.html").write_text("changed")\n' if change else '') +
                                    f'for event in {events!r}: print(json.dumps(event),flush=True)\n')
                    fake.chmod(0o755)
                    output = root / name
                    with patch.dict(os.environ, {'PATH': str(root) + os.pathsep + os.environ['PATH'], 'FIGLENS_BIN': str(binary)}), patch('sys.argv', ['check', str(source), str(output)]), redirect_stdout(io.StringIO()):
                        self.assertEqual(runner.main(), expected)
                    self.assertEqual(json.loads((output / 'manifest.json').read_text())['binary'], {
                        'path': str(binary.resolve()), 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                    })
                    result = json.loads((output / 'result.json').read_text())
                    self.assertEqual(result['fixtureFilesChanged'], ['index.html'] if change else [])
                    if name == 'completed':
                        self.assertEqual((output / 'plan.md').read_text(), 'Plan')


if __name__ == '__main__':
    unittest.main()
