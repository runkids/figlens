#!/usr/bin/env python3
"""Drive real OpenCode dialogs over a PTY, only on the isolated synthetic fixture."""
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

binary, url, session_id, output = sys.argv[1:]
output = Path(output)
pid, fd = pty.fork()
if pid == 0:
    os.chdir(output / 'project')
    os.execv(binary, [binary, '--server', url, '--session', session_id])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 50, 140, 0, 0))
raw = bytearray()
steps = []


def pump():
    if select.select([fd], [], [], 0.1)[0]:
        try:
            chunk = os.read(fd, 65536)
        except OSError as error:
            if error.errno == errno.EIO:
                raise RuntimeError('TUI exited before completing dialogs') from error
            raise
        if not chunk:
            raise RuntimeError('TUI closed its terminal')
        raw.extend(chunk)
        if b'\x1b[6n' in chunk:
            os.write(fd, b'\x1b[1;1R')


def wait_text(text, start):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        pump()
        visible = re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', raw[start:].decode('utf-8', errors='replace'))
        if text in visible:
            steps.append(text)
            return
    raise AssertionError(f'Native dialog did not show {text!r}')


def send(keys):
    start = len(raw)
    os.write(fd, keys)
    return start


try:
    # Wait for the session route to finish rendering before dispatching its command.
    wait_text('Mapping native UI', 0)
    wait_text('First button', send(b'/figlens review\r'))
    wait_text('Review component mapping', send(b'\r'))
    wait_text('path', send(b'\x1b[B\r'))
    wait_text('export', send(b'\r'))
    wait_text('import', send(b'\r'))
    wait_text('example', send(b'\r'))
    wait_text('Approval records your mapping decision', send(b'\x01\x0bButton({ label: "native UI" })\r'))
    wait_text('Second button', send(b'\r'))
    wait_text('Review component mapping', send(b'\r'))
    send(b'\x1b[B\x1b[B\r')
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        pump()
        guide = output / 'project/.figlens/component-mappings.md'
        if guide.exists() and '"status": "pending"' not in guide.read_text():
            break
    else:
        raise AssertionError('Native review did not finish')
    print(json.dumps({'nativeDialogs': steps, 'completed': True}))
finally:
    (output / 'tui.raw').write_bytes(raw)
    (output / 'tui-steps.json').write_text(json.dumps(steps, indent=2) + '\n')
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
