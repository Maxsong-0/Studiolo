#!/usr/bin/env python3
"""Actual Demo CLI process contract; argument: built studiolo demo example."""
import hashlib
import http.client
import json
from pathlib import Path
import queue
import socket
import stat
import subprocess
import sys
import tempfile
import threading
import unittest


BINARY = str(Path(sys.argv.pop(1)).resolve())


def digest_tree(root):
    return {
        str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in root.rglob("*") if p.is_file()
    }


class DemoInit(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="studiolo-demo-contract-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve() / "new demo's data"

    def invoke(self, *args):
        return subprocess.run(
            [BINARY, *map(str, args)], text=True, capture_output=True, timeout=30
        )

    def verify_candidates(self, record):
        self.assertEqual(record["mode"], "demo")
        self.assertEqual(record["status"], "needs_review")
        self.assertEqual(record["root"], str(self.root))
        self.assertGreaterEqual(record["audit_events"], 10)
        self.assertEqual(record["viewer_argv"][2], str(self.root / "vault"))
        self.assertEqual(record["viewer_argv"][6], "当前为演示源")
        self.assertEqual(stat.S_IMODE(self.root.stat().st_mode), 0o700)
        vault = self.root / "vault"
        manifest_path = (vault / record["manifest"]).resolve()
        self.assertTrue(manifest_path.is_relative_to(vault))
        manifest = json.loads(manifest_path.read_text())
        self.assertEqual(manifest["task_id"], record["task_id"])
        for artifact in manifest["artifacts"]:
            path = (vault / artifact["path"]).resolve()
            self.assertTrue(path.is_relative_to(vault))
            self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), artifact["sha256"])
        candidate = json.loads((manifest_path.parent / "gateway-demo.json").read_text())
        results = {r["tool"]: r["output"] for r in candidate["results"]}
        self.assertEqual(results["web.fetch"]["value"], 7)
        self.assertEqual(results["db.query"]["rows"][0]["value"], 7)

    def test_create_then_refuse_existing_without_modification(self):
        result = self.invoke(self.root)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("当前为演示源", result.stdout)
        record = next(json.loads(line) for line in result.stdout.splitlines() if line.startswith("{"))
        self.verify_candidates(record)
        self.assertFalse(record["web_running_until_input"])
        before = digest_tree(self.root)
        rejected = self.invoke(self.root)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertEqual(digest_tree(self.root), before)

    def test_reject_empty_existing_directory_and_relative_path(self):
        self.root.mkdir()
        self.assertNotEqual(self.invoke(self.root).returncode, 0)
        self.assertEqual(list(self.root.iterdir()), [])
        self.assertNotEqual(self.invoke("relative-demo").returncode, 0)

    def test_live_web_stops_on_enter_and_preserves_candidates(self):
        process = subprocess.Popen(
            [BINARY, str(self.root), "--keep-web"], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        lines = queue.Queue()

        def read_lines():
            for line in process.stdout:
                lines.put(line)
            lines.put(None)

        threading.Thread(target=read_lines, daemon=True).start()
        try:
            record = None
            for _ in range(10):
                line = lines.get(timeout=30)
                self.assertIsNotNone(line, "Demo exited before reporting its endpoint")
                if line.startswith("{"):
                    record = json.loads(line)
                    break
            self.assertIsNotNone(record)
            self.assertTrue(record["web_running_until_input"])
            host, port = record["demo_web"].split(":")
            self.assertEqual(host, "127.0.0.1")
            for path, status in [("/demo.json", 200), ("/private", 404)]:
                connection = http.client.HTTPConnection(host, int(port), timeout=3)
                try:
                    connection.request("GET", path)
                    response = connection.getresponse()
                    self.assertEqual(response.status, status)
                    body = json.loads(response.read())
                    if status == 200:
                        self.assertEqual(body["mode"], "demo")
                        self.assertEqual(body["value"], 7)
                finally:
                    connection.close()
            process.stdin.write("\n")
            process.stdin.flush()
            self.assertEqual(process.wait(timeout=10), 0)
            with socket.socket() as probe:
                probe.settimeout(2)
                self.assertNotEqual(probe.connect_ex((host, int(port))), 0)
            self.verify_candidates(record)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)
            for pipe in [process.stdin, process.stdout, process.stderr]:
                pipe.close()


if __name__ == "__main__":
    unittest.main()
