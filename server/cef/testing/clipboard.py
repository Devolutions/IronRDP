#!/usr/bin/env python3
"""Verify session-private clipboard redirection between the browser and an RDP client."""

import argparse
import json
from pathlib import Path
import struct
import subprocess
import sys
import time
import zlib

import e2e
from e2e import CHROME_HEIGHT, TIMEOUT

CLIENT_TEXT = "client→browser ✓"
CLIENT_HTML = "<i>client html</i>"


def encode_png(path, width, height, color):
    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF))

    rows = b"".join(b"\x00" + bytes(color) * width for _ in range(height))
    path.write_bytes(b"\x89PNG\r\n\x1a\n"
                     + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                     + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def host_pasteboard():
    """Return the macOS general pasteboard change count, or None elsewhere."""
    if sys.platform != "darwin":
        return None
    script = 'ObjC.import("AppKit"); JSON.stringify($.NSPasteboard.generalPasteboard.changeCount)'
    result = subprocess.run(["osascript", "-l", "JavaScript", "-e", script],
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=True)
    return json.loads(result.stdout)


class ClipboardRunner(e2e.Runner):
    def __init__(self, args):
        super().__init__(args)
        self.stock_server_bin = Path(args.stock_server_bin).resolve() if args.stock_server_bin else None
        self.host_before = None

    def start_server(self, generation, extra_args=("--clipboard",)):
        return super().start_server(generation, extra_args)

    def open_fixture(self):
        self.shortcut(0x26)
        self.agent("type-unicode", "--text", self.base_url + "clipboard.html")
        self.key(0x1c)
        self.content("01-fixture", [(48, 300, "ffffff"), (376, 48, "333333")])

    def paste_until(self, name, marker):
        """Paste into the page until it reports the expected clipboard content.

        The client's format list reaches the browser clipboard asynchronously, so an early paste may see
        the previous item.
        """
        deadline = time.monotonic() + TIMEOUT
        while time.monotonic() < deadline:
            self.click(100, CHROME_HEIGHT + 44)
            self.shortcut(0x2f)
            try:
                x, y, color = marker
                return self.expect(name, [(x, y + CHROME_HEIGHT, color)], timeout=2)
            except AssertionError:
                self.steps[:] = [step for step in self.steps if step["name"] != name]
        raise AssertionError(name + " never pasted the client item")

    def copy_until(self, name, button, read):
        """Copy from the page, then poll the client until `read` reports the browser's item."""
        self.agent("clipboard-set", "--text", "stale")
        self.click(*button)
        self.content(name + "-copied", [(680, 224, "0088ff")])
        deadline = time.monotonic() + TIMEOUT
        last = None
        while time.monotonic() < deadline:
            last = read()
            if last is True:
                self.steps.append({"name": name})
                return
            time.sleep(0.2)
        raise AssertionError("{} did not reach the client: {!r}".format(name, last))

    def check_host_untouched(self, name):
        after = host_pasteboard()
        if after is None:
            return
        if after != self.host_before:
            raise AssertionError("{}: host pasteboard changed from {!r} to {!r}".format(
                name, self.host_before, after))
        self.steps.append({"name": name, "host_pasteboard": after})

    def stock_rejects_host_clipboard(self):
        cases = [
            ("00-stock-rejects-without-redirection", []),
            ("01-stock-rejects-with-redirection", ["--clipboard"]),
        ]
        for name, options in cases:
            directory = self.artifacts / name
            directory.mkdir()
            log_path = directory / "server.log"
            with log_path.open("w", encoding="utf-8") as log:
                result = subprocess.run(
                    [str(self.stock_server_bin), "--bind-addr", "127.0.0.1:0",
                     "--url", self.base_url + "clipboard.html", "--artifacts", str(directory), *options],
                    env=self.env, stdout=log, stderr=subprocess.STDOUT, timeout=TIMEOUT)
            output = log_path.read_text(encoding="utf-8")
            if result.returncode == 0 or "browser startup requires the private-clipboard CEF runtime" not in output:
                raise AssertionError("{}: stock CEF was not rejected for missing isolation".format(name))
            self.steps.append({"name": name, "exit": result.returncode})

    def run(self):
        self.host_before = host_pasteboard()
        self.start_fixture()
        if self.stock_server_bin:
            self.stock_rejects_host_clipboard()
        self.daemon_started = True
        self.agent("daemon", "start")
        self.connect(self.start_server("clipboard"))
        self.open_fixture()

        # oracle: spec Clipboard API and events §8.1.3 (paste event exposes clipboard data by MIME type).
        self.agent("clipboard-set", "--text", CLIENT_TEXT)
        self.paste_until("02-client-text", (376, 48, "00ff88"))
        self.agent("clipboard-set-html", "--html", CLIENT_HTML)
        self.paste_until("03-client-html", (464, 48, "ff00ff"))
        image = self.artifacts / "client.png"
        encode_png(image, 4, 3, (0x12, 0x34, 0x56))
        self.agent("clipboard-set-image", str(image))
        self.paste_until("04-client-image", (552, 48, "ffaa00"))
        self.check_host_untouched("05-host-after-paste")

        # oracle: spec Clipboard API and events §8.1.1 (copy of a text selection).
        self.copy_until("06-browser-text", (100, CHROME_HEIGHT + 216),
                        lambda: self.agent("clipboard-get").strip() == "browser→client ✓" or None)

        # oracle: spec Clipboard API and events §8.1.1 (copy handler data replaces the selection).
        self.copy_until("07-browser-html", (300, CHROME_HEIGHT + 216),
                        lambda: "<b>rich copy</b>" in self.agent("clipboard-get-html") or None)

        # oracle: spec Clipboard API §7.2.3 (write of an image/png ClipboardItem).
        copied = self.artifacts / "browser.png"

        def read_image():
            copied.unlink(missing_ok=True)
            self.agent("clipboard-get-image", str(copied))
            if not copied.is_file():
                return None
            decoded = e2e.decode_png(copied)
            if decoded[:2] == (5, 2) and e2e.color_matches(e2e.pixel(decoded, 0, 0), (0x65, 0x43, 0x21)):
                return True
            return decoded[:2]

        self.copy_until("08-browser-image", (528, CHROME_HEIGHT + 216), read_image)
        self.check_host_untouched("09-host-after-copy")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-bin", required=True, help="Server bundle built against the isolated-clipboard CEF")
    parser.add_argument("--stock-server-bin", help="Server bundle built against stock CEF, expected to reject startup")
    parser.add_argument("--agent", required=True)
    parser.add_argument("--artifacts", default="server/cef/artifacts/clipboard-e2e")
    args = parser.parse_args()
    runner = ClipboardRunner(args)
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
