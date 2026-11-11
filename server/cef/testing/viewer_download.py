#!/usr/bin/env python3
"""Verify a download through the visible RPC viewer and its restricted client directory."""

import argparse
import ctypes
import json
from pathlib import Path
import subprocess
import sys
import time

import downloads


def endpoint_ready(endpoint):
    if sys.platform != "win32":
        return endpoint.exists()
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    wait = kernel.WaitNamedPipeW
    wait.argtypes = (ctypes.c_wchar_p, ctypes.c_uint32)
    wait.restype = ctypes.c_int
    if wait(str(endpoint), 1):
        return True
    error = ctypes.get_last_error()
    if error in (2, 121, 231):
        return False
    raise ctypes.WinError(error)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-bin", required=True)
    parser.add_argument("--agent", required=True)
    parser.add_argument("--viewer", required=True)
    parser.add_argument("--artifacts", default="server/cef/artifacts/viewer-download-e2e")
    parser.set_defaults(server_arg=[], skip_baseline=True, shutdown_only=False,
                        rejected_device_only=False, tab_close_only=False)
    args = parser.parse_args()
    if sys.platform not in ("darwin", "linux", "win32"):
        parser.error("the restricted download client is available on Windows, macOS, and Linux")

    runner = downloads.DownloadRunner(args)
    viewer_log = (runner.artifacts / "viewer.log").open("w", encoding="utf-8")
    viewer = None
    failed = False
    print("Artifacts: " + str(runner.artifacts), flush=True)
    try:
        runner.start_fixture()
        viewer_env = runner.env.copy()
        viewer_env["IRONRDP_LOG"] = "warn,ironrdp_viewer::app=debug"
        viewer = subprocess.Popen(
            [str(Path(args.viewer).resolve()), "--rpc", "--rpc-endpoint", str(runner.socket),
             "--rdpdr-download-dir", str(runner.client_directory)],
            env=viewer_env, stdout=viewer_log, stderr=subprocess.STDOUT,
        )
        deadline = time.monotonic() + 30
        while not endpoint_ready(runner.socket):
            if viewer.poll() is not None:
                raise RuntimeError("Viewer exited before publishing its RPC endpoint")
            if time.monotonic() >= deadline:
                raise TimeoutError("Viewer RPC endpoint did not become ready")
            time.sleep(0.05)
        runner.daemon_started = True
        # oracle: mutant loss of a resize made before the RPC connection.
        deadline = time.monotonic() + 30
        while True:
            log = (runner.artifacts / "viewer.log").read_text(encoding="utf-8")
            if ("Deferring resize until a session is available" in log
                    or "Unable to resize the RPC-backed RDP session" in log):
                break
            if viewer.poll() is not None:
                raise RuntimeError("Viewer exited before attempting its initial resize")
            if time.monotonic() >= deadline:
                raise TimeoutError("Viewer did not attempt its initial resize before connecting")
            time.sleep(0.05)
        runner.size = (1024, 768)
        endpoint = runner.start_server("browser")
        runner.connect(endpoint)
        runner.open_download_page("download-page")
        # oracle: mutant viewer omitting the selected restricted RDPDR backend.
        first_download = runner.download("client-download", "normal", downloads.NORMAL, "browser.bin")
        runner.agent("disconnect")
        # oracle: spec MS-RDPBCGR §1.3.1.4.2 permits the server to close after a shutdown request.
        deadline = time.monotonic() + 15
        while True:
            status = runner.agent("status")
            if "state: Disconnected" in status.splitlines():
                if "detail: UserInitiated" not in status.splitlines():
                    raise AssertionError("Session did not report a client-initiated disconnect: " + status)
                runner.report["disconnect_status"] = status.strip()
                break
            if "state: Failed" in status.splitlines():
                raise AssertionError("Session failed after client disconnect: " + status)
            if time.monotonic() >= deadline:
                raise TimeoutError("Session did not disconnect: " + status)
            time.sleep(0.05)
        # oracle: mutant losing the viewer's dimensions after disconnecting and reconnecting.
        runner.connect(endpoint)
        runner.open_download_page("reconnected-download-page")
        second_download = runner.download("reconnected-client-download", "normal", downloads.NORMAL)
        if first_download == second_download or first_download.read_bytes() != downloads.NORMAL:
            raise AssertionError("Reconnected download replaced the earlier client file")
    except Exception as error:
        runner.report["error"] = "{}: {}".format(type(error).__name__, error)
        print(runner.report["error"], file=sys.stderr)
        failed = True
    finally:
        if viewer is not None and not runner.daemon_started and viewer.poll() is None:
            viewer.terminate()
            try:
                viewer.wait(timeout=5)
            except subprocess.TimeoutExpired:
                viewer.kill()
                viewer.wait(timeout=5)
                runner.report["viewer_error"] = "viewer did not exit after startup failure"
                failed = True
        if runner.cleanup():
            failed = True
        if viewer is not None:
            try:
                viewer.wait(timeout=15)
            except subprocess.TimeoutExpired:
                viewer.kill()
                viewer.wait(timeout=5)
                runner.report["viewer_error"] = "viewer did not exit after RPC shutdown"
                failed = True
            runner.report["viewer_exit_code"] = viewer.returncode
            if viewer.returncode != 0:
                failed = True
        viewer_log.close()
        runner.report["passed"] = not failed
        (runner.artifacts / "report.json").write_text(
            json.dumps(runner.report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
