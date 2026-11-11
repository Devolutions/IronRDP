#!/usr/bin/env python3
"""Verify browser attachments delivered over RDPDR to an isolated client folder."""

import argparse
from contextlib import contextmanager
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import threading
import time
import unicodedata
from urllib.parse import urlsplit

import e2e


NORMAL = b"IronRDP redirected browser download\x00\xff\n"
PATTERN = bytes(range(256))
PAYLOADS = {
    "normal": ("browser.bin", NORMAL),
    "empty": ("empty.bin", b""),
    "unicode": ("caf\u00e9-\u6f22\u5b57.bin", NORMAL),
    "unknown": ("unknown.bin", NORMAL * 257),
    "multi": ("multi.bin", PATTERN * 2048),
    "slow": ("slow.bin", PATTERN * 4096),
    "tab-slow": ("slow.bin", PATTERN * 4096),
    "oversized": ("oversized.bin", PATTERN * 4096 + b"!"),
    "truncated": ("truncated.bin", NORMAL),
    "transfer": ("transfer.bin", PATTERN * 262144),
    "one": ("one.bin", b"x"),
    "under": ("under.bin", NORMAL[:-1]),
    "over": ("over.bin", NORMAL + b"!"),
}


class DownloadHandler(e2e.FixtureHandler):
    def do_POST(self):
        path = urlsplit(self.path).path
        if path not in ("/tab-beforeunload", "/tab-beforeunload-armed", "/tab-unloaded"):
            self.send_error(404)
            return
        self.rfile.read(int(self.headers.get("Content-Length", "0")))
        with self.server.activity_lock:
            if path == "/tab-beforeunload":
                self.server.beforeunload_count += 1
            elif path == "/tab-beforeunload-armed":
                self.server.beforeunload_armed += 1
            else:
                self.server.unload_count += 1
        self.send_response(204)
        self.end_headers()

    def do_GET(self):
        path = urlsplit(self.path).path
        if path == "/tab-activity":
            with self.server.activity_lock:
                self.server.activity_count += 1
            self.send_response(204)
            self.end_headers()
            return
        if not path.startswith("/download/"):
            return super().do_GET()
        case = path.removeprefix("/download/")
        if case == "redirect":
            self.send_response(302)
            self.send_header("Location", "/download/normal")
            self.end_headers()
            return
        if case not in PAYLOADS:
            self.send_error(404)
            return
        name, payload = PAYLOADS[case]
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        if case == "unicode":
            self.send_header("Content-Disposition",
                             "attachment; filename*=UTF-8''caf%C3%A9-%E6%BC%A2%E5%AD%97.bin")
        else:
            self.send_header("Content-Disposition", 'attachment; filename="{}"'.format(name))
        if case not in ("unknown", "oversized"):
            length = len(payload) + (4096 if case == "truncated" else 0)
            self.send_header("Content-Length", str(length))
        self.end_headers()
        try:
            for offset in range(0, len(payload), 4096):
                self.wfile.write(payload[offset:offset + 4096])
                self.wfile.flush()
                if case in ("slow", "tab-slow"):
                    time.sleep(0.04 if case == "tab-slow" else 0.02)
        except (BrokenPipeError, ConnectionAbortedError, ConnectionResetError):
            pass


@contextmanager
def deny_client_writes(directory):
    if sys.platform == "win32":
        identity = subprocess.run(
            ["whoami.exe", "/user", "/fo", "csv", "/nh"],
            check=True, stdout=subprocess.PIPE,
        ).stdout
        match = re.search(rb"S-1-(?:\d+-)+\d+", identity)
        if match is None:
            raise RuntimeError("Unable to identify the current Windows user")
        sid = "*" + match.group().decode("ascii")
        subprocess.run(["icacls.exe", str(directory), "/deny", sid + ":(WD)"], check=True, capture_output=True)
        try:
            yield
        finally:
            subprocess.run(["icacls.exe", str(directory), "/remove:d", sid], check=True, capture_output=True)
    else:
        directory.chmod(0o500)
        try:
            yield
        finally:
            directory.chmod(0o700)


