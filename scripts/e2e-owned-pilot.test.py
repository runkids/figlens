"""Small checks for the experiment-only repeated fixture runner."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location('owned_pilot', Path(__file__).with_name('e2e-owned-pilot.py'))
pilot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pilot)
base_spec = importlib.util.spec_from_file_location('original_pilot', Path(__file__).with_name('e2e-pilot.py'))
base = importlib.util.module_from_spec(base_spec)
with patch.object(sys, 'argv', ['e2e-pilot.py', 'evaluate', '.']):
    base_spec.loader.exec_module(base)


class PilotTests(unittest.TestCase):
    def test_order_is_fixed_and_contains_only_the_twelve_authorized_attempts(self):
        order = pilot.make_order()
        self.assertEqual(order, pilot.make_order())
        self.assertEqual(len(order), 12)
        self.assertEqual(len(set(order)), 12)
        for arm in ['screenshot', 'official', 'figlens', 'checklist']:
            self.assertEqual(sorted(x for x in order if x.startswith(arm + '-')),
                             [f'{arm}-{i}' for i in range(1, 4)])

    def test_evaluation_counts_each_text_occurrence_once(self):
        cases = [
            (['Ready', 'Ready'], 'Ready', ['Ready']),
            (['Ready', 'Ready'], 'Ready\nReady', []),
            (['Ready', 'Ready later'], 'Ready later', ['Ready']),
            (['Ready', 'Ready later'], 'Ready later\nReady', []),
            (['Ready now', 'Ready now'], 'Ready\nnow', ['Ready now']),
        ]
        for expected, text, missing in cases:
            with self.subTest(expected=expected, text=text), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                design = root / 'figlens/design'
                design.mkdir(parents=True)
                (design / 'nodes.json').write_text(json.dumps({'document': {
                    'type': 'FRAME', 'children': [{'type': 'TEXT', 'characters': t} for t in expected],
                }}))
                run = root / 'runs/figlens-1'
                run.mkdir(parents=True)
                state = {'viewport': [962, 642], 'dpr': 1, 'text': text}
                pixels = {'mae255': 0, 'fractionMaxChannelOver48': 0, 'overlay': '', 'difference': ''}
                with patch.object(pilot, 'make_order', return_value=['figlens-1']), \
                        patch.object(pilot.subprocess, 'run') as command, \
                        patch.object(base, 'browser', side_effect=[state, pixels]):
                    command.return_value.stdout = command.return_value.stderr = ''
                    pilot.evaluate(root, base)
                saved = json.loads((run / 'browser-state.json').read_text())
                self.assertEqual(saved['missingText'], missing)
                self.assertEqual(saved['matchedTextNodes'], len(expected) - len(missing))

    def test_checklist_is_generic_not_the_fixture_answer(self):
        for answer in ['960', '640', '20px', '6056', 'FIELDNOTES', '2:2']:
            self.assertNotIn(answer, pilot.CHECKLIST)
        for concept in ['radius', 'stroke', 'text', 'browser']:
            self.assertIn(concept, pilot.CHECKLIST)


if __name__ == '__main__':
    unittest.main()
