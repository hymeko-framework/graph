import copy
import json
import unittest
from unittest.mock import patch

from host import HostState, QuietHostGuard, HostBusy


class HostContracts(unittest.TestCase):
    def test_windows_gate_uses_per_cpu_load_memory_and_interactive_sessions(self):
        raw = {"cpu": [{"Name":"_Total", "PercentProcessorTime":0},
                       {"Name":"2", "PercentProcessorTime":1}],
               "available_kib": 64 * 1024**2, "sessions":["Researcher"]}
        guard = QuietHostGuard({"cpus":[2]}, uid=0, user="researcher")
        with patch("host.os.name", "nt"), patch("host.subprocess.check_output", return_value=json.dumps(raw)):
            observation = guard.check()
            self.assertEqual(observation["active"]["cpu2"], .01)
            self.assertEqual(observation["available_bytes"], 64*1024**3)
            raw["cpu"][1]["PercentProcessorTime"] = 80
            with patch("host.subprocess.check_output", return_value=json.dumps(raw)):
                with self.assertRaises(HostBusy): guard.check()
        for bad in [dict(raw,cpu=[]),dict(raw,available_kib=-1),
                    dict(raw,cpu=[{"Name":"_Total","PercentProcessorTime":101}])]:
            with self.assertRaises(ValueError): HostState.windows_admission(bad)
        other = HostState.windows_admission(dict(raw,sessions=["another-user"]))
        self.assertIn("another login session is present",guard.reasons(other))

    def test_linux_observation_keeps_the_existing_delta_gate(self):
        guard=QuietHostGuard({"cpus":[2]},uid=1000,user="researcher")
        with patch("host.time.sleep"), patch("host.HostState.linux_cpus",side_effect=[
            {"cpu":[0,0,0],"cpu2":[0,0,0]}, {"cpu":[100,100,0],"cpu2":[100,100,0]}]), \
            patch("host.HostState.memory",return_value={"MemAvailable":64*1024**3}), \
            patch("host.HostState.sessions",return_value=["researcher pts/0"]), \
            patch("host.subprocess.check_output",return_value="0 1 systemd\n1000 2 shell\n65534 3 worker"):
            self.assertEqual(guard.reasons(guard.linux_observation()),[])

    def test_service_identity_does_not_whitelist_an_account_or_hide_busy_cpu(self):
        identity={"uid":10001,"comm":"server","cgroup":"0::/system.slice/docker-example.scope"}
        guard=QuietHostGuard({"cpus":[2],"service_processes":[identity]},uid=1000,user="owner")
        with patch("host.subprocess.check_output",return_value="1000 1 shell\n10001 2 server"), \
             patch("host.Path.read_text",return_value=identity["cgroup"]):
            observed=guard.process_accounts()
        self.assertEqual(observed['human_uids'],[1000])
        self.assertEqual(observed['service_processes'],[dict(identity,pid=2)])
        for processes,cgroup in [("10001 2 shell",identity['cgroup']),
                                  ("10001 2 server","0::/user.slice/session.scope")]:
            with patch("host.subprocess.check_output",return_value=processes), \
                 patch("host.Path.read_text",return_value=cgroup):
                self.assertEqual(guard.process_accounts()['human_uids'],[10001])
        with patch("host.subprocess.check_output",return_value="10001 2 server"), \
             patch("host.Path.read_text",side_effect=FileNotFoundError):
            self.assertEqual(guard.process_accounts()['human_uids'],[])
        observed.update(sessions=["owner pts/0"],available_bytes=64*1024**3,
                        active={"cpu":.10,"cpu2":0.0})
        self.assertIn('host CPU activity exceeds the quiet threshold',guard.reasons(observed))
        observed['sessions']=['someone pts/1']
        self.assertIn('another login session is present',guard.reasons(observed))
        with self.assertRaises(ValueError):
            QuietHostGuard({'cpus':[2],'service_processes':[{'uid':10001}]},uid=1000)

    def test_bounded_admission_wait_preserves_threshold_and_reports_attempts(self):
        quiet={'human_uids':[1000],'sessions':[], 'available_bytes':64*1024**3,
               'active':{'cpu':0.0,'cpu2':0.0}}
        busy=dict(quiet,active={'cpu':.5,'cpu2':.5})
        guard=QuietHostGuard({'cpus':[2],'max_wait_seconds':5},uid=1000)
        with patch.object(guard,'observe',side_effect=[busy,quiet]), \
             patch('host.time.monotonic',side_effect=[0,1]), patch('host.time.sleep'):
            self.assertEqual(guard.check()['admission_attempts'],2)
        with patch.object(guard,'observe',return_value=busy), \
             patch('host.time.monotonic',side_effect=[0,6]):
            with self.assertRaises(HostBusy):guard.check()
        with self.assertRaises(ValueError):QuietHostGuard({'cpus':[2],'max_wait_seconds':61},uid=1000)

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
