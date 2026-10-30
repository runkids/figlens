#!/usr/bin/env python3
"""Check an explicitly selected figlens binary against the public synthetic example.

Python 3.8+ standard library only. No model, network, extraction or cache clearing.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
import zipfile
import zlib


def require(ok, message):
    if not ok:
        raise ValueError(message)


def invoke(binary, cwd, *args):
    result = subprocess.run([str(binary), *map(str, args)], cwd=cwd,
                            capture_output=True, text=True, encoding='utf-8', timeout=60)
    require(result.returncode == 0,
            f'{args[0]} exited {result.returncode}: {result.stderr.strip()[:2000]}')
    return result.stdout


def png(data, expected_size):
    """Normalize non-interlaced gray/indexed 1/2/4/8-bit and RGB/GA/RGBA8 PNGs to RGBA."""
    require(data[:8] == b'\x89PNG\r\n\x1a\n', 'not a PNG')
    offset, compressed, header, ended = 8, bytearray(), None, False
    palette, transparency, seen_data = None, None, False
    while offset < len(data):
        require(offset + 12 <= len(data), 'truncated PNG chunk')
        length = struct.unpack_from('>I', data, offset)[0]
        kind = data[offset + 4:offset + 8]
        end = offset + 8 + length
        require(end + 4 <= len(data), 'truncated PNG data')
        payload = data[offset + 8:end]
        require(zlib.crc32(kind + payload) & 0xffffffff == struct.unpack_from('>I', data, end)[0],
                'PNG checksum mismatch')
        if kind == b'IHDR':
            require(offset == 8 and header is None and len(payload) == 13, 'invalid PNG header')
            header = struct.unpack('>IIBBBBB', payload)
        elif kind == b'PLTE':
            require(header is not None and palette is None and transparency is None and not seen_data,
                    'invalid PNG palette order')
            palette = payload
        elif kind == b'tRNS':
            require(header is not None and transparency is None and not seen_data,
                    'invalid PNG transparency order')
            require(header[3] != 3 or palette is not None, 'PNG transparency precedes palette')
            transparency = payload
        elif kind == b'IDAT':
            require(header is not None, 'PNG pixel data precedes header')
            seen_data = True
            compressed.extend(payload)
        elif kind == b'IEND':
            require(not payload and end + 4 == len(data), 'invalid PNG end')
            ended = True
            break
        offset = end + 4
    require(header is not None and ended, 'PNG header or end missing')
    w, h, depth, color, compression, filtering, interlace = header
    require((w, h) == expected_size, f'PNG size {(w, h)} != {expected_size}')
    require(w > 0 and h > 0, 'invalid PNG dimensions')
    require((color in (0, 3) and depth in (1, 2, 4, 8)
             or color in (2, 4, 6) and depth == 8)
            and (compression, filtering, interlace) == (0, 0, 0),
            'smoke decoder supports non-interlaced gray/indexed 1/2/4/8-bit and RGB/GA/RGBA8 PNGs only')
    if palette is not None:
        require(color in (2, 3, 6) and 0 < len(palette) <= 768 and len(palette) % 3 == 0,
                'invalid PNG palette')
    if color == 3:
        require(palette is not None and len(palette) // 3 <= 1 << depth, 'invalid PNG palette')
    if transparency is not None:
        if color == 3:
            require(0 < len(transparency) <= len(palette) // 3, 'invalid PNG transparency')
        elif color in (0, 2):
            require(len(transparency) == (2 if color == 0 else 6), 'invalid PNG transparency')
            transparency = struct.unpack('>H' if color == 0 else '>HHH', transparency)
            require(all(value < 1 << depth for value in transparency), 'invalid PNG transparency')
        else:
            require(False, 'invalid PNG transparency')
    channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[color]
    stride = (w * channels * depth + 7) // 8
    bpp = max(1, (channels * depth + 7) // 8)
    decoder = zlib.decompressobj()
    raw = decoder.decompress(compressed, (stride + 1) * h + 1)
    require(decoder.eof and not decoder.unused_data and not decoder.unconsumed_tail
            and len(raw) == (stride + 1) * h, 'invalid PNG pixel stream')
    pixels, previous = bytearray(), bytearray(stride)
    for y in range(h):
        start = y * (stride + 1)
        mode, row = raw[start], bytearray(raw[start + 1:start + 1 + stride])
        require(mode <= 4, 'invalid PNG row filter')
        for x in range(stride):
            a = row[x - bpp] if x >= bpp else 0
            b, c = previous[x], previous[x - bpp] if x >= bpp else 0
            if mode == 4:
                p = a + b - c
                distances = [abs(p - a), abs(p - b), abs(p - c)]
                prediction = [a, b, c][distances.index(min(distances))]
            else:
                prediction = [0, a, b, (a + b) // 2][mode]
            row[x] = (row[x] + prediction) & 255
        previous = row
        if color == 6:
            pixels.extend(row)
            continue
        for x in range(w):
            if color in (0, 3):
                sample = (row[x * depth // 8] >> (8 - depth - x * depth % 8)) & ((1 << depth) - 1)
                if color == 3:
                    require(sample < len(palette) // 3, 'PNG palette index out of range')
                    pixels.extend(palette[sample * 3:sample * 3 + 3])
                    pixels.append(transparency[sample] if transparency is not None
                                  and sample < len(transparency) else 255)
                else:
                    gray = sample * 255 // ((1 << depth) - 1)
                    pixels.extend((gray, gray, gray, 0 if transparency == (sample,) else 255))
            elif color == 2:
                rgb = row[x * 3:x * 3 + 3]
                pixels.extend(rgb)
                pixels.append(0 if transparency == tuple(rgb) else 255)
            else:
                gray, alpha = row[x * 2:x * 2 + 2]
                pixels.extend((gray, gray, gray, alpha))
    return pixels


def check_pixels(pixels):
    # Interior points, not glyph edges: permit 1 channel level of float rounding.
    for x, y, rgba, name in [
        (0, 0, [240, 244, 248, 255], 'screen background'),
        (24, 80, [255, 255, 255, 255], 'white card'),
        (44, 33, [27, 153, 139, 255], 'original image tile'),
        (82, 34, [27, 39, 60, 255], 'READY outline'),
        (81, 77, [27, 39, 60, 255], 'LOCAL outline'),
        (260, 132, [80, 100, 220, 255], 'triangle'),
        (249, 118, [255, 255, 255, 255], 'outside triangle'),
    ]:
        actual = pixels[(y * 320 + x) * 4:(y * 320 + x + 1) * 4]
        require(len(actual) == 4 and all(abs(a - b) <= 1 for a, b in zip(actual, rgba)),
                f'reference content mismatch at {name}: {list(actual)} != {rgba}')


def validate_bundle(path):
    with zipfile.ZipFile(path) as bundle:
        require(bundle.testzip() is None, 'bundle ZIP checksum failure')
        required = {'manifest.json', 'design/nodes.json', 'design/variables.json', 'design/fonts.json',
                    'assets.json', 'missing-assets.json', 'references/screen.png', 'context.md'}
        require(required <= set(bundle.namelist()), f'bundle missing {sorted(required - set(bundle.namelist()))}')
        read = lambda name: json.loads(bundle.read(name))
        manifest, doc, assets = read('manifest.json'), read('design/nodes.json'), read('assets.json')
        require(manifest['format'] == 'figlens-bundle' and manifest['version'] == 1, 'unexpected bundle format')
        require(manifest['source']['nodeId'] == manifest['root']['id'] == '0:1'
                and manifest['root']['name'] == 'First success', 'manifest target is not First success [0:1]')
        root = doc['document']
        require((root['id'], root['name'], root['type']) == ('0:1', 'First success', 'FRAME'), 'wrong bundled frame')
        require(root['absoluteBoundingBox'] == {'x': 0, 'y': 0, 'width': 320, 'height': 180}, 'wrong frame bounds')
        def descendants(node):
            yield node
            for child in node.get('children', []):
                yield from descendants(child)
        nodes = {node['id']: node for node in descendants(root)}
        require(nodes['0:4']['characters'] == 'READY' and nodes['0:5']['characters'] == 'LOCAL', 'expected text missing')
        require(nodes['0:3']['fills'][0]['imageRef'] == 'first-success-tile', 'wrong image fill')
        require(nodes['0:6']['type'] == 'VECTOR', 'triangle is not a vector')
        require(read('missing-assets.json') == [], 'bundle reports missing assets')
        require(isinstance(read('design/variables.json'), dict), 'variables file is not an object')
        require([(f['family'], f['nodeIds']) for f in read('design/fonts.json')['faces']]
                == [('Figlens Blocks', ['0:4', '0:5'])], 'font inventory differs')
        require(b'First success' in bundle.read('context.md'), 'context is for a different target')
        mapping = {asset['key']: asset for asset in assets}
        require(set(mapping) == {'first-success-tile', 'node:0:6'}, 'expected image and vector assets missing')
        require(mapping['first-success-tile']['nodeId'] == '0:3' and mapping['node:0:6']['nodeId'] == '0:6', 'asset targets differ')
        tile = png(bundle.read(mapping['first-success-tile']['path']), (4, 4))
        for i in range(16):
            expected = [255] * 4 if i // 4 == i % 4 or i // 4 + i % 4 == 3 else [27, 153, 139, 255]
            require(list(tile[i * 4:i * 4 + 4]) == expected, 'original tile pixels differ')
        vector = ET.fromstring(bundle.read(mapping['node:0:6']['path']))
        require(vector.tag == '{http://www.w3.org/2000/svg}svg' and vector.find('.//{http://www.w3.org/2000/svg}path') is not None,
                'triangle SVG has no path')
        listed = {entry['path']: entry for entry in manifest['files']}
        require(required - {'manifest.json'} <= set(listed), 'manifest is missing required file records')
        require(listed['references/screen.png']['nodeId'] == '0:1', 'reference metadata targets another node')
        for entry in manifest['files']:
            data = bundle.read(entry['path'])
            require(len(data) == entry['size'] and hashlib.sha256(data).hexdigest() == entry['sha256'],
                    f'manifest integrity mismatch: {entry["path"]}')
        check_pixels(png(bundle.read('references/screen.png'), (320, 180)))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, help='explicit path to the native figlens executable; never searched on PATH')
    parser.add_argument('--example', type=Path, default=Path(__file__).resolve().parent.parent / 'examples/first-success/design.fig')
    args = parser.parse_args(argv)
    stage = 'A'
    try:
        binary = Path(args.binary).expanduser().resolve()
        require(binary.is_file(), f'binary not found: {binary}')
        with tempfile.TemporaryDirectory(prefix='figlens-first-success-') as work:
            version = invoke(binary, work, '--version').strip()
            require(version.startswith('figlens ') and len(version.split()) == 2, f'unexpected version output: {version!r}')
            print(f'PASS A: {binary} -> {version}')
            stage = 'B'
            require(args.example.is_file(), f'example not found: {args.example}; use the checkout copy or --example')
            source = Path(work) / 'design.fig'
            shutil.copyfile(args.example, source)
            found = json.loads(invoke(binary, work, 'find', source, 'First success', '--type', 'FRAME'))
            frames = [n for n in found['results'] if n['name'] == 'First success']
            require(len(frames) == 1 and frames[0]['id'] == '0:1' and frames[0]['size'] == {'width': 320, 'height': 180},
                    'expected unique First success [0:1], 320x180 frame')
            require(found['truncated'] is False, 'frame search was truncated')
            print('PASS B: First success [0:1], 320x180')
            stage = 'C'
            output = Path(work) / 'frame.zip'
            result = json.loads(invoke(binary, work, 'bundle', source, '0:1', output))
            require(result['nodeId'] == '0:1' and result['warnings'] == [], 'bundle target/warnings differ')
            validate_bundle(output)
            print('PASS C: bundle structure, READY / LOCAL, original image, vector and 320x180 reference verified')
        print('PASS: isolated temporary outputs removed; no cache was cleared')
        return 0
    except (OSError, ValueError, KeyError, TypeError, IndexError, AttributeError, struct.error, zlib.error,
            zipfile.BadZipFile, ET.ParseError, subprocess.SubprocessError) as error:
        print(f'FAIL {stage}: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
