#!/usr/bin/env python3
"""Verify redirected camera and microphone streams through a loopback RDP client."""

import argparse
import http.server
import json
import os
from pathlib import Path
import ssl
import subprocess
import threading
import time
import uuid

from e2e import interrupt_server, server_process_options

HERE = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--server-bin", required=True, help="Server bundle built against patched CEF")
parser.add_argument("--client-bin", required=True, help="Path to the rdp-media-fixture binary")
parser.add_argument("--artifacts", default="server/cef/artifacts/rdp-media-e2e")
parser.add_argument("--without-devices", action="store_true", help="Check silence and missing camera frames")
parser.add_argument("--camera-format", choices=("i420", "nv12", "yuy2", "mjpeg", "h264"), default="i420")
parser.add_argument("--restart", action="store_true", help="Stop and restart both capture devices")
parser.add_argument("--reconnect", action="store_true", help="Reconnect while browser capture stays active")
parser.add_argument("--hotplug", action="store_true", help="Replace the camera while capture stays active")
args = parser.parse_args()
if (args.restart or args.reconnect or args.hotplug) and args.without_devices:
    parser.error("capture lifecycle checks require synthetic devices")
if sum((args.restart, args.reconnect, args.hotplug)) > 1:
    parser.error("--restart, --reconnect, and --hotplug cannot be combined")
if (args.reconnect or args.hotplug) and args.camera_format != "i420":
    parser.error("--reconnect and --hotplug require I420")
SERVER = Path(args.server_bin).resolve(strict=True)
CLIENT = Path(args.client_bin).resolve(strict=True)
ARTIFACTS = Path(args.artifacts).resolve() / ("run-" + uuid.uuid4().hex)
ARTIFACTS.mkdir(parents=True)
PROVIDE_DEVICES = not args.without_devices
results = {}
condition = threading.Condition()
resume = threading.Event()


def wait_results(expected, client, server, deadline):
    with condition:
        while not expected <= results.keys():
            if client.poll() is not None:
                raise RuntimeError(f"client exited before reports: {client.returncode}")
            if server.poll() is not None:
                raise RuntimeError(f"server exited before reports: {server.returncode}")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"media reports incomplete: {results!r}")
            condition.wait(min(remaining, 0.5))


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        pass

    def do_GET(self):
        if self.path == "/webauthn-pending":
            # Answers once the client holds the page's WebAuthn request, so the
            # page's abort has to reach the client as CancelCurOp.
            log = ARTIFACTS / "client.log"
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                if log.exists() and "Synthetic WebAuthn waiting for cancel" in log.read_text(errors="replace"):
                    self.send_response(204)
                    self.end_headers()
                    return
                time.sleep(0.05)
            self.send_error(504)
            return
        if self.path == "/resume":
            if not resume.wait(60):
                self.send_error(504)
                return
            self.send_response(204)
            self.end_headers()
            return
        if self.path not in ("/", "/?restart", "/?reconnect", "/?hotplug"):
            self.send_error(404)
            return
        body = (HERE / "rdp_media.html").read_bytes()
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        if self.path != "/report" or not 0 < length < 65536:
            self.send_error(400)
            return
        entry = json.loads(self.rfile.read(length))
        with condition:
            results[entry["name"]] = entry["value"]
            condition.notify_all()
        self.send_response(204)
        self.end_headers()


http = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
thread = threading.Thread(target=http.serve_forever)
thread.start()
server = client = None
client_log2 = None
report = {"passed": False}
try:
    with (ARTIFACTS / "server.log").open("w") as server_log, (ARTIFACTS / "client.log").open("w") as client_log:
        server = subprocess.Popen(
            [str(SERVER), "--bind-addr", "127.0.0.1:0", "--url",
             f"http://localhost:{http.server_port}/" + ("?restart" if args.restart else
                                                        "?reconnect" if args.reconnect else
                                                        "?hotplug" if args.hotplug else ""),
             "--artifacts", str(ARTIFACTS), "--remote-media", "--webauthn", "--audio", "--location"],
            stdout=server_log, stderr=subprocess.STDOUT,
            **server_process_options(),
        )
        deadline = time.monotonic() + 40
        while not (ARTIFACTS / "server.addr").exists():
            if server.poll() is not None:
                raise RuntimeError(f"server exited before readiness: {server.returncode}")
            if time.monotonic() >= deadline:
                raise TimeoutError("server did not become ready")
            time.sleep(0.1)
        address = (ARTIFACTS / "server.addr").read_text().strip()
        host, port = address.rsplit(":", 1)
        if host != "127.0.0.1":
            raise RuntimeError(f"unexpected server address: {address}")
        certificate = ssl.PEM_cert_to_DER_cert((ARTIFACTS / "server.pem").read_text())
        (ARTIFACTS / "server.der").write_bytes(certificate)
        client = subprocess.Popen([str(CLIENT), port, str(ARTIFACTS / "server.der")],
                                  stdin=subprocess.PIPE, stdout=client_log, stderr=subprocess.STDOUT,
                                  env={**os.environ, "RUST_LOG": "info",
                                       "RDP_MEDIA_FIXTURE_DEVICES": "synthetic" if PROVIDE_DEVICES else "none",
                                       "RDP_MEDIA_FIXTURE_FORMAT": args.camera_format,
                                       "RDP_MEDIA_FIXTURE_RECONNECTED": "0"})
        deadline = time.monotonic() + 60
        expected = {"camera", "microphone", "webauthn", "touch", "pen", "location"}
        if args.hotplug:
            wait_results(expected, client, server, deadline)
            client.stdin.write(b"replace\n")
            client.stdin.flush()
            resume.set()
            expected.add("camera-replaced")
        if args.reconnect:
            wait_results(expected, client, server, deadline)
            client.stdin.close()
            report["first_client_exit"] = client.wait(timeout=10)
            if report["first_client_exit"] != 0:
                raise RuntimeError(f"first client exited with {report['first_client_exit']}")
            client_log2 = (ARTIFACTS / "client-reconnected.log").open("w")
            client = subprocess.Popen([str(CLIENT), port, str(ARTIFACTS / "server.der")],
                                      stdin=subprocess.PIPE, stdout=client_log2, stderr=subprocess.STDOUT,
                                      env={**os.environ, "RUST_LOG": "info",
                                           "RDP_MEDIA_FIXTURE_DEVICES": "synthetic",
                                           "RDP_MEDIA_FIXTURE_FORMAT": "i420",
                                           "RDP_MEDIA_FIXTURE_RECONNECTED": "1"})
            resume.set()
            expected.update({"camera-reconnected", "microphone-reconnected"})
        if args.restart:
            expected.update({"camera-restarted", "microphone-restarted"})
        wait_results(expected, client, server, deadline)
        # oracle: spec MS-RDPEI §2.2.3.3.1.1; mutant dropping touch contacts or omitting CHROME_HEIGHT translation.
        if results["touch"] != {"touched": True, "x": 60, "y": 32}:
            raise AssertionError(f"unexpected RDPEI touch result: {results['touch']!r}")
        # oracle: spec MS-RDPEI §2.2.3.7.1.1; mutant omitting pen contact delivery, pressure scaling, or CHROME_HEIGHT translation.
        if results["pen"] != {"pointerType": "pen", "x": 60, "y": 32, "pressure": 0.5}:
            raise AssertionError(f"unexpected RDPEI pen result: {results['pen']!r}")
        # The client logs what it received asynchronously; the page can report
        # its results (an aborted WebAuthn request, for one) before the client
        # has logged the matching RDP message.
        device_lines = (
            "Synthetic RDPSND playback verified",
            "Synthetic WebAuthn assertion checked",
            "Synthetic WebAuthn cancelled",
        )
        log_deadline = time.monotonic() + 5
        while time.monotonic() < log_deadline:
            initial_log = (ARTIFACTS / "client.log").read_text()
            if (
                "Synthetic pointer bitmap received" in initial_log
                and "Synthetic desktop frame received" in initial_log
                and (not PROVIDE_DEVICES or all(line in initial_log for line in device_lines))
            ):
                break
            time.sleep(0.05)
        if "Synthetic pointer bitmap received" not in initial_log:
            raise AssertionError("client did not receive CEF pointer bitmap over RDP")
        if "Synthetic desktop frame received" not in initial_log:
            raise AssertionError("client did not receive MS-RDPEGFX desktop frame over RDP")
        if PROVIDE_DEVICES:
            location = results["location"]
            # oracle: spec MS-RDPEL §§2.2.2.3-2.2.2.4; mutant failing to reconstruct base + 2D delta coordinates or omitting timezone override.
            if (
                location.get("ok") is not True
                or abs(location.get("latitude", 0) - 45.625) > 1e-4
                or abs(location.get("longitude", 0) - (-73.375)) > 1e-4
                or location.get("altitude") != 42
                or not location.get("timeZone")
            ):
                raise AssertionError(f"unexpected MS-RDPEL location result: {location!r}")
            for suffix in ("", "-restarted") if args.restart else ("",):
                camera = results["camera" + suffix]
                microphone = results["microphone" + suffix]
                # oracle: mutant dropped or corrupted redirected camera frames and microphone samples.
                if not (camera.get("g", 0) > 200 and camera.get("r", 255) < 60 and camera.get("b", 255) < 60):
                    raise AssertionError(f"unexpected camera frame{suffix}: {camera!r}")
                # oracle: mutant interpreting full-range JPEG as limited-range video darkens the green frame.
                if args.camera_format == "mjpeg" and not (camera["g"] >= 245 and camera["r"] <= 10 and camera["b"] <= 10):
                    raise AssertionError(f"incorrect JPEG color{suffix}: {camera!r}")
                if (camera.get("width"), camera.get("height")) != (320, 240):
                    raise AssertionError(f"unexpected camera dimensions{suffix}: {camera!r}")
                if not 0.25 < microphone.get("rms", 0) < 0.45:
                    raise AssertionError(f"unexpected microphone level{suffix}: {microphone!r}")
            webauthn = results["webauthn"]
            # oracle: spec MS-RDPEWA §§2.2.1, 2.2.2.1, and 4.3.1; mutant failing to bridge WebAuthn metadata, AAGUID, or abort over RDPEWA.
            if webauthn != {
                "uvpaa": True,
                "aaguid": "ironrdp-rdpewa!!",
                "assertionOutcome": "NotAllowedError",
                "abortError": "AbortError",
            }:
                raise AssertionError(f"unexpected WebAuthn result: {webauthn!r}")
            log = initial_log
            if "Synthetic WebAuthn assertion checked" not in log:
                raise AssertionError("client did not verify GetAssertion over RDPEWA")
            if "Synthetic WebAuthn cancelled" not in log:
                raise AssertionError("client did not receive RDPEWA CancelCurOp")
            if "Synthetic RDPSND playback verified" not in log:
                raise AssertionError("client did not receive non-silent RDPSND browser audio")
            if args.restart or args.hotplug:
                # oracle: mutant leaving the old client capture open instead of reopening it.
                log = (ARTIFACTS / "client.log").read_text()
                captures = [("Synthetic camera started", "Synthetic camera stopped")]
                if args.restart:
                    captures.append(("Synthetic microphone opened", "Synthetic microphone closed"))
                for opened, closed in captures:
                    first = log.find(opened)
                    stopped = log.find(closed, first + len(opened))
                    second = log.find(opened, stopped + len(closed))
                    if first < 0 or stopped < 0 or second < 0:
                        raise AssertionError(f"capture did not close and reopen: {opened!r}")
            if args.reconnect:
                # oracle: mutant retaining the previous client's frame or audio after reconnection.
                camera = results["camera-reconnected"]
                microphone = results["microphone-reconnected"]
                if not (camera.get("r", 0) > 200 and camera.get("g", 255) < 60 and camera.get("b", 255) < 60):
                    raise AssertionError(f"unexpected reconnected camera frame: {camera!r}")
                if (camera.get("width"), camera.get("height")) != (320, 240):
                    raise AssertionError(f"unexpected reconnected camera dimensions: {camera!r}")
                if microphone.get("stable") is not True or not 0.09 < microphone.get("rms", 0) < 0.22:
                    raise AssertionError(f"unexpected reconnected microphone level: {microphone!r}")
            if args.hotplug:
                # oracle: mutant retaining the old camera after its removal and re-advertisement.
                camera = results["camera-replaced"]
                if not (camera.get("r", 0) > 200 and camera.get("g", 255) < 60 and camera.get("b", 255) < 60):
                    raise AssertionError(f"unexpected replacement camera frame: {camera!r}")
                if (camera.get("width"), camera.get("height")) != (320, 240):
                    raise AssertionError(f"unexpected replacement camera dimensions: {camera!r}")
        else:
            # oracle: mutant host camera frames or microphone samples arrive without redirected devices.
            camera = results["camera"]
            microphone = results["microphone"]
            if camera.get("missingFrame") is not True:
                raise AssertionError(f"unexpected camera frame: {camera!r}")
            if not microphone.get("rms", 1) < 0.001:
                raise AssertionError(f"unexpected microphone level: {microphone!r}")
            webauthn = results["webauthn"]
            # oracle: mutant reaching a host authenticator or hanging when the RDP client does not expose WebAuthN_Channel.
            if "NotAllowedError" not in webauthn.get("error", ""):
                raise AssertionError(f"expected WebAuthn NotAllowedError without client channel: {webauthn!r}")
            location = results["location"]
            # oracle: mutant reaching macOS CoreLocation or host location providers when client omits MS-RDPEL.
            if location.get("ok") is not False or location.get("code") != 2:
                raise AssertionError(f"expected POSITION_UNAVAILABLE (2) without MS-RDPEL: {location!r}")
        report["passed"] = True
