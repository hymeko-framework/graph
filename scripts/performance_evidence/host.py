"""Host observations and the explicit quiet-host gate; no workloads run here."""
from __future__ import annotations

import ctypes
import getpass
import json
import math
import os
import platform
import subprocess
import time
from pathlib import Path
from typing import Any, cast


class ProcessMemory:
    """OS process counters; lifetime peaks remain distinct from phase sampling."""
    class WindowsCounters(ctypes.Structure):
        _fields_ = [('cb',ctypes.c_uint32),('faults',ctypes.c_uint32)] + [
            (name,ctypes.c_size_t) for name in ('peak','working','paged_peak','paged',
                'nonpaged_peak','nonpaged','page','page_peak','private')]

    @staticmethod
    def parent_pid(pid):
        command=f"(Get-CimInstance Win32_Process -Filter 'ProcessId={int(pid)}').ParentProcessId"
        value=subprocess.check_output(['powershell','-NoProfile','-Command',command],text=True,
                                      timeout=15,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0)).strip()
        return int(value) if value else 0

    @classmethod
    def is_descendant(cls,pid,root):
        seen=set()
        while pid>0 and pid not in seen:
            if pid==root:return True
            seen.add(pid);pid=cls.parent_pid(pid)
        return False

    @staticmethod
    def open_windows_process(pid):
        function=ctypes.windll.kernel32.OpenProcess
        function.restype=ctypes.c_void_p
        # Read counters, set affinity, and terminate only our verified child on failure.
        handle=function(0x1000|0x0010|0x0200|0x0001,False,int(pid))
        if not handle:raise OSError('cannot open measured child')
        return ctypes.c_void_p(handle)

    @classmethod
    def snapshot(cls, pid: int, handle=None):
        if os.name == 'nt':
            counters=cls.WindowsCounters();counters.cb=ctypes.sizeof(counters)
            process=handle if handle is not None else ctypes.c_void_p(-1)
            ok=ctypes.windll.psapi.GetProcessMemoryInfo(process,ctypes.byref(counters),counters.cb)
            if not ok:return None
            return {'working_set_b':int(counters.working),'self_peak_b':int(counters.peak),
                    'private_committed_b':int(counters.private)}
        try:
            status=dict(line.split(':',1) for line in Path(f'/proc/{pid}/status').read_text().splitlines() if ':' in line)
            # A just-exited child can still have a /proc entry as a zombie,
            # with no address-space counters. Keep the last live observation.
            if not {'VmRSS', 'VmHWM'} <= status.keys():
                return None
            return {'working_set_b':int(status['VmRSS'].split()[0])*1024,
                    'self_peak_b':int(status['VmHWM'].split()[0])*1024,'private_committed_b':'NA'}
        except (FileNotFoundError,ProcessLookupError):return None


