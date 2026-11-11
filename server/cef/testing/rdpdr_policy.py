#!/usr/bin/env python3
"""Exercise the agent's download-only RDPDR policy over a real RDP connection."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import ssl
import subprocess
import sys
import time

import e2e


PAYLOAD = b"RDPDR wire policy\n"
EXPECTED_CHECKS = {
    "existing-read", "existing-overwrite", "existing-open-write", "create-traversal", "create-link", "create-directory",
    "create-temporary", "create-information", "read-denied", "listing-denied", "device-control-denied",
    "replace-denied", "collision-denied", "rename-link", "rename-traversal", "unowned-handle-denied",
    "unowned-close-denied",
    "write-temporary", "flush-temporary", "publish", "published-write-denied", "published-delete-denied", "close",
}


class PolicyRunner(e2e.Runner):
    def __init__(self, args):
        super().__init__(args, require_bundle=False)
        self.client_directory = self.artifacts / "client-downloads"
        self.client_directory.mkdir(mode=0o700)

    def agent(self, *args, **kwargs):
        if args == ("daemon", "start"):
            args += ("--rdpdr-download-dir", str(self.client_directory))
        return super().agent(*args, **kwargs)

    def run(self):
        existing = self.client_directory / "existing.bin"
        outside = self.artifacts / "outside.bin"
        link = self.client_directory / "link.bin"
        existing.write_bytes(b"existing client file")
        outside.write_bytes(b"outside client folder")
        if sys.platform == "win32":
            os.link(outside, link)
        else:
            link.symlink_to(outside)

        self.daemon_started = True
        self.agent("daemon", "start")
        selected = self.client_directory
        self.client_directory = self.artifacts / "approved-downloads"
        selected.rename(self.client_directory)
        if sys.platform == "win32":
            replacement = selected
            replacement.mkdir(mode=0o700)
        else:
            replacement = self.artifacts / "replacement-downloads"
            replacement.mkdir(mode=0o700)
            selected.symlink_to(replacement, target_is_directory=True)
        existing = self.client_directory / "existing.bin"
        link = self.client_directory / "link.bin"
        directory = self.artifacts / "policy-server"
        directory.mkdir()
        self.server_log = (directory / "server.log").open("w", encoding="utf-8")
        self.server = subprocess.Popen(
            [str(self.server_bin), "--artifacts", str(directory)],
            stdout=self.server_log, stderr=subprocess.STDOUT, env=self.env,
        )
        self.report["servers"] = [{"artifacts": "policy-server"}]
        cert, address = directory / "server.pem", directory / "server.addr"
        deadline = time.monotonic() + e2e.TIMEOUT
        while not (cert.is_file() and address.is_file() and address.stat().st_size):
            if self.server.poll() is not None:
                raise RuntimeError("Policy server exited during startup")
            if time.monotonic() >= deadline:
                raise TimeoutError("Policy server did not become ready")
            time.sleep(0.05)
        endpoint = address.read_text(encoding="utf-8").strip()
        if not re.fullmatch(r"127\.0\.0\.1:[1-9][0-9]{0,4}", endpoint):
            raise ValueError("Unexpected policy server endpoint: " + endpoint)
        fingerprint = hashlib.sha256(ssl.PEM_cert_to_DER_cert(cert.read_text(encoding="utf-8"))).hexdigest()
        self.agent("cert", "trust", endpoint, fingerprint)
        self.connect(endpoint)
        (directory / "go").touch()
        self.server.wait(timeout=e2e.TIMEOUT * 2)
        if self.server.returncode != 0:
            raise RuntimeError("Policy server failed; see " + str(directory / "server.log"))
        passed = {line.removeprefix("PASS ") for line in (directory / "server.log").read_text().splitlines()
                  if line.startswith("PASS ")}
        if passed != EXPECTED_CHECKS:
            raise AssertionError("Unexpected policy checks: {}".format(sorted(passed)))
        # oracle: mutant unauthorized RDPDR requests modify a pre-existing, linked, or outside file.
        if existing.read_bytes() != b"existing client file" or outside.read_bytes() != b"outside client folder":
            raise AssertionError("Policy requests changed a protected file")
        if sys.platform == "win32":
            if not os.path.samefile(link, outside):
                raise AssertionError("Policy requests changed the existing hard link")
        elif not link.is_symlink() or link.resolve() != outside:
            raise AssertionError("Policy requests changed the existing symbolic link")
        if (self.client_directory / "delivered.bin").read_bytes() != PAYLOAD:
            raise AssertionError("Published client bytes differ from the wire payload")
        if {path.name for path in self.client_directory.iterdir()} != {"existing.bin", "link.bin", "delivered.bin"}:
            raise AssertionError("Unexpected files remain in the client directory")
        # oracle: mutant re-resolving the approved path after daemon startup.
        if any(replacement.iterdir()):
            raise AssertionError("RDPDR requests reached the substituted download directory")
        self.report["checks"] = sorted(passed)
        self.report["client_checks"] = ["approved-directory-remains-pinned"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-bin", required=True, help="rdpdr-policy-fixture executable")
    parser.add_argument("--agent", required=True)
    parser.add_argument("--artifacts", default="server/cef/artifacts/rdpdr-policy-e2e")
    args = parser.parse_args()
    runner = PolicyRunner(args)
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
            json.dumps(runner.report, indent=2) + "\n", encoding="utf-8")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
