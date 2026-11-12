#!/usr/bin/env python3
"""Exercise the native browser through an isolated ironrdp-agent daemon."""

import argparse
import functools
import hashlib
import http.server
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import signal
import ssl
import struct
import subprocess
import sys
import threading
import time
import uuid
import zlib


CHROME_HEIGHT = 88
TIMEOUT = 30
COLOR_TOLERANCE = 4


def require_background_bundle(server):
    if sys.platform != "darwin":
        return
    plist = server.parent.parent / "Info.plist"
    with plist.open("rb") as source:
        info = plistlib.load(source)
    if info.get("LSUIElement") is not True and info.get("LSBackgroundOnly") is not True:
        raise RuntimeError(
            "Server bundle must set LSUIElement or LSBackgroundOnly: " + str(plist)
        )


def server_process_options():
    return {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP} if sys.platform == "win32" else {}


def interrupt_server(process):
    process.send_signal(signal.CTRL_BREAK_EVENT if sys.platform == "win32" else signal.SIGINT)


def decode_png(path):
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("Not a PNG: {}".format(path))
    offset, compressed, header, ended = 8, bytearray(), None, False
    while offset < len(data):
        if offset + 12 > len(data):
            raise ValueError("Truncated PNG chunk")
        length = struct.unpack_from(">I", data, offset)[0]
        kind = data[offset + 4:offset + 8]
        payload = data[offset + 8:offset + 8 + length]
        if offset + 12 + length > len(data):
            raise ValueError("Truncated PNG payload")
        crc = struct.unpack_from(">I", data, offset + 8 + length)[0]
        if zlib.crc32(kind + payload) & 0xffffffff != crc:
            raise ValueError("PNG CRC mismatch")
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", payload)
        elif kind == b"IDAT":
            compressed.extend(payload)
        elif kind == b"IEND":
            ended = True
            break
        offset += length + 12
    if header is None or not ended:
        raise ValueError("Missing PNG header or end")
    width, height, depth, color, compression, filtering, interlace = header
    if depth != 8 or color not in (2, 6) or (compression, filtering, interlace) != (0, 0, 0):
        raise ValueError("Expected noninterlaced 8-bit RGB/RGBA PNG")
    if not (0 < width <= 8192 and 0 < height <= 8192):
        raise ValueError("Invalid screenshot dimensions")
    channels = 3 if color == 2 else 4
    stride = width * channels
    expected = (stride + 1) * height
    inflater = zlib.decompressobj()
    raw = inflater.decompress(bytes(compressed), expected + 1)
    if len(raw) != expected or not inflater.eof or inflater.unused_data:
        raise ValueError("Incorrect PNG image data length")
    pixels, previous = bytearray(), bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        filter_kind = raw[start]
        row = bytearray(raw[start + 1:start + 1 + stride])
        if filter_kind > 4:
            raise ValueError("Unknown PNG filter")
        for x in range(stride):
            left = row[x - channels] if x >= channels else 0
            above = previous[x]
            upper_left = previous[x - channels] if x >= channels else 0
            if filter_kind == 1:
                predictor = left
            elif filter_kind == 2:
                predictor = above
            elif filter_kind == 3:
                predictor = (left + above) // 2
            elif filter_kind == 4:
                estimate = left + above - upper_left
                distances = (abs(estimate - left), abs(estimate - above), abs(estimate - upper_left))
                predictor = (left, above, upper_left)[distances.index(min(distances))]
            else:
                predictor = 0
            row[x] = (row[x] + predictor) & 255
        pixels.extend(row)
        previous = row
    return width, height, channels, pixels


def pixel(image, x, y):
    width, height, channels, pixels = image
    if not (0 <= x < width and 0 <= y < height):
        raise AssertionError("Pixel outside image: {},{}".format(x, y))
    offset = (y * width + x) * channels
    return tuple(pixels[offset:offset + 3])


def rgb(value):
    return tuple(bytes.fromhex(value))


def color_matches(actual, expected):
    return all(abs(channel - target) <= COLOR_TOLERANCE
               for channel, target in zip(actual, expected))


class FixtureHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, fmt, *args):
        pass

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


class Runner:
    fixture_handler = FixtureHandler

    def __init__(self, args, *, require_bundle=True):
        self.server_bin = Path(args.server_bin).resolve()
        if require_bundle:
            require_background_bundle(self.server_bin)
        self.agent_bin = Path(args.agent).resolve()
        root = Path(args.artifacts).resolve()
        root.mkdir(parents=True, exist_ok=True)
        self.artifacts = root / ("run-" + uuid.uuid4().hex)
        self.artifacts.mkdir()
        # Relative paths also keep the daemon's bootstrap socket below macOS SUN_LEN.
        self.ipc = Path(".cef-" + uuid.uuid4().hex[:12])
        self.ipc.mkdir(mode=0o700)
        self.socket = (r"\\.\pipe\ironrdp-cef-" + uuid.uuid4().hex
                       if sys.platform == "win32" else self.ipc / "agent.sock")
        self.env = os.environ.copy()
        self.env["IRONRDP_AGENT_KNOWN_CERTIFICATES"] = str(self.artifacts / "known-certificates")
        self.env["XDG_STATE_HOME"] = str(self.ipc / "state")
        self.http = None
        self.http_thread = None
        self.server = None
        self.server_log = None
        self.profile = None
        self.daemon_started = False
        self.size = (1280, 800)
        self.steps = []
        self.commands = (self.artifacts / "agent.log").open("w", encoding="utf-8")
        self.report = {"passed": False, "steps": self.steps, "artifacts": str(self.artifacts),
                       "ipc": str(self.socket), "color_tolerance_per_channel": COLOR_TOLERANCE}

    def agent_result(self, *args, timeout=TIMEOUT):
        command = [str(self.agent_bin), "--endpoint", str(self.socket)] + list(args)
        self.commands.write(json.dumps(command, ensure_ascii=False) + "\n")
        self.commands.flush()
        result = subprocess.run(command, env=self.env, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, text=True, timeout=timeout)
        self.commands.write(result.stdout + "\n")
        self.commands.flush()
        return result

    def agent(self, *args, timeout=TIMEOUT):
        result = self.agent_result(*args, timeout=timeout)
        if result.returncode:
            raise RuntimeError("Agent {} failed: {}".format(args, result.stdout))
        return result.stdout

    def key(self, scancode):
        self.agent("key-scancode", "--scancode", hex(scancode), "--pressed", "true")
        self.agent("key-scancode", "--scancode", hex(scancode), "--pressed", "false")

    def shortcut(self, scancode):
        self.agent("key-scancode", "--scancode", "0x1d", "--pressed", "true")
        try:
            self.key(scancode)
        finally:
            self.agent("key-scancode", "--scancode", "0x1d", "--pressed", "false")

    def click(self, x, y):
        self.agent("mouse-move", "--x", str(x), "--y", str(y))
        self.agent("mouse-button", "--button", "left", "--pressed", "true")
        self.agent("mouse-button", "--button", "left", "--pressed", "false")

    def capture(self, name):
        self.agent("mouse-move", "--x", "900", "--y", "90")
        path = self.artifacts / (name + ".png")
        self.agent("screenshot", str(path))
        return path, decode_png(path)

    def expect(self, name, markers, timeout=TIMEOUT):
        deadline = time.monotonic() + timeout
        last_error = None
        while time.monotonic() < deadline:
            if self.server.poll() is not None:
                raise RuntimeError("Server exited during " + name)
            status = self.agent("status")
            if "state: Connected" not in status.splitlines():
                raise RuntimeError("Unexpected disconnect during {}: {}".format(name, status))
            try:
                path, image = self.capture(name)
                if image[:2] != self.size:
                    raise AssertionError("Dimensions {} != {}".format(image[:2], self.size))
                for x, y, color in markers:
                    actual = pixel(image, x, y)
                    if not color_matches(actual, rgb(color)):
                        raise AssertionError("Pixel {},{}: {} differs from #{} by more than {}".format(
                            x, y, actual, color, COLOR_TOLERANCE))
                self.steps.append({"name": name, "screenshot": path.name, "size": list(image[:2]),
                                   "markers": markers})
                return image
            except (AssertionError, RuntimeError) as error:
                last_error = error
                time.sleep(0.15)
        raise AssertionError("{} timed out: {}".format(name, last_error))

    def content(self, name, markers, chrome=()):
        return self.expect(name, list(chrome) +
                           [(x, y + CHROME_HEIGHT, color) for x, y, color in markers])

    def scroll_to(self, name, delta, markers):
        last_error = None
        deadline = time.monotonic() + TIMEOUT
        for _ in range(12):
            for _ in range(3):
                self.agent("wheel", "--delta", str(delta))
            try:
                return self.expect(name, markers, timeout=1)
            except AssertionError as error:
                last_error = error
            if time.monotonic() >= deadline:
                break
        raise AssertionError("{} did not reach its target: {}".format(name, last_error))

    def navigate(self, page):
        self.shortcut(0x26)
        self.agent("type-unicode", "--text", self.base_url)
        scans = {"i": 0x17, "n": 0x31, "d": 0x20, "e": 0x12, "x": 0x2d,
                 ".": 0x34, "h": 0x23, "t": 0x14, "m": 0x32, "l": 0x26,
                 "s": 0x1f, "c": 0x2e, "o": 0x18}
        for character in page:
            self.key(scans[character])
        self.key(0x1c)

    def start_server(self, generation, extra_args=()):
        directory = self.artifacts / generation
        directory.mkdir()
        self.server_log = (directory / "server.log").open("w", encoding="utf-8")
        self.server = subprocess.Popen(
            [str(self.server_bin), "--bind-addr", "127.0.0.1:0", "--url",
             self.base_url + "index.html", "--artifacts", str(directory),
             "--width", "1280", "--height", "800"] + list(extra_args),
            env=self.env, stdout=self.server_log, stderr=subprocess.STDOUT, **server_process_options())
        record = {"artifacts": generation}
        self.report.setdefault("servers", []).append(record)
        deadline = time.monotonic() + TIMEOUT
        cert, address = directory / "server.pem", directory / "server.addr"
        profile_path = directory / "profile.path"
        while not (cert.is_file() and address.is_file() and address.stat().st_size
                   and profile_path.is_file() and profile_path.stat().st_size):
            if self.server.poll() is not None:
                raise RuntimeError("Server startup failed; see " + str(directory / "server.log"))
            if time.monotonic() >= deadline:
                raise TimeoutError("Server readiness artifacts missing")
            time.sleep(0.1)
        endpoint = address.read_text().strip()
        if not re.fullmatch(r"127\.0\.0\.1:[1-9][0-9]{0,4}", endpoint):
            raise ValueError("Unexpected server endpoint: " + endpoint)
        der = ssl.PEM_cert_to_DER_cert(cert.read_text())
        fingerprint = hashlib.sha256(der).hexdigest()
        self.profile = Path(profile_path.read_text().strip())
        record.update({"endpoint": endpoint, "sha256": fingerprint, "profile": str(self.profile)})
        if not self.profile.is_dir():
            raise RuntimeError("Published browser profile directory does not exist")
        self.agent("cert", "trust", endpoint, fingerprint)
        return endpoint

    def connect(self, endpoint, extra_args=()):
        self.agent("connect", "--no-prompt", "--no-auto-start", "--server", endpoint,
                   "--username", "browser", "--password", "test",
                   "--prop", "enablecredsspsupport:i:0", "--prop", "desktopwidth:i:1280",
                   "--prop", "desktopheight:i:800", *extra_args)
        deadline = time.monotonic() + TIMEOUT
        while time.monotonic() < deadline:
            status = self.agent("status")
            if "state: Connected" in status.splitlines():
                return
            if "state: Failed" in status.splitlines():
                raise RuntimeError("RDP connect failed: " + status)
            if self.server.poll() is not None:
                raise RuntimeError("Server exited while connecting")
            time.sleep(0.1)
        raise TimeoutError("RDP connect timed out: " + status)

    def stop_server(self):
        errors = []
        interrupted = False
        if self.server is not None:
            started = time.monotonic()
            forced = False
            if self.server.poll() is None:
                interrupt_server(self.server)
                interrupted = True
                try:
                    self.server.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    forced = True
                    self.server.kill()
                    self.server.wait(timeout=5)
            return_code = self.server.returncode
            duration = time.monotonic() - started
            self.report["servers"][-1].update({
                "return_code": return_code, "shutdown_duration_seconds": duration,
                "forced_termination": forced})
            if return_code != 0:
                errors.append("Server exited with return code {} after {:.3f}s{}".format(
                    return_code, duration, " (forced termination)" if forced else ""))
            self.server = None
        if self.server_log is not None:
            self.server_log.close()
            self.server_log = None
        if self.profile is not None:
            removed = not self.profile.exists()
            self.report["servers"][-1]["profile_removed"] = removed
            self.profile = None
            if not removed:
                errors.append("Server browser profile was not removed")
        if errors:
            raise RuntimeError("; ".join(errors))
        return interrupted

    def start_fixture(self):
        handler = functools.partial(self.fixture_handler, directory=str(Path(__file__).parent))
        self.http = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self.http_thread = threading.Thread(target=self.http.serve_forever, daemon=True)
        self.http_thread.start()
        self.base_url = "http://127.0.0.1:{}/".format(self.http.server_port)
        self.report["fixture_url"] = self.base_url

    def run(self):
        self.start_fixture()
        self.daemon_started = True
        self.agent("daemon", "start")
        endpoint = self.start_server("first")
        self.connect(endpoint)
        # oracle: spec Indexed Database API 3.0 §5.1, §5.2, and §5.4.
        self.content("01-initial", [(48, 48, "ff0000"), (136, 48, "0000ff"),
                                    (224, 48, "00ff00"), (256, 296, "00ffff"), (824, 48, "0088ff")],
                     chrome=[(2, 3, "f8f9fa")])
        self.click(100, CHROME_HEIGHT + 136)
        self.content("02-click", [(256, 136, "ffff00")])
        self.click(100, CHROME_HEIGHT + 212)
        # oracle: spec UI Events KeyboardEvent code Values §code-KeyA.
        self.key(0x1e)
        self.agent("key-scancode", "--scancode", "0x2a", "--pressed", "true")
        try:
            self.key(0x1e)
        finally:
            self.agent("key-scancode", "--scancode", "0x2a", "--pressed", "false")
        self.content("02-physical-keys", [(640, 216, "00ff88")])
        self.key(0x0e)
        self.key(0x0e)
        # oracle: spec HTML §the input element (text entry updates its value).
        self.agent("key-scancode", "--scancode", "0x30", "--pressed", "true")
        try:
            self.content("02-first-keydown", [(728, 216, "ff0000")])
            self.agent("key-scancode", "--scancode", "0x30", "--pressed", "true")
            self.content("02-repeated-text", [(728, 216, "00ff88")])
        finally:
            self.agent("key-scancode", "--scancode", "0x30", "--pressed", "false")
        self.key(0x0e)
        self.key(0x0e)
        self.agent("type-unicode", "--text", "IronRDP café 漢字!")
        self.content("03-unicode", [(376, 216, "ff00ff"), (552, 216, "00ff88")])
        self.key(0x0e)
        self.content("04-backspace", [(464, 216, "00aaff")])
        for label, scancode in (("escape", 0x01), ("tab", 0x0f)):
            self.shortcut(0x26)
            self.content("04-address-focus-" + label, [(552, 216, "333333")])
            self.key(scancode)
            self.content("04-content-focus-" + label, [(552, 216, "00ff88")])
        self.click(100, CHROME_HEIGHT + 296)
        self.content("05-storage-saved", [(256, 296, "8844ff")])
        self.click(150, CHROME_HEIGHT + 388)
        self.expect_popup("06-popup-open", True)
        self.key(0x01)
        self.expect_popup("07-popup-dismissed", False)
        self.click(150, CHROME_HEIGHT + 388)
        self.key(0x50)
        self.key(0x1c)
        self.content("08-popup-selected", [(376, 392, "ee00ee")])
        self.agent("mouse-move", "--x", "600", "--y", "600")
        self.scroll_to("09-scroll", -120, [(48, self.size[1] - 64, "ff8800")])
        self.scroll_to("09-scroll-restored", 120, [(48, CHROME_HEIGHT + 48, "ff0000")])
        self.navigate("second.html")
        self.content("10-navigate", [(48, 48, "ffaa00")])
        self.click(16, 60)
        self.content("11-back", [(48, 48, "ff0000"), (256, 296, "8844ff")])
        self.click(48, 60)
        self.content("12-forward", [(48, 48, "ffaa00")])
        self.click(80, 60)
        self.content("13-reload", [(168, 48, "ff4488")])
        self.shortcut(0x14)
        self.navigate("index.html")
        self.content("14-new-tab", [(48, 48, "ff0000"), (256, 296, "8844ff")],
                     chrome=[(2, 3, "c4c6c8"), (182, 3, "f8f9fa")])
        self.click(40, 16)
        self.content("15-select-first-tab", [(48, 48, "ffaa00")],
                     chrome=[(2, 3, "f8f9fa"), (182, 3, "c4c6c8")])
        self.click(220, 16)
        self.content("16-select-second-tab", [(48, 48, "ff0000")])
        self.shortcut(0x11)
        self.content("17-close-tab", [(48, 48, "ffaa00")])
        self.click(196, 16)
        self.navigate("index.html")
        self.content("17-mouse-new-tab", [(48, 48, "ff0000")])
        self.click(348, 16)
        self.content("17-mouse-close-tab", [(48, 48, "ffaa00")])
        self.agent("resize", "--width", "1024", "--height", "768")
        self.size = (1024, 768)
        self.expect("18-resize", [(48, 136, "ffaa00"), (1000, 744, "123456")])
        self.agent("disconnect")
        self.size = (1280, 800)
        self.connect(endpoint)
        self.content("19-reconnect", [(48, 48, "ffaa00")])
        self.agent("disconnect")
        self.stop_server()
        endpoint = self.start_server("second")
        self.size = (1280, 800)
        self.connect(endpoint)
        self.content("20-fresh-profile", [(48, 48, "ff0000"), (256, 296, "00ffff")])

    def expect_popup(self, name, visible):
        deadline = time.monotonic() + TIMEOUT
        while time.monotonic() < deadline:
            image = self.content(name, [(48, 48, "ff0000")])
            count = sum(color_matches(pixel(image, x, y), rgb("ee00ee"))
                        for y in range(CHROME_HEIGHT + 408, CHROME_HEIGHT + 510, 2)
                        for x in range(32, 300, 2))
            if (count >= 20) == visible:
                self.steps[-1]["popup_literal_pixels"] = count
                return image
            self.steps.pop()
            time.sleep(0.15)
        raise AssertionError("Popup {} literal-color visibility != {}".format(name, visible))

    def cleanup(self):
        errors = []
        if self.daemon_started:
            try:
                self.agent("daemon", "stop", timeout=15)
            except Exception as error:
                errors.append(str(error))
        try:
            self.stop_server()
        except (Exception, KeyboardInterrupt) as error:
            errors.append(str(error))
        if self.http is not None:
            self.http.shutdown()
            self.http.server_close()
            self.http_thread.join(timeout=5)
        if sys.platform != "win32" and self.socket.exists():
            self.socket.unlink()
        try:
            if (self.ipc / "state").exists():
                shutil.copytree(str(self.ipc / "state"), str(self.artifacts / "state"))
            shutil.rmtree(str(self.ipc))
        except Exception as error:
            errors.append(str(error))
        self.commands.close()
        self.report["cleanup_errors"] = errors
        return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-bin", required=True)
    parser.add_argument("--agent", required=True)
    parser.add_argument("--artifacts", "--output", default="server/cef/artifacts/e2e")
    args = parser.parse_args()
    runner = Runner(args)
    print("Artifacts: " + str(runner.artifacts), flush=True)
    failed = False
    try:
        runner.run()
        runner.report["passed"] = True
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
