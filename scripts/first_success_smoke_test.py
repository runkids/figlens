import contextlib
import hashlib
import io
import json
from pathlib import Path
import runpy
import struct
import sys
import tempfile
import unittest
import zipfile
import zlib

SMOKE = runpy.run_path(str(Path(__file__).with_name('first-success-smoke.py')))
EXAMPLE = Path(__file__).resolve().parent.parent / 'examples/first-success/design.fig'


def png_chunk(kind, payload):
    return (struct.pack('>I', len(payload)) + kind + payload
            + struct.pack('>I', zlib.crc32(kind + payload) & 0xffffffff))


def make_png(width, height, depth, color, raw, palette=None, transparency=None,
             interlace=0, compressed=None):
    data = b'\x89PNG\r\n\x1a\n' + png_chunk(
        b'IHDR', struct.pack('>IIBBBBB', width, height, depth, color, 0, 0, interlace))
    if palette is not None:
        data += png_chunk(b'PLTE', palette)
    if transparency is not None:
        data += png_chunk(b'tRNS', transparency)
    return (data + png_chunk(b'IDAT', zlib.compress(raw) if compressed is None else compressed)
            + png_chunk(b'IEND', b''))


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

    def test_packed_indexed_and_grayscale_depths_and_transparency(self):
        for depth in (1, 2, 4, 8):
            maximum = (1 << depth) - 1
            samples = [0, maximum, 1, maximum // 2, 0]
            row = bytearray((len(samples) * depth + 7) // 8)
            for x, sample in enumerate(samples):
                row[x * depth // 8] |= sample << (8 - depth - x * depth % 8)
            # Sub's left byte is one packed byte, not one sample or four RGBA bytes.
            sub = bytes((value - (row[x - 1] if x else 0)) & 255 for x, value in enumerate(row))
            raw = b'\0' + row + b'\1' + sub
            palette = bytes(channel for i in range(maximum + 1) for channel in (i, 255 - i, 73))
            for color in (0, 3):
                with self.subTest(depth=depth, color=color):
                    if color == 3:
                        data = make_png(5, 2, depth, color, raw, palette, b'\0\x7f')
                        expected = bytes(channel for i in samples
                                         for channel in (i, 255 - i, 73, 0 if i == 0 else 127 if i == 1 else 255))
                    else:
                        data = make_png(5, 2, depth, color, raw, transparency=struct.pack('>H', maximum))
                        expected = bytes(channel for i in samples for channel in
                                         (i * 255 // maximum,) * 3 + (0 if i == maximum else 255,))
                    self.assertEqual(SMOKE['png'](data, (5, 2)), expected * 2)

    def test_rgb_gray_alpha_and_rgba_filters_preserve_exact_channels(self):
        for color, channels in ((2, 3), (4, 2), (6, 4)):
            with self.subTest(color=color):
                rows = [bytes((x * 37 + y * 53) % 256 for x in range(3 * channels)) for y in range(5)]
                raw = bytearray()
                previous = bytes(3 * channels)
                for mode, row in enumerate(rows):
                    raw.append(mode)
                    for x, value in enumerate(row):
                        left = row[x - channels] if x >= channels else 0
                        above = previous[x]
                        upper_left = previous[x - channels] if x >= channels else 0
                        if mode == 4:
                            p = left + above - upper_left
                            prediction = min((left, above, upper_left), key=lambda v: abs(p - v))
                        else:
                            prediction = (0, left, above, (left + above) // 2)[mode]
                        raw.append((value - prediction) & 255)
                    previous = row
                expected = bytearray()
                for row in rows:
                    for x in range(3):
                        sample = row[x * channels:(x + 1) * channels]
                        expected.extend(sample + b'\xff' if color == 2 else
                                        bytes((sample[0],) * 3 + (sample[1],)) if color == 4 else sample)
                self.assertEqual(SMOKE['png'](make_png(3, 5, 8, color, raw), (3, 5)), expected)

    def test_rgb_transparency_preserves_hidden_rgb_and_matches_all_channels(self):
        rgb = b'\x0b\x16\x21'
        raw = b'\0' + rgb + b'\x0b\x16\x22'
        data = make_png(2, 1, 8, 2, raw, transparency=struct.pack('>HHH', 11, 22, 33))
        self.assertEqual(SMOKE['png'](data, (2, 1)), rgb + b'\0\x0b\x16\x22\xff')
        for color, raw, expected in (
                (4, b'\0\x49\0\x23\x7f', b'\x49\x49\x49\0\x23\x23\x23\x7f'),
                (6, b'\0\x0b\x16\x21\0\x01\x02\x03\x7f', b'\x0b\x16\x21\0\x01\x02\x03\x7f')):
            with self.subTest(color=color):
                self.assertEqual(SMOKE['png'](make_png(2, 1, 8, color, raw), (2, 1)), expected)

    def test_malformed_palette_and_transparency_are_rejected(self):
        cases = [
            (3, 1, None, None),
            (3, 1, b'', None),
            (3, 1, b'\x01\x02', None),
            (3, 1, b'\0' * 9, None),
            (3, 8, b'\0' * 771, None),
            (3, 1, b'\0' * 3, b''),
            (3, 1, b'\0' * 3, b'\0\x7f'),
            (0, 1, b'\0' * 3, None),
            (0, 1, None, b'\0'),
            (0, 1, None, b'\0\x02'),
            (2, 8, None, b'\0' * 5),
            (2, 8, None, struct.pack('>HHH', 256, 0, 0)),
            (4, 8, None, b'\0\0'),
            (6, 8, None, b'\0\0'),
        ]
        for color, depth, palette, transparency in cases:
            with self.subTest(color=color, depth=depth, palette=palette, transparency=transparency):
                data = make_png(1, 1, depth, color, b'\0\0', palette, transparency)
                with self.assertRaisesRegex(ValueError, 'palette|transparency'):
                    SMOKE['png'](data, (1, 1))
        with self.assertRaisesRegex(ValueError, 'palette index'):
            SMOKE['png'](make_png(1, 1, 1, 3, b'\0\x80', b'\0' * 3), (1, 1))

    def test_palette_transparency_order_duplicates_and_truncation_are_rejected(self):
        header = b'\x89PNG\r\n\x1a\n' + png_chunk(b'IHDR', struct.pack('>IIBBBBB', 1, 1, 1, 3, 0, 0, 0))
        palette = png_chunk(b'PLTE', b'\0' * 3)
        transparency = png_chunk(b'tRNS', b'\x7f')
        pixels = png_chunk(b'IDAT', zlib.compress(b'\0\0'))
        end = png_chunk(b'IEND', b'')
        for chunks in (transparency + palette + pixels,
                       palette + palette + pixels,
                       palette + transparency + transparency + pixels,
                       pixels + palette,
                       palette + pixels + transparency):
            with self.subTest(chunks=chunks):
                with self.assertRaisesRegex(ValueError, 'palette|transparency'):
                    SMOKE['png'](header + chunks + end, (1, 1))
        for chunk in (palette, transparency):
            with self.subTest(chunk=chunk):
                with self.assertRaisesRegex(ValueError, 'truncated'):
                    SMOKE['png'](header + chunk[:-1], (1, 1))

    def test_unsupported_forms_invalid_filters_and_pixel_stream_boundaries(self):
        for depth, color, interlace in ((16, 0, 0), (16, 2, 0), (16, 4, 0), (16, 6, 0),
                                        (2, 2, 0), (4, 4, 0), (1, 6, 0), (8, 1, 0), (8, 6, 1)):
            with self.subTest(depth=depth, color=color, interlace=interlace):
                with self.assertRaisesRegex(ValueError, 'supports non-interlaced'):
                    SMOKE['png'](make_png(1, 1, depth, color, b'\0' * 5, interlace=interlace), (1, 1))
        with self.assertRaisesRegex(ValueError, 'row filter'):
            SMOKE['png'](make_png(1, 1, 8, 6, b'\5' + b'\0' * 4), (1, 1))
        for compressed in (zlib.compress(b'\0' * 4), zlib.compress(b'\0' * 6),
                           zlib.compress(b'\0' * 5)[:-1], zlib.compress(b'\0' * 5) + b'extra',
                           zlib.compress(b'\0' * 5) + zlib.compress(b'\0' * 5)):
            with self.subTest(compressed=compressed):
                with self.assertRaisesRegex(ValueError, 'pixel stream'):
                    SMOKE['png'](make_png(1, 1, 8, 6, b'', compressed=compressed), (1, 1))
        valid = make_png(1, 1, 8, 6, b'\0' * 5)
        for malformed in (valid[:-12], valid + b'extra',
                          valid[:-12] + png_chunk(b'IEND', b'nonempty')):
            with self.subTest(malformed=malformed):
                with self.assertRaisesRegex(ValueError, 'end'):
                    SMOKE['png'](malformed, (1, 1))

    def test_an_empty_reference_is_not_success(self):
        with self.assertRaisesRegex(ValueError, 'screen background'):
            SMOKE['check_pixels'](bytearray(320 * 180 * 4))

    def test_existing_zip_with_wrong_target_is_not_success(self):
        with tempfile.TemporaryDirectory() as work:
            path = Path(work) / 'wrong.zip'
            with zipfile.ZipFile(path, 'w') as bundle:
                for name in ['design/nodes.json', 'design/variables.json', 'design/fonts.json', 'assets.json',
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
