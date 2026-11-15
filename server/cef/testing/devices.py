#!/usr/bin/env python3
"""Verify that pages reach only simulated client devices through the patched CEF runtime."""

import argparse
import json
import sys
import threading
import time

import e2e
from e2e import CHROME_HEIGHT, TIMEOUT

RESULTS = {}
RESULTS_CHANGED = threading.Condition()


class DevicesHandler(e2e.FixtureHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        if self.path != "/report" or not 0 < length <= 65536:
            self.send_error(400)
            return
        entry = json.loads(self.rfile.read(length))
        with RESULTS_CHANGED:
            RESULTS[entry["name"]] = entry["value"]
            RESULTS_CHANGED.notify_all()
        self.send_response(204)
        self.end_headers()


def wait_result(name):
    deadline = time.monotonic() + TIMEOUT
    with RESULTS_CHANGED:
        while name not in RESULTS:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("page never reported " + name)
            RESULTS_CHANGED.wait(remaining)
        return RESULTS[name]


class DevicesRunner(e2e.Runner):
    fixture_handler = DevicesHandler

    def __init__(self, args):
        super().__init__(args)
        self.permission_only = args.permission_only

    def start_fixture(self):
        super().start_fixture()
        # Web Authentication rejects IP-address relying parties, and localhost stays a secure context.
        self.base_url = "http://localhost:{}/".format(self.http.server_port)
        self.report["fixture_url"] = self.base_url

    def check(self, name, condition):
        value = wait_result(name)
        self.steps.append({"name": name, "result": value})
        if not condition(value):
            raise AssertionError("{} reported {!r}".format(name, value))

    def run(self):
        self.start_fixture()
        self.daemon_started = True
        self.agent("daemon", "start")
        if not self.permission_only:
            self.check_simulated_devices()

        # oracle: mutant missing default-deny or opt-in permission gate.
        denied = self.probe_permission("media-disabled", ())
        if not denied.get("error", "").startswith("NotAllowedError:"):
            raise AssertionError("media-disabled reported {!r}".format(denied))
        allowed = self.probe_permission("media-enabled", ("--remote-media",))
        if allowed.get("granted") is not True:
            raise AssertionError("media-enabled reported {!r}".format(allowed))

    def probe_permission(self, generation, options):
        if self.server:
            self.agent("disconnect")
            self.stop_server()
        self.connect(self.start_server(generation, options))
        self.shortcut(0x26)
        self.agent("type-unicode", "--text", self.base_url + "devices.html")
        self.key(0x1c)
        self.content(generation + "-fixture", [(48, 300, "ffffff")])
        with RESULTS_CHANGED:
            RESULTS.pop("permission", None)
        self.click(100, CHROME_HEIGHT + 112)
        value = wait_result("permission")
        self.steps.append({"name": generation, "result": value})
        return value

    def check_simulated_devices(self):
        self.connect(self.start_server("devices", ("--clipboard", "--simulate-devices")))
        self.shortcut(0x26)
        self.agent("type-unicode", "--text", self.base_url + "devices.html")
        self.key(0x1c)
        self.content("01-fixture", [(48, 300, "ffffff")])

        self.click(100, CHROME_HEIGHT + 48)
        # oracle: WebAuthn L3 §5.1.3 (create() returns an attested credential); the simulator registers ES256.
        self.check("webauthn", lambda v: v.get("type") == "public-key" and v.get("rawIdLength") == 16
                   and v.get("publicKeyAlgorithm") == -7 and v.get("platform") is True)
        # oracle: mutant dropped or corrupted the simulator's green camera frame.
        self.check("camera", lambda v: "error" not in v and v["g"] >= 245 and v["r"] <= 10 and v["b"] <= 10)
        self.capture("02-camera")
        # oracle: a full-scale/2 sine has RMS 0.354; silence or a missing device stays near zero.
        self.check("microphone", lambda v: "error" not in v and 0.25 < v["rms"] < 0.45)
        # oracle: the media policy denies desktop capture, so the host display stays private.
        self.check("screen", lambda v: v.get("error", "").startswith("NotAllowedError:"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-bin", required=True,
                        help="Server bundle built against patched CEF; simulator required unless --permission-only")
    parser.add_argument("--agent", required=True)
    parser.add_argument("--artifacts", default="server/cef/artifacts/devices-e2e")
    parser.add_argument("--permission-only", action="store_true",
                        help="Check media permission with a bundle built without the simulator")
    args = parser.parse_args()
    runner = DevicesRunner(args)
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