class HostState:
    @staticmethod
    def windows_admission(raw: dict[str, Any]) -> dict[str, Any]:
        """Normalize Windows' formatted CPU counters and interactive accounts.

        These counters describe the preceding WMI sampling interval, unlike the
        explicit one-second Linux delta. The observation records that distinction.
        """
        active = {}
        for row in raw["cpu"]:
            name = "cpu" if row["Name"] == "_Total" else "cpu" + row["Name"]
            value = float(row["PercentProcessorTime"]) / 100
            if not 0 <= value <= 1 or name in active:
                raise ValueError("invalid Windows CPU observation")
            active[name] = value
        if "cpu" not in active:
            raise ValueError("missing Windows aggregate CPU observation")
        available = int(raw["available_kib"]) * 1024
        if available < 0:
            raise ValueError("invalid Windows memory observation")
        return {"active": active, "available_bytes": available, "human_uids": [],
                "sessions": [str(s) for s in raw["sessions"]],
                "session_scope": "interactive logon types 2, 10 and 11",
                "cpu_scope": "Windows WMI preceding formatted sampling interval"}

    @classmethod
    def windows_observation(cls) -> dict[str, Any]:
        command = (
            "$ErrorActionPreference='Stop'; "
            "$accounts=@(Get-CimInstance Win32_LogonSession -Filter "
            "'LogonType=2 OR LogonType=10 OR LogonType=11' | "
            "Get-CimAssociatedInstance -Association Win32_LoggedOnUser | "
            "Select-Object -ExpandProperty Name -Unique); "
            "@{cpu=@(Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor | "
            "Select-Object Name,PercentProcessorTime); sessions=$accounts; "
            "available_kib=(Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory} "
            "| ConvertTo-Json -Depth 4 -Compress"
        )
        raw = subprocess.check_output(["powershell", "-NoProfile", "-Command", command],
                                      encoding="utf-8", timeout=30,
                                      creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
        return cls.windows_admission(json.loads(raw))

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
        if config.get('host_cpu_policy', 'strict') not in {'strict', 'observe'}:
            raise ValueError('host CPU policy must be strict or observe')
        if uid is None:
            getuid = getattr(os, "getuid", None)
            if getuid is None and os.name != "nt":
                raise OSError("quiet-host guard requires a Unix user ID")
            uid = int(getuid()) if getuid is not None else 0
        self.uid = uid
        self.user = getpass.getuser() if user is None else user
        for service in config.get("service_processes", []):
            if (set(service) != {"uid", "comm", "cgroup"}
                    or type(service["uid"]) is not int or service["uid"] < 1000
                    or not service["comm"] or not service["cgroup"].startswith("0::/")
                    or service["cgroup"] == "0::/"):
                raise ValueError("service exemption requires an exact UID, command and cgroup")
        wait = config.get("max_wait_seconds", 0)
        if type(wait) not in {int, float} or not 0 <= wait <= 60:
            raise ValueError("quiet-host wait must be between zero and 60 seconds")

    def reasons(self, observation: dict[str, Any]) -> list[str]:
        result = []
        if set(observation["human_uids"]) - {self.uid}:
            result.append("another user has processes on this host")
        for session in observation["sessions"]:
            fields = session.split()
            if fields and fields[0].casefold() != self.user.casefold() and session not in self.config.get("allowed_sessions", []):
                result.append("another login session is present")
        if observation["available_bytes"] < self.config.get("min_available_bytes", 32 * 1024**3):
            result.append("available memory is below the declared headroom")
        if (self.config.get('host_cpu_policy', 'strict') == 'strict'
                and observation["active"]["cpu"] > self.config.get("max_host_fraction", 0.005)):
            result.append("host CPU activity exceeds the quiet threshold")
        for cpu in self.config["cpus"]:
            if observation["active"][f"cpu{cpu}"] > self.config.get("max_core_fraction", 0.05):
                result.append(f"CPU {cpu} or its SMT sibling is busy")
        outer_config = self.config.get('outer_memory_observation')
        if outer_config:
            outer = observation.get('outer_host')
            if not outer:
                result.append('outer host memory observation unavailable')
            else:
                age = observation['observed_unix_s'] - outer['captured_unix_s']
                available = outer['available_bytes']
                if not math.isfinite(age) or not -2 <= age <= outer_config['max_age_seconds']:
                    result.append('outer host memory observation stale')
                if not math.isfinite(available) or available < outer_config['min_available_bytes']:
                    result.append('outer host available memory below headroom')
        if self.config.get('require_zero_guest_swap') and observation.get('swap_used_bytes', 0) != 0:
            result.append('guest swap is in use')
        return result

    def check(self) -> dict[str, Any]:
        deadline = time.monotonic() + self.config.get("max_wait_seconds", 0)
        attempts = 0
        while True:
            observation = self.observe()
            attempts += 1
            reasons = self.reasons(observation)
            if not reasons:
                if attempts > 1:
                    observation["admission_attempts"] = attempts
                return observation
            if time.monotonic() >= deadline:
                raise HostBusy(observation, reasons)
            # No workload is admitted during this wait. Thresholds and checks
            # stay unchanged; a short background burst may finish on its own.
            time.sleep(1)

    def observe(self) -> dict[str, Any]:
        if os.name == "nt":
            observation = HostState.windows_observation()
        elif platform.system() == "Linux":
            observation = self.linux_observation()
        else:
            raise OSError("quiet-host guard requires Windows or Linux")
        outer_config = self.config.get('outer_memory_observation')
        if outer_config:
            try:
                observation['outer_host'] = json.loads(Path(outer_config['path']).read_text(encoding='utf8'))
            except (OSError, ValueError) as error:
                observation['outer_host_error'] = str(error)
            observation['observed_unix_s'] = time.time()
        return observation

    def process_accounts(self) -> dict[str, Any]:
        """Classify declared service identities without exempting their CPU load.

        A UID alone is never exempt: another process under the same account or a
        replacement container must be reviewed separately. Interactive sessions
        are independently checked even when their UID has a declared service.
        """
        rules = self.config.get("service_processes", [])
        uids, services = set(), []
        for line in subprocess.check_output(["ps", "-eo", "uid=,pid=,comm="], encoding="utf-8").splitlines():
            uid_text, pid_text, comm = line.split(None, 2)
            uid, pid = int(uid_text), int(pid_text)
            if uid < 1000 or uid == 65534:
                continue
            candidates = [r for r in rules if r["uid"] == uid and r["comm"] == comm]
            if candidates:
                try:
                    cgroup = (Path("/proc") / str(pid) / "cgroup").read_text().strip()
                except FileNotFoundError:
                    continue  # The observed process exited before identification.
                identity = {"uid": uid, "comm": comm, "cgroup": cgroup}
                if identity in candidates:
                    services.append(dict(identity, pid=pid))
                    continue
            uids.add(uid)
        return {"human_uids": sorted(uids), "service_processes": services}

    def linux_observation(self) -> dict[str, Any]:
        first = HostState.linux_cpus()
        time.sleep(1)
        last = HostState.linux_cpus()
        selected = ["cpu", *(f"cpu{cpu}" for cpu in self.config["cpus"])]
        active = {cpu: HostState.active_fraction(first[cpu], last[cpu]) for cpu in selected}
        memory = HostState.memory()
        return {
            "sessions": HostState.sessions(),
            **self.process_accounts(),
            "available_bytes": memory["MemAvailable"], "active": active,
            "swap_used_bytes": memory.get('SwapTotal', 0) - memory.get('SwapFree', 0),
        }
