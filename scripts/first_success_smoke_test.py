import contextlib
import hashlib
import io
import json
from pathlib import Path
import runpy
import sys
import tempfile
import unittest
import zipfile

SMOKE = runpy.run_path(str(Path(__file__).with_name('first-success-smoke.py')))
EXAMPLE = Path(__file__).resolve().parent.parent / 'examples/first-success/design.fig'


class FirstSuccessSmokeTest(unittest.TestCase):
    def test_published_fixture_size_and_hash_match_its_provenance(self):
        data = EXAMPLE.read_bytes()
        provenance = EXAMPLE.with_name('README.md').read_text(encoding='utf-8')
        self.assertIn(format(len(data), ','), provenance)
        self.assertIn(hashlib.sha256(data).hexdigest(), provenance)

    def test_original_png_decodes_and_corrupt_or_wrong_size_data_fails(self):
        with zipfile.ZipFile(EXAMPLE) as source:
            data = source.read('images/first-success-tile')
        pixels = SMOKE['png'](data, (4, 4))
        self.assertEqual(len(pixels), 64)
        self.assertEqual(list(pixels[:4]), [255] * 4)
        self.assertEqual(list(pixels[4:8]), [27, 153, 139, 255])
        with self.assertRaisesRegex(ValueError, 'PNG size'):
            SMOKE['png'](data, (320, 180))
        corrupt = bytearray(data)
        corrupt[29] ^= 1
        with self.assertRaisesRegex(ValueError, 'checksum'):
            SMOKE['png'](corrupt, (4, 4))
        with self.assertRaisesRegex(ValueError, 'truncated'):
            SMOKE['png'](data[:20], (4, 4))

    def test_an_empty_reference_is_not_success(self):
        with self.assertRaisesRegex(ValueError, 'screen background'):
            SMOKE['check_pixels'](bytearray(320 * 180 * 4))

    def test_existing_zip_with_wrong_target_is_not_success(self):
        with tempfile.TemporaryDirectory() as work:
            path = Path(work) / 'wrong.zip'
            with zipfile.ZipFile(path, 'w') as bundle:
                for name in ['design/nodes.json', 'design/variables.json', 'assets.json',
                             'missing-assets.json', 'references/screen.png', 'context.md']:
                    bundle.writestr(name, '{}')
                bundle.writestr('manifest.json', json.dumps({
                    'format': 'figlens-bundle', 'version': 1,
                    'source': {'nodeId': '9:9'}, 'root': {'id': '9:9', 'name': 'Wrong'},
                }))
            with self.assertRaisesRegex(ValueError, 'manifest target'):
                SMOKE['validate_bundle'](path)

    def test_explicit_missing_or_wrong_binary_reports_checkpoint_a(self):
        with tempfile.TemporaryDirectory() as work:
            for binary in [str(Path(work) / 'absent'), sys.executable]:
                output = io.StringIO()
                with contextlib.redirect_stderr(output):
                    result = SMOKE['main'](['--binary', binary])
                self.assertEqual(result, 1)
                self.assertIn('FAIL A:', output.getvalue())


if __name__ == '__main__':
    unittest.main()
