#!/usr/bin/env python3
"""Exercise single-interrupt shutdown across repeated native server starts."""

import argparse
import ctypes
import json
import os
from pathlib import Path
import random
import shutil
import subprocess
import sys
import time
import uuid

from e2e import interrupt_server, require_background_bundle, server_process_options


def helper_pids(profile):
    if sys.platform == "win32":
        env = os.environ.copy()
        env["IRONRDP_CEF_TEST_PROFILE"] = str(profile)
        command = (
            "Get-CimInstance Win32_Process | Where-Object { "
            "$_.Name -eq 'ironrdp_cef_server.exe' -and $_.CommandLine -and "
            "$_.CommandLine.Contains('--user-data-dir=') -and "
            "$_.CommandLine.Contains($env:IRONRDP_CEF_TEST_PROFILE) "
            "} | Select-Object -ExpandProperty ProcessId"
        )
        output = subprocess.run(
            ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", command],
            env=env, check=True, capture_output=True, text=True, timeout=5,
        ).stdout
        return [int(line.strip()) for line in output.splitlines() if line.strip()]
    output = subprocess.run(
        ["ps", "-xww", "-o", "pid=,command="],
        check=True, capture_output=True, text=True, timeout=5,
    ).stdout
    marker = "--user-data-dir=" + str(profile)
    return [
        int(line.split(maxsplit=1)[0])
        for line in output.splitlines()
        if ("ironrdp-cef-server Helper" in line or "ironrdp-cef-helper" in line) and marker in line
    ]


def lock_windows_profile(profile):
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    create_file = kernel32.CreateFileW
    create_file.argtypes = (ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p,
                            ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p)
    create_file.restype = ctypes.c_void_p
    kernel32.CloseHandle.argtypes = (ctypes.c_void_p,)
    kernel32.CloseHandle.restype = ctypes.c_int
    handle = create_file(str(profile / ".cleanup-lock"), 0x40000000, 3, None, 1, 0x80, None)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    return kernel32, handle


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-bin", required=True)
    parser.add_argument("--iterations", type=int, default=100)
    parser.add_argument("--phase", choices=("startup", "immediate", "ready"), default="ready")
    parser.add_argument("--fail-log-copy", action="store_true")
    parser.add_argument("--hold-profile-lock", action="store_true")
    parser.add_argument("--artifacts", default="server/cef/artifacts/shutdown-stress")
    args = parser.parse_args()
    if sys.platform == "win32" and args.fail_log_copy:
        parser.error("--fail-log-copy applies only when CEF logs are copied from the profile")
    if args.hold_profile_lock and (sys.platform != "win32" or args.phase != "ready" or args.fail_log_copy):
        parser.error("--hold-profile-lock requires Windows and the ready phase")
    if args.iterations < 1:
        parser.error("--iterations must be positive")

    server = Path(args.server_bin).resolve()
    require_background_bundle(server)
    root = Path(args.artifacts).resolve()
    root.mkdir(parents=True, exist_ok=True)
    run = root / ("run-" + uuid.uuid4().hex)
    run.mkdir()
    print("Artifacts: " + str(run), flush=True)
    report = {"passed": False, "iterations": []}
    phases = random.Random(1729)
    try:
        for number in range(args.iterations):
            directory = run / "{:03d}".format(number)
            directory.mkdir()
            if args.fail_log_copy:
                (directory / "cef.log").mkdir()
            record = {"iteration": number, "phase": args.phase}
            report["iterations"].append(record)
            with (directory / "server.log").open("w", encoding="utf-8") as log:
                process = subprocess.Popen(
                    [str(server), "--bind-addr", "127.0.0.1:0", "--url", "about:blank",
                     "--artifacts", str(directory)],
                    stdout=log, stderr=subprocess.STDOUT, **server_process_options(),
                )
                try:
                    deadline = time.monotonic() + 15
                    address = directory / "server.addr"
                    profile_file = directory / "profile.path"
                    while not (profile_file.is_file() and profile_file.stat().st_size):
                        if process.poll() is not None:
                            raise RuntimeError("Server exited before publishing its profile path")
                        if time.monotonic() >= deadline:
                            raise RuntimeError("Server profile creation timed out")
                        time.sleep(0.001)
                    profile = Path(profile_file.read_text(encoding="utf-8").strip())
                    record["profile"] = str(profile)
                    if profile.parent != directory or not profile.name.startswith("profile-"):
                        raise RuntimeError("Published browser profile is outside the test directory")
                    if not profile.is_dir():
                        raise RuntimeError("Published browser profile does not exist")
                    if args.phase == "startup":
                        if address.exists():
                            raise RuntimeError("Server became ready before the startup signal")
                    else:
                        while not (address.is_file() and address.stat().st_size):
                            if process.poll() is not None:
                                raise RuntimeError("Server exited before publishing its listener")
                            if time.monotonic() >= deadline:
                                raise RuntimeError("Server readiness timed out")
                            time.sleep(0.01)
                        if args.phase == "ready":
                            deadline = time.monotonic() + 10
                            while not (helpers := helper_pids(profile)):
                                if process.poll() is not None:
                                    raise RuntimeError("Server exited before starting a CEF helper")
                                if time.monotonic() >= deadline:
                                    raise RuntimeError("CEF helper startup timed out")
                                time.sleep(0.05)
                            record["observed_cef_helpers"] = helpers
                            time.sleep(0.15 + phases.random() * 0.03)
                    lock = lock_windows_profile(profile) if args.hold_profile_lock else None
                    try:
                        # oracle: mutant lost interrupt, retained profile, or orphaned CEF helper.
                        interrupt_server(process)
                        if lock is not None:
                            # oracle: mutant transient sharing violation prematurely aborts cleanup.
                            time.sleep(1)
                            if process.poll() is not None:
                                raise RuntimeError("Server exited while the browser profile was temporarily locked")
                    finally:
                        if lock is not None and not lock[0].CloseHandle(lock[1]):
                            raise ctypes.WinError(ctypes.get_last_error())
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired as error:
                        raise RuntimeError("Server did not exit after one interrupt") from error
                    if args.fail_log_copy:
                        # oracle: mutant deleting the only CEF log after artifact-copy failure.
                        if process.returncode == 0:
                            raise RuntimeError("Server did not report the failed CEF log copy")
                        if not (profile / "cef.log").is_file():
                            raise RuntimeError("Failed CEF log copy did not retain the browser profile and log")
                        if "failed to preserve CEF log" not in (directory / "server.log").read_text(encoding="utf-8"):
                            raise RuntimeError("Server did not identify the failed CEF log copy")
                        record["retained_cef_log"] = True
                    else:
                        if process.returncode != 0:
                            raise RuntimeError("Server exited with return code {}".format(process.returncode))
                        if profile.exists():
                            raise RuntimeError("Server did not remove its browser profile")
                    deadline = time.monotonic() + 5
                    while helpers := helper_pids(profile):
                        if time.monotonic() >= deadline:
                            raise RuntimeError("CEF helpers survived server shutdown: {}".format(helpers))
                        time.sleep(0.05)
                    record["cef_helpers_stopped"] = True
                    if args.fail_log_copy:
                        shutil.copy2(profile / "cef.log", directory / "retained-cef.log")
                        shutil.rmtree(profile)
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait(timeout=5)
                        record["forced_termination"] = True
                    record["return_code"] = process.returncode
            if (number + 1) % 20 == 0:
                print("Completed {} shutdowns".format(number + 1), flush=True)
        report["passed"] = True
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        report["error"] = "{}: {}".format(type(error).__name__, error)
        print(report["error"], file=sys.stderr)
    finally:
        (run / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
