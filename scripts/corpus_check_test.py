import contextlib
import io
from pathlib import Path
import runpy
import sys
import tempfile
import unittest
from unittest import mock

CHECK = runpy.run_path(str(Path(__file__).with_name('corpus-check.py')))


def run(*args):
    output = io.StringIO()
    with contextlib.redirect_stderr(output):
        result = CHECK['main']([str(a) for a in args])
    return result, output.getvalue()


class CorpusCheckTest(unittest.TestCase):
    def test_json_diff_names_each_changed_missing_and_unexpected_path(self):
        expected = {'a': 1, 'b': [1, 2], 'c': {'d': 'x'}}
        actual = {'a': 2, 'b': [1], 'c': {}, 'e': True}
        self.assertEqual(list(CHECK['json_diff'](expected, actual)), [
            '$.a: 1 -> 2', '$.b: length 2 -> 1', '$.c.d: missing', '$.e: unexpected'])

    def test_json_diff_reports_a_changed_value_type(self):
        self.assertEqual(list(CHECK['json_diff']({'v': True}, {'v': 1})), ['$.v: true -> 1'])

    def test_pixel_share_ignores_small_channel_moves_and_counts_larger_ones(self):
        base = bytes([10, 20, 30, 255] * 4)
        within = bytes([26, 20, 30, 255] + [10, 20, 30, 255] * 3)
        beyond = bytes([27, 20, 30, 255] + [10, 20, 30, 255] * 3)
        self.assertEqual(CHECK['pixel_share'](base, within), 0.0)
        self.assertEqual(CHECK['pixel_share'](base, beyond), 0.25)

    def test_missing_inputs_are_setup_errors_not_regressions(self):
        with tempfile.TemporaryDirectory() as work:
            result, output = run('--binary', Path(work) / 'absent', '--corpus', Path(work) / 'corpus.fig')
        self.assertEqual(result, 2)
        self.assertIn('SETUP: binary not found', output)

    def test_an_unrecorded_input_is_a_setup_error(self):
        # The public fixture is a valid .fig, but not the recorded corpus input.
        result, output = run('--binary', sys.executable, '--corpus', CHECK['PUBLIC'])
        self.assertEqual(result, 2)
        self.assertIn("input identity differs from the recorded one for ['corpus']", output)

    def test_a_case_without_recorded_baselines_is_a_setup_error(self):
        with tempfile.TemporaryDirectory() as work:
            baselines = Path(work)
            (baselines / 'expected.json').write_text('{"inputs": {"public": "", "corpus": ""}, "cases": {}}')
            # run_path returns a copy of the globals; patch the ones `check` reads.
            with mock.patch.dict(CHECK['check'].__globals__, BASELINES=baselines):
                result, output = run('--binary', sys.executable, '--corpus', CHECK['PUBLIC'])
        self.assertEqual(result, 2)
        self.assertIn("SETUP: no recorded baselines for ['first-success'", output)


if __name__ == '__main__':
    unittest.main()
