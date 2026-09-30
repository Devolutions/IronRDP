import datetime
import json
import os
import pathlib
import subprocess
import sys
import time

output = pathlib.Path(sys.argv[1])
command = sys.argv[2:]
if not command:
    raise ValueError("a build command is required")

started_utc = datetime.datetime.now(datetime.timezone.utc).isoformat()
started_monotonic = time.monotonic_ns()
timer = time.perf_counter()
process = subprocess.Popen(command, stderr=subprocess.STDOUT)
exit_code = process.wait()
seconds = time.perf_counter() - timer
ended_monotonic = time.monotonic_ns()
ended_utc = datetime.datetime.now(datetime.timezone.utc).isoformat()

interval = {
    "instrumented": True,
    "command": command,
    "processId": process.pid,
    "parentProcessId": os.getpid(),
    "startedUtc": started_utc,
    "endedUtc": ended_utc,
    "startedMonotonicNs": started_monotonic,
    "endedMonotonicNs": ended_monotonic,
    "coreBuildSeconds": round(seconds, 3),
    "exitCode": exit_code,
}
if sys.platform == "linux":
    import resource

    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    interval["resources"] = {
        "userCpuSeconds": usage.ru_utime,
        "systemCpuSeconds": usage.ru_stime,
        "maxResidentSetKiB": usage.ru_maxrss,
        "minorFaults": usage.ru_minflt,
        "majorFaults": usage.ru_majflt,
        "voluntaryContextSwitches": usage.ru_nvcsw,
        "involuntaryContextSwitches": usage.ru_nivcsw,
    }
output.write_text(json.dumps(interval, indent=2) + "\n", encoding="utf-8")
sys.exit(exit_code)