except Exception as error:
    report["error"] = f"{type(error).__name__}: {error}"
finally:
    resume.set()
    if client is not None and client.poll() is None:
        client.stdin.close()
        try:
            client.wait(timeout=10)
        except subprocess.TimeoutExpired:
            client.kill()
            client.wait()
            report["passed"] = False
            report["cleanup_error"] = "client did not stop after close request"
    if server is not None and server.poll() is None:
        interrupt_server(server)
        try:
            server.wait(timeout=45)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait()
            report["passed"] = False
            report["cleanup_error"] = "server did not shut down"
    if client_log2 is not None:
        client_log2.close()
    http.shutdown()
    http.server_close()
    thread.join()
    report["results"] = results
    report["server_exit"] = None if server is None else server.returncode
    report["client_exit"] = None if client is None else client.returncode
    if report["server_exit"] != 0 or report["client_exit"] != 0:
        report["passed"] = False
    profile_file = ARTIFACTS / "profile.path"
    if not profile_file.exists():
        report["passed"] = False
        report["cleanup_error"] = "browser profile path not recorded"
    elif Path(profile_file.read_text().strip()).exists():
        report["passed"] = False
        report["cleanup_error"] = "browser profile still exists"
    (ARTIFACTS / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(ARTIFACTS)
    print(json.dumps(report, indent=2))
raise SystemExit(0 if report["passed"] else 1)