class DownloadRunner(e2e.Runner):
    fixture_handler = DownloadHandler

    def __init__(self, args):
        super().__init__(args)
        self.client_directory = self.artifacts / "client-downloads"
        self.client_directory.mkdir(mode=0o700)
        self.extra_server_args = args.server_arg or [
            "--downloads", "--download-max-file-bytes=1048576",
            "--download-max-session-bytes=4194304",
        ]
        self.redirect_downloads = True
        self.skip_baseline = args.skip_baseline
        self.shutdown_only = args.shutdown_only
        self.rejected_device_only = args.rejected_device_only
        self.tab_close_only = args.tab_close_only
        self.checked_device_conflicts = False
        self.report["download_checks"] = []

    def agent(self, *args, **kwargs):
        if args == ("daemon", "start") and self.redirect_downloads:
            args += ("--rdpdr-download-dir", str(self.client_directory))
        return super().agent(*args, **kwargs)

    def start_server(self, generation, extra_args=None):
        return super().start_server(generation, self.extra_server_args if extra_args is None else list(extra_args))

    def start_fixture(self):
        super().start_fixture()
        self.http.activity_lock = threading.Lock()
        self.http.activity_count = 0
        self.http.beforeunload_count = 0
        self.http.beforeunload_armed = 0
        self.http.unload_count = 0

    def activity_count(self):
        with self.http.activity_lock:
            return self.http.activity_count

    def unload_count(self):
        with self.http.activity_lock:
            return self.http.unload_count

    def beforeunload_counts(self):
        with self.http.activity_lock:
            return self.http.beforeunload_armed, self.http.beforeunload_count

    def open_download_page(self, label, activity=False):
        self.shortcut(0x26)
        self.agent("type-unicode", "--text", self.base_url + "downloads.html" + ("?activity" if activity else ""))
        self.key(0x1c)
        self.content(label, [(440, 48, "22cc88")])

    def files(self):
        return {entry.name: entry for entry in self.client_directory.iterdir() if entry.is_file()}

    def log(self):
        path = self.artifacts / self.report["servers"][-1]["artifacts"] / "server.log"
        return re.sub(r"\x1b\[[0-9;]*m", "", path.read_text(encoding="utf-8"))

    def wait_log(self, marker, offset):
        deadline = time.monotonic() + e2e.TIMEOUT
        while time.monotonic() < deadline:
            for line in self.log()[offset:].splitlines():
                if marker in line:
                    return line
            time.sleep(0.02)
        raise AssertionError("Missing server outcome: " + marker)

    def connect(self, endpoint, extra_args=(), *, expect_download_device=True):
        if sys.platform == "win32" and expect_download_device and not self.checked_device_conflicts:
            self.reject_conflicting_devices(endpoint)
        offset = len(self.log())
        super().connect(endpoint, extra_args)
        if expect_download_device:
            self.wait_log("Approved download drive announced", offset)

    def reject_conflicting_devices(self, endpoint):
        properties = (
            "ironrdp_smartcard:i:1",
            "redirectprinters:i:1",
            "redirectcomports:i:1",
            "drivestoredirect:s:*",
        )
        for prop in properties:
            result = self.agent_result(
                "connect", "--no-prompt", "--no-auto-start", "--server", endpoint,
                "--username", "browser", "--password", "test", "--prop", prop,
            )
            if result.returncode == 0 or "rdpdr download directory cannot be combined" not in result.stdout:
                raise AssertionError("Conflicting device redirection was not rejected: " + prop + "\n" + result.stdout)
            if "state: NoSession" not in self.agent("status").splitlines():
                raise AssertionError("Rejected device redirection started a session: " + prop)
        self.checked_device_conflicts = True
        self.report["download_checks"].append({"name": "conflicting-windows-devices", "properties": properties})

    def trigger(self, case):
        self.shortcut(0x26)
        self.agent("type-unicode", "--text", self.base_url + "download/" + case)
        self.key(0x1c)

    def clean_staging(self):
        root = self.profile / "downloads"
        deadline = time.monotonic() + e2e.TIMEOUT
        while time.monotonic() < deadline:
            try:
                staged = list(root.iterdir())
            except FileNotFoundError:
                staged = []
            if not staged and not any(name.endswith(".part") for name in self.files()):
                return
            time.sleep(0.02)
        raise AssertionError("Uncommitted files remain: {}".format(staged))

    def download(self, name, case, payload, expected_name=None):
        before = set(self.files())
        offset = len(self.log())
        self.trigger(case)
        deadline = time.monotonic() + e2e.TIMEOUT
        while time.monotonic() < deadline:
            new = [entry for filename, entry in self.files().items()
                   if filename not in before and not filename.endswith(".part")]
            if len(new) == 1 and 'download_state="completed"' in self.log()[offset:]:
                data = new[0].read_bytes()
                if data == payload:
                    if expected_name is not None:
                        if unicodedata.normalize("NFC", new[0].name) != expected_name:
                            raise AssertionError("Unexpected client filename: " + new[0].name)
                    terminal = self.wait_log('download_state="completed"', offset)
                    delivered = int(re.search(r"\brdpdr_bytes=(\d+)", terminal).group(1))
                    writes = int(re.search(r"\brdpdr_writes=(\d+)", terminal).group(1))
                    if delivered != len(payload) or bool(writes) != bool(payload):
                        raise AssertionError("Incorrect acknowledged RDPDR byte/write counts")
                    self.clean_staging()
                    self.report["download_checks"].append({
                        "name": name,
                        "file": new[0].name,
                        "bytes": len(data),
                        "sha256": hashlib.sha256(data).hexdigest(),
                        "rdpdr_bytes": delivered,
                        "rdpdr_writes": writes,
                    })
                    self.capture(name)
                    return new[0]
            if self.server.poll() is not None:
                raise RuntimeError("Server exited during " + name)
            time.sleep(0.05)
        raise AssertionError("No exact client payload received for " + name)

    def rejected(self, name, trigger, reason, screenshot=True):
        before = set(self.files())
        offset = len(self.log())
        trigger()
        terminal = self.wait_log(reason, offset)
        self.clean_staging()
        if set(self.files()) != before:
            raise AssertionError("Rejected download left a client file: " + name)
        if screenshot:
            self.capture(name)
        self.report["download_checks"].append({"name": name, "outcome": terminal})

    def restart(self, name, limits=None):
        status = self.agent("status").splitlines()
        if "state: Connected" in status:
            self.agent("disconnect")
        elif "state: Disconnected" not in status:
            raise AssertionError("Unexpected session state before restarting: " + repr(status))
        self.stop_server()
        self.connect(self.start_server(name, limits))
        self.open_download_page(name + "-ready")

    def rejected_device(self):
        self.connect(self.start_server("rejected-device", []), expect_download_device=False)
        # oracle: mutant a rejected redirected drive tears down the browser session.
        self.content("rejected-device-browser", [(48, 48, "ff0000")])
        self.rejected("rejected-device-download", lambda: self.trigger("normal"), "downloads are disabled")

    def download_after_tab_close(self):
        self.open_download_page("tab-close-download-page", activity=True)
        deadline = time.monotonic() + e2e.TIMEOUT
        while self.activity_count() < 2:
            if time.monotonic() >= deadline:
                raise AssertionError("The originating tab did not start its activity probe")
            time.sleep(0.05)
        self.click(600, e2e.CHROME_HEIGHT + 112)
        self.shortcut(0x14)
        self.navigate("index.html")
        self.content("tab-close-other-tab", [(48, 48, "ff0000")])
        self.click(40, 16)
        self.content("tab-close-originating-tab", [(440, 48, "22cc88")])
        before = set(self.files())
        offset = len(self.log())
        self.click(450, e2e.CHROME_HEIGHT + 48)
        self.wait_log('download_state="browser"', offset)
        self.click(600, e2e.CHROME_HEIGHT + 192)
        deadline = time.monotonic() + e2e.TIMEOUT
        while self.beforeunload_counts()[0] == 0:
            if time.monotonic() >= deadline:
                raise AssertionError("The page did not arm its before-unload handler")
            time.sleep(0.05)
        if self.unload_count() != 0:
            raise AssertionError("The download unloaded its page before the tab was closed")
        self.shortcut(0x11)
        self.content("tab-close-returned-to-other-tab", [(48, 48, "ff0000")])
        deadline = time.monotonic() + 3
        while (self.unload_count() == 0 or self.beforeunload_counts()[1] == 0) and time.monotonic() < deadline:
            time.sleep(0.05)
        # oracle: mutant hiding a page whose before-unload prompt prevented its navigation.
        if self.unload_count() == 0:
            raise AssertionError("The closed tab did not unload its page")
        if self.beforeunload_counts()[1] == 0:
            raise AssertionError("The closed tab did not run its before-unload handler")
        time.sleep(0.3)
        count = self.activity_count()
        time.sleep(2.2)
        # oracle: mutant leaving the closed page running while its download remains active.
        if self.activity_count() != count:
            raise AssertionError("The closed tab continued making requests during its download")
        if 'download_state="completed"' in self.log()[offset:]:
            raise AssertionError("The download completed before the closed-tab activity check")
        # oracle: mutant closing the originating tab strands an active browser download.
        self.wait_log('download_state="completed"', offset)
        created = [path for name, path in self.files().items() if name not in before]
        if len(created) != 1 or created[0].read_bytes() != PAYLOADS["slow"][1]:
            raise AssertionError("Closing the tab lost or corrupted its download")
        self.clean_staging()

        self.shortcut(0x14)
        self.open_download_page("tab-close-cancel-page")
        offset = len(self.log())
        self.trigger("slow")
        self.wait_log('download_state="browser"', offset)
        self.shortcut(0x11)
        self.content("tab-close-cancel-returned", [(48, 48, "ff0000")])
        # oracle: mutant cancellation after closing a tab leaves staged bytes or a concurrency slot.
        self.rejected("tab-close-cancel", lambda: self.click(self.size[0] - 48, self.size[1] - 16),
                      'download_state="cancelled"')
        self.download("tab-close-next-download", "normal", NORMAL)
        self.report["download_checks"].append({"name": "download-after-tab-close"})

    def concurrent_downloads(self):
        before = set(self.files())
        offset = len(self.log())
        self.trigger("slow")
        self.wait_log('download_state="browser"', offset)
        second = len(self.log())
        self.trigger("slow")
        self.wait_log('download_state="browser"', second)
        self.trigger("normal")
        self.wait_log("download concurrency limit reached", second)
        self.capture("36-two-active-downloads")
        deadline = time.monotonic() + e2e.TIMEOUT
        while time.monotonic() < deadline:
            completed = self.log()[offset:].count('download_state="completed"')
            if completed == 2:
                self.clean_staging()
                created = [path for name, path in self.files().items() if name not in before]
                if len(created) != 2 or any(path.read_bytes() != PAYLOADS["slow"][1] for path in created):
                    raise AssertionError("Concurrency test did not preserve exactly two payloads")
                self.capture("36-concurrent-downloads-completed")
                self.report["download_checks"].append({
                    "name": "36-concurrency-limit",
                    "completed": [path.name for path in created],
                    "rejected": "third simultaneous download",
                })
                return
            time.sleep(0.02)
        raise AssertionError("Concurrent downloads did not complete")

    def interrupted_transfer(self):
        self.restart("transfer-disconnect", (
            "--downloads", "--download-max-file-bytes=67108864",
            "--download-max-session-bytes=134217728",
        ))
        before = set(self.files())
        offset = len(self.log())
        self.trigger("transfer")
        deadline = time.monotonic() + e2e.TIMEOUT
        while time.monotonic() < deadline:
            for name, path in self.files().items():
                if name.endswith(".part"):
                    try:
                        size = path.stat().st_size
                    except FileNotFoundError:
                        continue
                    if 0 < size < len(PAYLOADS["transfer"][1]):
                        self.agent("disconnect")
                        self.wait_log("Download interrupted by client disconnect", offset)
                        self.clean_staging()
                        if set(self.files()) != before:
                            raise AssertionError("Disconnected transfer published a file")
                        self.report["download_checks"].append({
                            "name": "40-mid-rdpdr-disconnect",
                            "observed_client_partial_bytes": size,
                        })
                        if "Stopping server to fence an unacknowledged download IRP" in self.log()[offset:]:
                            self.server.wait(timeout=15)
                        self.stop_server()
                        self.connect(self.start_server("after-transfer-disconnect"))
                        self.open_download_page("41-fresh-peer")
                        self.download("41-no-stale-transfer", "normal", NORMAL)
                        return
            time.sleep(0.001)
        raise AssertionError("Could not observe an in-progress client transfer to interrupt")

    def shutdown_during_browser_download(self, completed, expected_bytes):
        offset = len(self.log())
        before = set(self.files())
        self.trigger("slow")
        self.wait_log('download_state="browser"', offset)
        # oracle: mutant SIGINT that publishes an unfinished download or retains its profile.
        if not self.stop_server():
            raise AssertionError("Server exited before SIGINT during browser download")
        deadline = time.monotonic() + e2e.TIMEOUT
        while True:
            status = self.agent("status")
            if "state: Disconnected" in status.splitlines():
                break
            if "state: Failed" in status.splitlines():
                raise AssertionError("Session failed after server shutdown: " + status)
            if time.monotonic() >= deadline:
                raise AssertionError("Session did not terminate after server shutdown: " + status)
            time.sleep(0.05)
        # oracle: spec MS-RDPBCGR §2.2.5.1.1 identifies server shutdown as error 0x00000019.
        if "The remote server is busy shutting down" not in status:
            raise AssertionError("Session did not receive the server shutdown reason: " + status)
        if set(self.files()) != before:
            raise AssertionError("Server shutdown published an unfinished browser download")
        self.report["download_checks"].append({"name": "42-sigint-during-browser-download"})
        if any(name.endswith(".part") for name in self.files()):
            raise AssertionError("Uncommitted client files survived shutdown")
        if completed.read_bytes() != expected_bytes:
            raise AssertionError("Completed client download did not survive shutdown")

    def run(self):
        if self.tab_close_only:
            self.start_fixture()
            self.daemon_started = True
            self.agent("daemon", "start")
            self.connect(self.start_server("tab-close"))
            self.download_after_tab_close()
            return
        if self.rejected_device_only:
            self.start_fixture()
            self.daemon_started = True
            self.agent("daemon", "start")
            self.rejected_device()
            return
        if self.shutdown_only:
            self.start_fixture()
            self.daemon_started = True
            self.agent("daemon", "start")
            self.connect(self.start_server("shutdown"))
            self.open_download_page("shutdown-download-page")
            completed = self.download("shutdown-completed-download", "normal", NORMAL, "browser.bin")
            self.shutdown_during_browser_download(completed, NORMAL)
            return
        if self.skip_baseline:
            self.start_fixture()
            self.daemon_started = True
            self.agent("daemon", "start")
            self.connect(self.start_server("downloads"))
            self.content("01-initial", [(48, 48, "ff0000")])
        else:
            super().run()
        self.open_download_page("21-download-page")
        first = self.download("22-binary-download", "normal", NORMAL, "browser.bin")
        self.click(1184, 744)
        self.content("22-download-bar-preserves-viewport", [(1110, 644, "44dd11")])
        self.shortcut(0x14)
        self.navigate("index.html")
        self.content("22-status-new-tab", [(48, 48, "ff0000")])
        self.click(150, e2e.CHROME_HEIGHT + 388)
        self.expect_popup("22-status-popup-open", True)
        self.key(0x01)
        self.expect_popup("22-status-popup-dismissed", False)
        self.agent("resize", "--width", "1024", "--height", "768")
        self.size = (1024, 768)
        self.expect("22-status-resize", [(48, 136, "ff0000"), (1000, 712, "123456")])
        self.agent("resize", "--width", "1280", "--height", "800")
        self.size = (1280, 800)
        # oracle: spec MS-RDPBCGR §3.3.5.8.1.2: an in-flight keyboard event reaches the session.
        self.shortcut(0x11)
        self.content("22-status-original-tab-restored", [(1110, 644, "44dd11")])
        second = self.download("23-duplicate-download", "normal", NORMAL)
        if first == second or first.read_bytes() != NORMAL:
            raise AssertionError("Duplicate download overwrote an existing client file")
        self.download("24-empty-download", "empty", b"", "empty.bin")
        self.download("25-unicode-download", "unicode", NORMAL, "caf\u00e9-\u6f22\u5b57.bin")
        self.download("26-redirect-download", "redirect", NORMAL)
        self.download("27-unknown-length-download", "unknown", PAYLOADS["unknown"][1], "unknown.bin")
        self.download("28-multi-chunk-download", "multi", PAYLOADS["multi"][1], "multi.bin")
        self.rejected("29-over-limit-rejected", lambda: self.trigger("oversized"),
                      "download exceeds the file byte limit")
        self.rejected("30-interrupted-cleanup", lambda: self.trigger("truncated"),
                      "Browser cancelled or interrupted download")
        self.rejected("31-native-upload-denied",
                      lambda: self.click(65, e2e.CHROME_HEIGHT + 516),
                      "native file dialogs and uploads are disabled")
        before = set(self.files())
        offset = len(self.log())
        self.trigger("slow")
        self.wait_log('download_state="browser"', offset)
        self.rejected("32-user-cancelled", lambda: self.click(self.size[0] - 48, self.size[1] - 16),
                      'download_state="cancelled"')
        if set(self.files()) != before:
            raise AssertionError("Cancelled download published a client file")
        self.restart("boundaries")
        self.download("33-exact-file-limit", "slow", PAYLOADS["slow"][1], "slow.bin")
        self.concurrent_downloads()
        endpoint = self.report["servers"][-1]["endpoint"]
        self.agent("disconnect")
        self.connect(endpoint, ("--prop", "ironrdp_rdpdr:i:0"), expect_download_device=False)
        self.rejected("34-missing-drive", lambda: self.trigger("normal"),
                      "approved DL redirected drive is unavailable")
        self.agent("disconnect")
        self.connect(endpoint)
        with deny_client_writes(self.client_directory):
            self.rejected("35-client-write-denied", lambda: self.trigger("normal"),
                          "client rejected Create", screenshot=False)
        self.restart("small-file-limit", (
            "--downloads", "--download-max-file-bytes=38", "--download-max-session-bytes=152",
        ))
        self.download("37-one-below-limit", "under", NORMAL[:-1])
        self.download("37-exact-limit", "normal", NORMAL)
        self.rejected("37-one-above-limit", lambda: self.trigger("over"),
                      "download exceeds the file byte limit")
        self.restart("small-session-limit", (
            "--downloads", "--download-max-file-bytes=38", "--download-max-session-bytes=76",
        ))
        self.download("38-session-first-half", "normal", NORMAL)
        self.download("38-session-exact-boundary", "normal", NORMAL)
        self.rejected("38-session-exhausted", lambda: self.trigger("one"),
                      "download session byte budget exhausted")
        self.restart("browser-disconnect")
        offset = len(self.log())
        before = set(self.files())
        self.trigger("slow")
        self.wait_log('download_state="browser"', offset)
        self.agent("disconnect")
        self.wait_log("Download interrupted by client disconnect", offset)
        self.clean_staging()
        if set(self.files()) != before:
            raise AssertionError("Disconnect during browser download published a file")
        self.report["download_checks"].append({"name": "39-mid-browser-disconnect"})
        self.connect(self.report["servers"][-1]["endpoint"])
        self.download("39-reconnected-download", "normal", NORMAL)
        self.interrupted_transfer()
        self.download_after_tab_close()
        self.shutdown_during_browser_download(first, NORMAL)
        self.rejected_device()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-bin", required=True)
    parser.add_argument("--agent", required=True)
    parser.add_argument("--artifacts", default="server/cef/artifacts/download-e2e")
    parser.add_argument("--server-arg", action="append", default=[])
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--skip-baseline", action="store_true")
    selection.add_argument("--shutdown-only", action="store_true")
    selection.add_argument("--rejected-device-only", action="store_true")
    selection.add_argument("--tab-close-only", action="store_true")
    args = parser.parse_args()
    if sys.platform not in ("darwin", "linux", "win32"):
        parser.error("download acceptance requires a supported restricted client sink")
    runner = DownloadRunner(args)
    print("Artifacts: " + str(runner.artifacts), flush=True)
    failed = False
    try:
        runner.run()
    except Exception as error:
        runner.report["error"] = "{}: {}".format(type(error).__name__, error)
        print(runner.report["error"], file=sys.stderr)
        failed = True
    finally:
        if runner.cleanup():
            failed = True
        runner.report["passed"] = not failed
        (runner.artifacts / "report.json").write_text(
            json.dumps(runner.report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
