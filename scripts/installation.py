"""Shared subprocess setup for candidate packaging and isolated installation checks."""
import os
from pathlib import Path
import signal
import subprocess


def toolchain():
    node = Path(subprocess.check_output(['node', '-p', 'process.execPath'], text=True).strip())
    choices = [node.parent/'node_modules/npm/bin/npm-cli.js',
               node.parent.parent/'lib/node_modules/npm/bin/npm-cli.js']
    npm = next((p for p in choices if p.is_file()), None)
    if npm is None:
        raise RuntimeError(f'Cannot locate npm next to selected Node: {node}')
    return str(node), [str(node), str(npm)]


def environment(work, node):
    keep = {'path', 'systemroot', 'windir', 'comspec', 'pathext', 'lang', 'lc_all'}
    env = {k.upper(): v for k, v in os.environ.items() if k.lower() in keep}
    for name in ['home', 'tmp', 'cache', 'agent', 'npm-cache']:
        (work/name).mkdir(exist_ok=True)
    for name in ['user.npmrc', 'global.npmrc']:
        (work/name).touch()
    env.update(HOME=str(work/'home'), USERPROFILE=str(work/'home'),
               APPDATA=str(work/'home'), LOCALAPPDATA=str(work/'cache'),
               TMPDIR=str(work/'tmp'), TEMP=str(work/'tmp'), TMP=str(work/'tmp'),
               XDG_CACHE_HOME=str(work/'cache'), PI_CODING_AGENT_DIR=str(work/'agent'),
               npm_config_cache=str(work/'npm-cache'), npm_config_userconfig=str(work/'user.npmrc'),
               npm_config_globalconfig=str(work/'global.npmrc'), CI='1', NO_COLOR='1',
               PATH=str(Path(node).parent) + os.pathsep + env.get('PATH', ''))
    return env


def stop_process(process):
    # The Node wrapper owns a native child; stopping the wrapper alone leaves it running.
    try:
        if os.name == 'nt':
            if process.poll() is None:
                try:
                    subprocess.run(['taskkill', '/PID', str(process.pid), '/T', '/F'],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                   check=True, timeout=10)
                except subprocess.CalledProcessError:
                    if process.poll() is None:
                        raise
        else:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
    finally:
        if process.poll() is None:
            process.kill()
        process.wait(timeout=10)


def run(command, cwd, env, timeout=60):
    with subprocess.Popen(list(map(str, command)), cwd=cwd, env=env, text=True,
                          encoding='utf-8', stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          start_new_session=os.name != 'nt') as process:
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except BaseException as error:
            try:
                stop_process(process)
            finally:
                stdout, stderr = process.communicate(timeout=10)
            if isinstance(error, subprocess.TimeoutExpired):
                # What the command said before it was stopped is the only clue to where it hung.
                error.output, error.stderr = stdout, stderr
            raise
    if process.returncode:
        raise RuntimeError(f'{command[0]} exited {process.returncode}\n{stdout[-4000:]}\n{stderr[-4000:]}')
    return stdout
