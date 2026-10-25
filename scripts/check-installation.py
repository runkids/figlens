#!/usr/bin/env python3
"""Layer B: install a real candidate tarball and exercise its wrapper, native CLI and MCP.

The test-only fetch preload supplies the candidate archive to the unchanged wrapper.
This is not layer C (a published release download).
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tarfile
import tempfile
from installation import environment, run, toolchain

ROOT = Path(__file__).resolve().parent.parent
check_mcp = runpy.run_path(str(ROOT/'scripts/mcp-smoke.py'))['check_mcp']


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', required=True, type=Path)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--pi-version', choices=['1.3.0', '1.4.0'])
    args = parser.parse_args()
    artifacts = args.artifacts.resolve()
    metadata = json.loads((artifacts/'candidate.json').read_text())
    assert metadata['commit'] == args.commit, 'Candidate is from another commit'
    package = artifacts/metadata['package']['file']
    assert sha(package) == metadata['package']['sha256'], 'Candidate tarball hash differs'
    required = {'package.json', 'README.md', 'LICENSE', 'bin/figlens.js', 'target.js',
                'pi/index.ts', 'pi/workflow.mjs', 'pi/viewer.mjs', 'pi/package.json', 'pi/README.md',
                'skills/figlens/SKILL.md', 'skills/figlens/references/planning.md',
                'skills/figlens/references/planning-details.md', 'skills/figlens/references/yolo.md'}
    with tarfile.open(package) as archive:
        files = {p.name.removeprefix('package/') for p in archive.getmembers() if p.isfile()}
        assert files == required, f'Unexpected npm package files: missing={required-files}, extra={files-required}'
    node, npm = toolchain()
    with tempfile.TemporaryDirectory(prefix='figlens-consumer-') as temp:
        work = Path(temp)
        env = environment(work, node)
        run([node, '-e', "if (typeof fetch !== 'function') throw new Error('Native fetch is unavailable on this Node runtime')"], work, env)
        consumer = work/'consumer'
        consumer.mkdir()
        (consumer/'package.json').write_text('{"private":true}')
        install = npm + ['install', '--ignore-scripts', '--no-audit', '--no-fund', '--package-lock=false']
        run(install + ['--offline', '--omit=peer', package], consumer, env)
        installed = consumer/'node_modules/@runkids/figlens'
        assert installed.is_dir() and not installed.is_symlink(), 'Not an extracted tarball installation'
        manifest = json.loads((installed/'package.json').read_text())
        assert manifest['version'] == metadata['version']
        assert manifest['bin']['figlens'] == 'bin/figlens.js'
        assert manifest['pi']['extensions'] == ['./pi/index.ts'] and manifest['pi']['skills'] == ['./skills']
        asset = json.loads(run([node, '-e', 'console.log(JSON.stringify(require(process.argv[1]).asset(process.platform,process.arch,process.argv[2])))',
                                installed/'target.js', metadata['version']], consumer, env))
        archive = asset['archive']
        expected = metadata['archives'][archive]
        assert sha(artifacts/archive) == expected['sha256'], 'Candidate native archive hash differs'
        # Only these two exact release URLs can be resolved; no old GitHub release is reachable.
        feed = work/'feed.json'
        events = work/'fetch.jsonl'
        feed.write_text(json.dumps({'version': metadata['version'], 'archive': archive,
                                    'directory': str(artifacts), 'events': str(events)}))
        preload = str(ROOT/'scripts/candidate-fetch.cjs').replace('\\', '/')
        wrapped_env = dict(env, FIGLENS_TEST_FEED=str(feed), NODE_OPTIONS=f'--require="{preload}"')
        wrapper = [node, installed/'bin/figlens.js']
        cache = work/'cache/figlens/npm'/metadata['version']/asset['binary']
        assert not cache.exists(), 'Cache was not cold'
        expected_version = f'figlens {metadata["version"]}'
        assert run(wrapper + ['--version'], consumer, wrapped_env).strip() == expected_version
        assert sha(cache) == expected['binarySha256'], 'Wrapper ran a different native binary'
        downloads = events.read_text().splitlines()
        assert sorted(json.loads(line)['name'] for line in downloads) == sorted([archive, 'SHA256SUMS'])
        assert run(wrapper + ['--version'], consumer, wrapped_env).strip() == expected_version
        assert events.read_text().splitlines() == downloads, 'Warm cache downloaded again'
        # A second cold cache exercises real simultaneous wrapper processes and real extraction.
        concurrent = work/'parallel-cache'
        parallel_env = dict(wrapped_env, XDG_CACHE_HOME=str(concurrent), LOCALAPPDATA=str(concurrent))
        with ThreadPoolExecutor(max_workers=2) as pool:
            versions = list(pool.map(lambda _: run(wrapper + ['--version'], consumer, parallel_env), range(2)))
        assert all(v.strip() == expected_version for v in versions)
        assert sha(concurrent/'figlens/npm'/metadata['version']/asset['binary']) == expected['binarySha256']
        assert not list((work/'cache/figlens/npm').glob('.tmp-*'))
        assert not list((concurrent/'figlens/npm').glob('.tmp-*'))
        fixture = consumer/'design.fig'
        shutil.copy2(ROOT/'examples/first-success/design.fig', fixture)
        print(run([sys.executable, ROOT/'scripts/first-success-smoke.py', '--binary', cache, '--example', fixture], consumer, env), end='')
        native_mcp = check_mcp([cache, 'mcp'], consumer, env, fixture, metadata['version'])
        gcs = runpy.run_path(str(ROOT/'scripts/check-gcs-snapshot.py'))['check_gcs'](cache, fixture)
        # Cold MCP startup must keep extraction/installation output off its protocol stdout.
        mcp_cache = work/'mcp-cache'
        mcp_env = dict(wrapped_env, XDG_CACHE_HOME=str(mcp_cache), LOCALAPPDATA=str(mcp_cache))
        wrapped_mcp = check_mcp(wrapper + ['mcp'], consumer, mcp_env, fixture, metadata['version'])
        assert sha(mcp_cache/'figlens/npm'/metadata['version']/asset['binary']) == expected['binarySha256']
        pi = {'status': 'not requested', 'reason': 'CLI-only job; Pi requires Node >=22.19.0'}
        if args.pi_version:
            run(install + [f'@earendil-works/pi-coding-agent@{args.pi_version}'], consumer, env, timeout=240)
            sdk = consumer/'node_modules/@earendil-works/pi-coding-agent'
            pi = json.loads(run([node, ROOT/'scripts/check-pi-integration.mjs', sdk, installed, fixture, consumer], consumer, wrapped_env))
            assert pi['pi'] == args.pi_version and pi['packageVersion'] == metadata['version'], pi
            config = pi['mcp']
            pi['stdio'] = check_mcp([config['command'], *config['args']], consumer, wrapped_env, fixture, metadata['version'])
        print(json.dumps({'layer': 'B: candidate artifacts, injected local transport; not a published release',
                          'commit': metadata['commit'], 'dirty': metadata['dirty'], 'sourceDigest': metadata['sourceDigest'],
                          'package': manifest['version'], 'tarballSha256': sha(package), 'binarySha256': sha(cache),
                          'node': run([node, '--version'], consumer, env).strip(),
                          'npm': run(npm + ['--version'], consumer, env).strip(),
                          'coldInstall': True, 'warmCacheNoDownload': True, 'parallelFirstUse': True,
                          'nativeMcp': native_mcp, 'wrapperMcp': wrapped_mcp, 'gcsMock': gcs, 'pi': pi}, indent=2))


if __name__ == '__main__':
    try:
        main()
    except (AssertionError, OSError, ValueError, KeyError, RuntimeError, subprocess.SubprocessError) as error:
        raise SystemExit(f'Installation smoke failed: {error}')
