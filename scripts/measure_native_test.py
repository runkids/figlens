import importlib
import os
import shutil
import subprocess
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

native = importlib.import_module('measure-native')


class MeasureNativeTest(unittest.TestCase):
    def test_process_table_reads_a_running_child(self):
        # Windows has no sleep executable; its Python is a real one.
        exe, command = (sys.executable, [sys.executable, '-c', 'import time; time.sleep(5)']) if os.name == 'nt' \
            else (shutil.which('sleep'), [shutil.which('sleep'), '5'])
        child = subprocess.Popen(command)
        try:
            for _ in range(50):
                row = native.table(os.path.realpath(exe)).get(child.pid)
                if row:
                    break
                native.time.sleep(0.05)
            self.assertEqual((row['ppid'], row['rss'] > 0, row['cpuMs'] >= 0, row['writeBytes'] >= 0),
                             (os.getpid(), True, True, True))
        finally:
            child.kill()
            child.wait()

    def test_runner_and_worker_are_told_from_launcher_and_cli_children(self):
        me, gone = os.getpid(), 4_000_000_000  # a parent no longer listed; in a container this test may be PID 1
        rows = {10: {'ppid': me}, 20: {'ppid': 10}, 21: {'ppid': 20}, 22: {'ppid': 21},  # holder, launcher, runner, worker
                11: {'ppid': me}, 31: {'ppid': gone}, 32: {'ppid': 31},  # holder; a runner whose launcher exited
                13: {'ppid': me}, 14: {'ppid': 13}}  # CLI call and its build worker
        self.assertEqual(native.roles(rows, {10, 11}), ({21, 31}, {22, 32}))

    def test_idle_cost_separates_verification_seconds_from_polling(self):
        second = {'cpuMs': 2, 'readBytes': 10, 'writeBytes': 5, 'physReadBytes': None, 'physWriteBytes': 0, 'ended': False}
        series = [second] * 130
        series[30] = series[90] = {**second, 'cpuMs': 200, 'readBytes': 5_000_000, 'ended': True}
        cost = native.idle_cost(series, cpu_exact=True)
        self.assertEqual((cost['excludedFromPolling'], cost['pollingCpuMsPer60s'], cost['cpuMsPer60s'],
                          cost['readBytesPer60s'], cost['ioExact']), (3, 120, 318, 5_000_590, False))

    @unittest.skipUnless(os.name == 'nt', 'Windows job accounting')
    def test_a_job_keeps_an_ended_childs_cpu(self):
        job = native.Job()
        try:
            # The child starts and waits for a grandchild: both ended, both still accounted.
            grandchild = 'import subprocess, sys; subprocess.run([sys.executable, "-c", "sum(range(20_000_000))"], check=True)'
            child = job.start([sys.executable, '-c', grandchild])
            child.wait()
            cpu, _, _, processes, active = job.counters()
            self.assertEqual((cpu > 0, processes, active), (True, 2, 0))
        finally:
            job.close()

    def test_a_failed_second_holder_still_releases_the_first(self):
        class Workspace:
            held, released = [], []
            def hold(self, _job):
                if self.held:
                    raise OSError('second holder failed')
                self.held.append('first')
                return mock.Mock(pid=1)
            def release(self, *holders):
                self.released.extend(holders)
        ws = Workspace()
        with mock.patch.object(native, 'Job', mock.Mock(), create=True), self.assertRaises(OSError):
            native.idle(ws, 'unused', 2, 1)
        self.assertEqual(len(ws.released), 1)

    def test_only_the_saves_own_complete_revision_counts(self):
        def revision(rid, snapshot, stop=None):
            coverage = {'complete': True, 'new': {'complete': True, 'stop': stop, 'scannedRecords': 9}}
            return {'revisionId': rid, 'snapshotId': snapshot, 'summary': {'changes': 1}, 'coverage': coverage}
        rows = [revision('old', 'sha256:a'), revision('new', 'sha256:b', stop='work')]
        self.assertEqual([native.landed(rows[:1], {'old'}, 'sha256:b', 9), native.landed(rows, {'old'}, 'sha256:b', 9)],
                         [None, {'revisionId': 'new', 'snapshotMatches': True, 'changes': 1, 'complete': False}])

    def test_windows_cpu_ticks_still_count_as_polling(self):
        second = {'readBytes': 0, 'writeBytes': 0, 'physReadBytes': None, 'physWriteBytes': None, 'ended': False}
        series = [{**second, 'cpuMs': 16 if i % 2 else 0} for i in range(130)]
        series[30] = series[90] = {**second, 'cpuMs': 250}
        self.assertEqual(native.idle_cost(series, cpu_exact=True)['pollingCpuMsPer60s'], 484)

    def test_a_reaped_worker_counts_once(self):
        row = lambda ppid, cpu, child: {'ppid': ppid, 'cpuMs': cpu, 'childCpuMs': child, 'readBytes': 0,
                                        'writeBytes': 0, 'physReadBytes': 0, 'physWriteBytes': 0}
        # The runner (1) ran 2 ms; its worker (2), seen at 30 ms, ended at 50 ms and was reaped.
        cost = native.step({1: row(0, 10, 0), 2: row(1, 30, 0)}, {1: row(0, 12, 50)})
        self.assertEqual((cost['cpuMs'], cost['ended']), (22, True))

    def test_a_vanished_state_file_is_skipped(self):
        with tempfile.TemporaryDirectory() as temp:
            kept = Path(temp, 'index.sqlite')
            kept.write_bytes(b'x' * 3)
            self.assertEqual(native.sizes([kept, Path(temp, 'index.sqlite-wal')]), [('index.sqlite', 3)])

    def test_a_failed_sampler_fails_its_block(self):
        def broken(_):
            raise OSError('table failed')
        with mock.patch.object(native, 'table', broken), self.assertRaises(OSError):
            with native.Tree('unused'):
                native.time.sleep(0.05)

    def test_only_a_cancel_proven_in_flight_counts(self):
        self.assertEqual([native.classify(True, 'j', 'cancelled', False), native.classify(True, 'j', 'succeeded', False),
                          native.classify(False, 'j', 'cancelled', False), native.classify(True, 'j', 'cancelled', True)],
                         ['cancelled_in_flight', 'completed_before_cancel', 'not_proven_in_flight', 'timed_out'])

    def test_a_gate_needs_its_sample_count(self):
        self.assertEqual([native.verdict('cancel', [100] * 19)['status'], native.verdict('cancel', [100] * 20)['status'],
                          native.verdict('cancel', [100] * 18 + [600] * 2)['status']], ['insufficient', 'pass', 'fail'])


if __name__ == '__main__':
    unittest.main()
