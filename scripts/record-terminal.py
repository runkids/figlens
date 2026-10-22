#!/usr/bin/env python3
"""Record real terminal output as asciicast v2; forward stdin without storing it.

Usage: record-terminal.py OUTPUT.cast CWD COLS ROWS COMMAND [ARGS...]
Use a clean demo profile: terminal output can contain private data.
"""
import codecs
import fcntl
import json
import os
import pty
import select
import signal
import struct
import sys
import termios
import time
import tty

output, cwd, cols, rows, *command = sys.argv[1:]
assert command and int(cols) > 0 and int(rows) > 0
pid, terminal = pty.fork()
if pid == 0:
    os.chdir(cwd)
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack('HHHH', int(rows), int(cols), 0, 0))
    os.execvp(command[0], command)

decoder = codecs.getincrementaldecoder('utf-8')('replace')
started = time.monotonic()
saved = termios.tcgetattr(0) if os.isatty(0) else None
try:
    if saved:
        tty.setraw(0)
    with open(output, 'x', encoding='utf-8') as recording:
        recording.write(json.dumps({'version': 2, 'width': int(cols), 'height': int(rows), 'timestamp': int(time.time()), 'title': 'figlens component review in Pi'}) + '\n')
        recording.flush()
        while True:
            readable, _, _ = select.select([terminal, 0], [], [])
            if terminal in readable:
                try:
                    data = os.read(terminal, 65536)
                except OSError:
                    break
                if not data:
                    break
                os.write(1, data)
                recording.write(json.dumps([round(time.monotonic() - started, 4), 'o', decoder.decode(data)]) + '\n')
                recording.flush()
            if 0 in readable:
                data = os.read(0, 65536)
                if not data:
                    break
                os.write(terminal, data)
finally:
    if saved:
        termios.tcsetattr(0, termios.TCSADRAIN, saved)
    os.close(terminal)
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    os.waitpid(pid, 0)
