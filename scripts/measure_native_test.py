import importlib
import os
import shutil
import subprocess
import sys
import unittest

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

    def test_a_detached_runner_and_its_worker_are_told_from_cli_children(self):
        me = os.getpid()
        # 4e9 stands for a reparenting init; in a container this test may itself be PID 1.
        rows = {10: {'ppid': me}, 11: {'ppid': 4_000_000_000}, 12: {'ppid': 11}, 13: {'ppid': me}, 14: {'ppid': 13}, 15: {'ppid': 10}}
        self.assertEqual(native.roles(rows, {10}), ({11, 15}, {12}))

    def test_idle_cost_separates_verification_seconds_from_polling(self):
        series = [(2, 10, 5, None, 0)] * 130
        series[30] = series[90] = (200, 5_000_000, 5, None, 0)
        cost = native.idle_cost(series)
        self.assertEqual((cost['verificationSeconds'], cost['pollingCpuMsPer60s'], cost['cpuMsPer60s'], cost['readBytesPer60s']),
                         (2, 120, 318, 5_000_590))

    def test_only_a_cancel_proven_in_flight_counts(self):
        self.assertEqual([native.classify(True, 'j', 'cancelled', False), native.classify(True, 'j', 'succeeded', False),
                          native.classify(False, 'j', 'cancelled', False), native.classify(True, 'j', 'cancelled', True)],
                         ['cancelled_in_flight', 'completed_before_cancel', 'not_proven_in_flight', 'timed_out'])

    def test_a_gate_needs_its_sample_count(self):
        self.assertEqual([native.verdict('cancel', [100] * 19)['status'], native.verdict('cancel', [100] * 20)['status'],
                          native.verdict('cancel', [100] * 18 + [600] * 2)['status']], ['insufficient', 'pass', 'fail'])


if __name__ == '__main__':
    unittest.main()
