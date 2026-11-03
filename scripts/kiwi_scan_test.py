import contextlib
import io
from pathlib import Path
import runpy
import unittest

HERE = Path(__file__).resolve().parent
SCAN = runpy.run_path(str(HERE / 'kiwi-scan.py'))
FIXTURE = str(HERE.parent / 'examples' / 'first-success' / 'design.fig')


def run(*args):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        SCAN['main'](list(args))
    return out.getvalue()


class KiwiScanTest(unittest.TestCase):
    def test_defs_lists_a_named_type_with_its_fields(self):
        self.assertRegex(run('defs', FIXTURE, 'NodeChange'), r'MESSAGE NodeChange \{ .*name: string')

    def test_defs_field_names_every_type_declaring_it(self):
        self.assertIn('NodeChange.guid: GUID', run('defs', FIXTURE, '--field', 'guid').splitlines())

    def test_scan_counts_nodes_carrying_a_field(self):
        out = run('scan', FIXTURE, '--field', 'name')
        nodes = int(out.split(': ')[1].split()[0])
        self.assertIn(f'  name: {nodes}', out.splitlines())


if __name__ == '__main__':
    unittest.main()
