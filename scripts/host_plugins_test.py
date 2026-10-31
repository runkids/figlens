"""Host plugin manifests stay in lockstep with the npm package and point at real files."""
import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HOST_FILES = ['.claude-plugin/plugin.json', '.claude-plugin/marketplace.json', '.codex-plugin/plugin.json',
              '.codex-plugin/mcp.json', '.agents/plugins/marketplace.json', '.cursor-plugin/plugin.json',
              '.kimi-plugin/plugin.json', '.antigravity-plugin/plugin.json', 'mcp_config.json']


def load(path):
    return json.loads((ROOT / path).read_text())


def resolve(data, jsonpath):
    # The release config only uses `$.key.key[0]` paths.
    for key, index in re.findall(r'\.([^.\[]+)(?:\[(\d+)\])?', jsonpath):
        data = data[key]
        if index:
            data = data[int(index)]
    return data


def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for item in value.values():
            yield from strings(item)
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)


class HostPluginsTest(unittest.TestCase):
    version = load('npm/package.json')['version']
    extra = load('.github/release-please-config.json')['packages']['.']['extra-files']

    def test_release_bumps_every_version_and_pinned_package(self):
        bumped = {(e['path'], e['jsonpath']) for e in self.extra}
        for path, jsonpath in bumped:
            value = resolve(load(path), jsonpath)
            self.assertIn(value, (self.version, f'@runkids/figlens@{self.version}'), f'{path} {jsonpath}')
        for path in HOST_FILES:
            data = load(path)
            pinned = [s for s in strings(data) if s.startswith('@runkids/figlens')]
            versions = ['version' in data, *(('plugins' in data and 'version' in p) for p in data.get('plugins', []))]
            paths = {jp for p, jp in bumped if p == path}
            self.assertEqual(len(paths), len(pinned) + sum(versions), f'{path}: an unbumped version or package')

    def test_manifests_point_at_existing_paths(self):
        for path in HOST_FILES:
            data = load(path)
            for key in ('skills', 'mcpServers'):
                if isinstance(data.get(key), str):
                    self.assertTrue((ROOT / data[key]).exists(), f'{path} {key}')
            for plugin in data.get('plugins', []):
                source = plugin['source'] if isinstance(plugin['source'], str) else plugin['source']['url']
                self.assertTrue((ROOT / source / 'skills/figlens/SKILL.md').is_file(), path)


if __name__ == '__main__':
    unittest.main()
