#!/usr/bin/env python3
"""Exercise native OMP review dialogs on an isolated synthetic fixture, without a model."""
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import sys
import termios
import time

bun, cli, plugin, binary, output_arg = sys.argv[1:]
output = Path(output_arg).resolve()
project = output / 'project 空白'
environment = {'PATH': '/usr/bin:/bin', 'HOME': str(output / 'home'), 'TERM': 'xterm-256color',
               'PI_CODING_AGENT_DIR': str(output / 'agent-tui'), 'FIGLENS_BIN': binary}
pid, fd = pty.fork()
if pid == 0:
    os.chdir(project)
    os.execve(bun, [bun, cli, '--no-extensions', '-e', plugin, '--no-skills', '--no-rules',
                   '--no-session', '--no-tools', '--no-lsp', '--no-title'], environment)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 50, 140, 0, 0))
raw = bytearray()
steps = []


def pump():
    if select.select([fd], [], [], 0.1)[0]:
        try:
            chunk = os.read(fd, 65536)
        except OSError as error:
            if error.errno == errno.EIO:
                raise RuntimeError('OMP exited before completing native dialogs') from error
            raise
        if not chunk:
            raise RuntimeError('OMP closed its terminal')
        raw.extend(chunk)
        if b'\x1b[6n' in chunk:
            os.write(fd, b'\x1b[1;1R')


def wait_text(text, start=0):
    deadline = time.monotonic() + 25
    while time.monotonic() < deadline:
        pump()
        visible = re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', raw[start:].decode('utf-8', errors='replace'))
        if text in visible:
            steps.append(text)
            return
    raise AssertionError(f'Native OMP dialog did not show {text!r}')


def send(keys):
    start = len(raw)
    os.write(fd, keys)
    return start


try:
    wait_text('no-model')
    for _ in range(5):
        pump()
    wait_text('Review this component mapping', send(b'/figlens review\r'))
    wait_text('path (leave empty to keep)', send(b'\x1b[B\r'))
    wait_text('export (leave empty to keep)', send(b'\r'))
    wait_text('import (leave empty to keep)', send(b'\r'))
    wait_text('example (leave empty to keep)', send(b'\r'))
    wait_text('Approve the mapping shown above?', send(b'Button({ label: "native OMP" })\r'))
    send(b'\r')
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        pump()
        guide = project / '.figlens/component-mappings.md'
        if guide.exists() and '"status": "approved"' in guide.read_text():
            break
    else:
        raise AssertionError('Native OMP review did not finish')
    print(json.dumps({'nativeDialogs': steps, 'completed': True}))
finally:
    (output / 'omp-tui.raw').write_bytes(raw)
    (output / 'omp-tui-steps.json').write_text(json.dumps(steps, indent=2) + '\n')
    os.kill(pid, signal.SIGTERM)
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if os.waitpid(pid, os.WNOHANG)[0]:
            break
        time.sleep(0.05)
    else:
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
    os.close(fd)
