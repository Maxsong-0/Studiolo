#!/usr/bin/env python3
"""原生虚构schema2组合测试；不创建或读取任何真实凭据。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import stat
import sys
import unittest

sys.path.insert(0, str(Path(__file__).absolute().parent))
import test_pack_offline as old

SOURCE = None
HumanFixture = None

class HumanPackTests(unittest.TestCase):
    write = old.OfflineTests.write
    run_command = old.OfflineTests.run_command
    pack = old.OfflineTests.pack
    verify = old.OfflineTests.verify

    def setUp(self):
        old.OfflineTests.setUp(self)
        self.addCleanup(old.OfflineTests.tearDown, self)
        self.human_fixture = HumanFixture()
        self.human_fixture.setUp()
        self.addCleanup(self.human_fixture.doCleanups)
        self.human_fixture.create()
        self.args.human = str(self.human_fixture.output)
        for name in ("human_snapshot.py", "human_snapshot_io.py", "human_snapshot_format.py", "human_snapshot_core.py"):
            self.write(self.tools / name, (SOURCE / "deploy/human/offline" / name).read_bytes())

    def change_manifest(self, callback):
        path = self.output / "manifest.json"
        value = json.loads(path.read_bytes())
        callback(value)
        self.write(path, json.dumps(value).encode())

    def rehash(self, relative):
        path = self.output / relative
        raw = path.read_bytes()
        self.change_manifest(lambda v: v["files"].__setitem__(relative,
            {"sha256": hashlib.sha256(raw).hexdigest(), "size": len(raw), "mode": stat.S_IMODE(path.stat().st_mode)}))

    def test_schema2_roundtrip_exact_human_files_modes_and_snapshot(self):
        source = self.human_fixture.output
        before = {str(p.relative_to(source)): (p.read_bytes(), stat.S_IMODE(p.stat().st_mode)) for p in source.rglob("*") if p.is_file()}
        self.pack()
        self.verify()
        manifest = json.loads((self.output / "manifest.json").read_bytes())
        self.assertEqual(manifest["schema_version"], 2)
        self.assertEqual(manifest["snapshot_id"], self.human_fixture.args.snapshot_id)
        actual = {str(p.relative_to(self.output / "human")): (p.read_bytes(), stat.S_IMODE(p.stat().st_mode)) for p in (self.output / "human").rglob("*") if p.is_file()}
        self.assertEqual(actual, before)
        self.assertIn("release/example-vault/附件演示/空目录/.gitkeep", before)

    def test_snapshot_id_mismatch_is_rejected(self):
        self.pack()
        self.change_manifest(lambda v: v.__setitem__("snapshot_id", "another-snapshot"))
        self.verify(ok=False)

    def test_schema1_cannot_smuggle_human_root(self):
        self.pack()
        def mutate(value):
            value["schema_version"] = 1
            del value["snapshot_id"]
        self.change_manifest(mutate)
        self.verify(ok=False)

    def test_independent_human_validator_is_mandatory(self):
        (self.tools / "human_snapshot_core.py").unlink()
        self.pack(ok=False)
        self.assertFalse(self.output.exists())

    def test_inner_damage_rejected_even_if_outer_digest_updated(self):
        self.pack()
        self.write(self.output / "human/portable-factor.age", b"damaged encrypted fixture")
        self.rehash("human/portable-factor.age")
        self.verify(ok=False)

    def test_bundle_tool_is_not_executed(self):
        self.pack()
        self.write(self.output / "tools/human_snapshot.py", b"raise SystemExit(99)\n")
        self.rehash("tools/human_snapshot.py")
        self.verify()

    def test_human_source_change_preserves_incomplete_output(self):
        module = runpy.run_path(str(old.TOOL))
        def mutate():
            self.write(self.human_fixture.output / "portable-factor.age", b"changed fixture")
        with self.assertRaises(ValueError):
            module["pack"](self.args, between_scans=mutate)
        self.assertTrue((self.output / "INCOMPLETE").is_file())
        self.verify(ok=False)

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    args, remaining = parser.parse_known_args()
    SOURCE = args.source.absolute()
    old.SOURCE = SOURCE
    sys.path.insert(0, str(SOURCE / "deploy/human/offline"))
    from test_human_snapshot_fixture import Fixture as HumanFixture
    unittest.main(argv=[sys.argv[0], *remaining])
