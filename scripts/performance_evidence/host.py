"""Host observations and the explicit quiet-host gate; no workloads run here."""
from __future__ import annotations

import ctypes
import getpass
import json
import os
import platform
import subprocess
import time
from pathlib import Path
from typing import Any, cast


class HostState:
    @staticmethod
    def parse_cpu_stat(text: str) -> dict[str, list[int]]:
        result = {}
        for line in text.splitlines():
            fields = line.split()
            if not fields or not fields[0].startswith("cpu"):
                continue
            values = [int(value) for value in fields[1:9]]
            if len(values) != 8 or min(values) < 0:
                raise ValueError("invalid Linux CPU counters")
            user, nice, system, idle, wait, irq, softirq, steal = values
            # Same tuple contract as GetSystemTimes: kernel includes idle.
            result[fields[0]] = [idle + wait, idle + wait + system + irq + softirq + steal, user + nice]
        if "cpu" not in result:
            raise ValueError("missing aggregate CPU counters")
        return result

    @classmethod
    def linux_cpus(cls) -> dict[str, list[int]]:
        return cls.parse_cpu_stat(Path("/proc/stat").read_text(encoding="ascii"))

    @classmethod
    def cpu_times(cls) -> list[int]:
        if platform.system() == "Linux":
            return cls.linux_cpus()["cpu"]
        if os.name != "nt":
            raise OSError("host CPU counters require Windows or Linux")
        values = [ctypes.c_ulonglong() for _ in range(3)]
        if not ctypes.windll.kernel32.GetSystemTimes(*(ctypes.byref(v) for v in values)):
            raise ctypes.WinError()
        return [v.value for v in values]

    @staticmethod
    def active_fraction(first: list[int], last: list[int]) -> float:
        elapsed = last[1] - first[1] + last[2] - first[2]
        idle = last[0] - first[0]
        if elapsed <= 0 or not 0 <= idle <= elapsed:
            raise ValueError("nonmonotonic or empty CPU observation")
        return 1 - idle / elapsed

    @staticmethod
    def sessions() -> list[str]:
        return subprocess.check_output(["who"], encoding="utf-8", env=os.environ | {"LC_ALL": "C"}).splitlines()

    @staticmethod
    def memory() -> dict[str, int]:
        values = {}
        for line in Path("/proc/meminfo").read_text(encoding="ascii").splitlines():
            fields = line.split()
            if len(fields) == 3 and fields[2] == "kB":
                values[fields[0].rstrip(":")] = int(fields[1]) * 1024
        return values

    @classmethod
    def snapshot(cls) -> dict[str, Any]:
        if os.name == "nt":
            command = "@{cpu=(Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors); os=(Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,TotalVisibleMemorySize); processes=@(Get-Process | Where-Object CPU -gt 0 | Select-Object Id,ProcessName,CPU,StartTime)} | ConvertTo-Json -Depth 5"
            return cast(dict[str, Any], json.loads(subprocess.check_output(
                ["powershell", "-NoProfile", "-Command", command], encoding="utf-8")))
        if platform.system() != "Linux":
            raise OSError("host snapshot requires Windows or Linux")
        affinity = getattr(os, "sched_getaffinity", None)
        priority = getattr(os, "getpriority", None)
        priority_kind = getattr(os, "PRIO_PROCESS", None)
        if affinity is None or priority is None or priority_kind is None:
            raise OSError("Linux affinity/priority observation is unavailable")
        return {
            "hostname": platform.node(), "os": platform.platform(),
            "cpu": json.loads(subprocess.check_output(["lscpu", "-J"], encoding="utf-8",
                                                     env=os.environ | {"LC_ALL": "C"})),
            "memory": cls.memory(), "sessions": cls.sessions(),
            "affinity": sorted(affinity(0)), "nice": priority(priority_kind, 0),
            "processes": subprocess.check_output(
                ["ps", "-eo", "uid=,pid=,etimes=,time=,comm="], encoding="utf-8").splitlines(),
        }

    @staticmethod
    def power_plan() -> str:
        if os.name == "nt":
            return subprocess.check_output(["powercfg", "/getactivescheme"]).decode(errors="replace")
        return "Linux CPU policy recorded separately; system settings are not changed."


class HostBusy(RuntimeError):
    def __init__(self, observation: dict[str, Any], reasons: list[str]):
        super().__init__("; ".join(reasons))
        self.observation = observation
        self.reasons = reasons


class QuietHostGuard:
    def __init__(self, config: dict[str, Any], *, uid: int | None = None, user: str | None = None):
        self.config = config
        if uid is None:
            getuid = getattr(os, "getuid", None)
            if getuid is None:
                raise OSError("quiet-host guard requires a Unix user ID")
            uid = int(getuid())
        self.uid = uid
        self.user = getpass.getuser() if user is None else user

    def reasons(self, observation: dict[str, Any]) -> list[str]:
        result = []
        if set(observation["human_uids"]) - {self.uid}:
            result.append("another user has processes on this host")
        for session in observation["sessions"]:
            fields = session.split()
            if fields and fields[0] != self.user and session not in self.config.get("allowed_sessions", []):
                result.append("another login session is present")
        if observation["available_bytes"] < self.config.get("min_available_bytes", 32 * 1024**3):
            result.append("available memory is below the declared headroom")
        if observation["active"]["cpu"] > self.config.get("max_host_fraction", 0.005):
            result.append("host CPU activity exceeds the quiet threshold")
        for cpu in self.config["cpus"]:
            if observation["active"][f"cpu{cpu}"] > self.config.get("max_core_fraction", 0.05):
                result.append(f"CPU {cpu} or its SMT sibling is busy")
        return result

    def check(self) -> dict[str, Any]:
        if platform.system() != "Linux":
            raise OSError("the explicit quiet-host guard currently requires Linux")
        first = HostState.linux_cpus()
        time.sleep(1)
        last = HostState.linux_cpus()
        selected = ["cpu", *(f"cpu{cpu}" for cpu in self.config["cpus"])]
        active = {cpu: HostState.active_fraction(first[cpu], last[cpu]) for cpu in selected}
        uids = {int(uid) for uid in subprocess.check_output(["ps", "-eo", "uid="], encoding="ascii").split()}
        observation = {
            "sessions": HostState.sessions(),
            "human_uids": sorted(uid for uid in uids if uid >= 1000 and uid != 65534),
            "available_bytes": HostState.memory()["MemAvailable"], "active": active,
        }
        reasons = self.reasons(observation)
        if reasons:
            raise HostBusy(observation, reasons)
        return observation
