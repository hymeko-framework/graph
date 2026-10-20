import copy
import unittest

from host import HostState, QuietHostGuard


class HostContracts(unittest.TestCase):
    def test_linux_cpu_counters_keep_idle_and_guest_accounting_distinct(self) -> None:
        counters = HostState.parse_cpu_stat("cpu 10 2 3 40 5 6 7 8 99 77\ncpu2 1 0 1 8 0 0 0 0\n")
        self.assertEqual(counters["cpu"], [45, 69, 12])
        self.assertEqual(counters["cpu2"], [8, 9, 1])
        self.assertAlmostEqual(HostState.active_fraction([0, 0, 0], [50, 75, 25]), 0.5)
        for text in ["intr 3", "cpu 1 2", "cpu 0 0 0 -1 0 0 0 0"]:
            with self.assertRaises(ValueError):
                HostState.parse_cpu_stat(text)
        with self.assertRaises(ValueError):
            HostState.active_fraction([0, 0, 0], [0, 0, 0])

    def test_quiet_gate_rejects_other_users_activity_and_insufficient_memory(self) -> None:
        root_session = "root tty7 2026-08-30 13:17 (:0)"
        guard = QuietHostGuard({"cpus": [2, 66], "allowed_sessions": [root_session]},
                               uid=1001, user="researcher")
        quiet = {"sessions": [root_session, "researcher pts/0 now (vpn)"],
                 "human_uids": [1001], "available_bytes": 64 * 1024**3,
                 "active": {"cpu": 0.001, "cpu2": 0.01, "cpu66": 0.0}}
        self.assertEqual(guard.reasons(quiet), [])
        for field, value in [
            ("human_uids", [1001, 1002]),
            ("sessions", [root_session, "someone pts/1 now (vpn)"]),
            ("available_bytes", 8 * 1024**3),
            ("active", {"cpu": 0.02, "cpu2": 0.0, "cpu66": 0.0}),
            ("active", {"cpu": 0.0, "cpu2": 0.0, "cpu66": 0.1}),
        ]:
            changed = copy.deepcopy(quiet)
            changed[field] = value
            self.assertTrue(guard.reasons(changed), field)


if __name__ == "__main__":
    unittest.main()
