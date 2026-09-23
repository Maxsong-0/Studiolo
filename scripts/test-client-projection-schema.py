#!/usr/bin/env python3
"""投影配置及来源完整性、私有目录边界契约。"""
import os
from pathlib import Path
import tempfile
import unittest
import sys
from unittest.mock import patch

sys.dont_write_bytecode = True

from client_projection_io import read_bundle, write_bundle
import client_projection_io as projection_io
from client_projection_schema import MANIFEST_KEYS, config, projection_key, validate_bundle
from client_projection_test_support import (fixture_bundle, fixture_config, fixture_policy_bundle,
                                            fixture_policy_config)
from replica_common import Invalid, digest, encode


def resealed(mutate, factory=fixture_policy_bundle):
    """改动后重算 projection_key 与 bundle_id，确保拒绝来自语义校验而不是哈希失配。"""
    bundle = factory()
    manifest = bundle["manifest"]
    mutate(manifest)
    try:
        manifest["projection_key"] = projection_key(manifest)
    except (KeyError, TypeError):
        pass  # 缺键时无从重算，校验照样必须拒绝
    manifest["bundle_id"] = digest(encode({k: v for k, v in manifest.items() if k != "bundle_id"}))
    return bundle


class SchemaTests(unittest.TestCase):
    def test_clients_and_byte_exact_roundtrip(self):
        for client in ("claude", "codex"):
            with self.subTest(client=client), tempfile.TemporaryDirectory() as tmp:
                path = Path(tmp).resolve() / "bundle"
                bundle = fixture_bundle(client=client)
                write_bundle(path, bundle)
                self.assertEqual(read_bundle(path), bundle)
                self.assertEqual(path.stat().st_mode & 0o777, 0o700)
                for file in path.rglob("*"):
                    self.assertEqual(file.stat().st_mode & 0o777, 0o700 if file.is_dir() else 0o600)

    def test_config_rejects_live_or_ambiguous_sources_and_unsafe_urls(self):
        for field, value in (
            ("rules", ["项目/Studiolo/04-协调/TASKS.md"]),
            ("rules", ["_规则/04-协调/TASKS.md"]),
            ("rules", ["_规则/../secret.md"]),
            ("rules", ["_规则/.secret.md"]),
            ("rules", ["_规则/A.md", "_规则/a.md"]),
            ("skills", ["_规则/skills/a/nested"]),
            ("project", "项目/Alpha/child"),
            ("token_env", "TOKEN\nInjected"),
            ("source_url", "http://example.com/read"),
            ("source_url", "https://user:secret@example.com/read"),
            ("source_url", "https://example.com/read?token=secret"),
            ("source_url", "https://example.com/read#fragment"),
            ("source_url", "https://example.com/\nread"),
            ("schema_version", True),
        ):
            candidate = fixture_config()
            candidate[field] = value
            with self.subTest(field=field, value=value), self.assertRaises((Invalid, ValueError)):
                config(candidate)

    def test_manifest_mutations_rejected(self):
        for key in ("generated_at_ms", "bundle_id", "projection_key", "identity", "sources", "files"):
            bundle = fixture_bundle()
            bundle["manifest"][key] = 1 if key == "generated_at_ms" else "forged"
            with self.subTest(key=key), self.assertRaises((Invalid, ValueError, TypeError)):
                validate_bundle(bundle)
        bundle = fixture_bundle()
        bundle["files"]["sources/_规则/rules.md"] = b"modified"
        with self.assertRaises(Invalid):
            validate_bundle(bundle)

    def test_instance_and_timestamp_do_not_change_projection_key(self):
        first = fixture_bundle(instance="0" * 26)
        second = fixture_bundle(instance="0" * 25 + "1")
        self.assertEqual(first["manifest"]["projection_key"], second["manifest"]["projection_key"])
        self.assertNotEqual(first["manifest"]["bundle_id"], second["manifest"]["bundle_id"])

    def test_disk_drift_extra_file_and_links_rejected(self):
        for change in ("body", "extra", "symlink", "hardlink", "mode", "manifest"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp).resolve()
                target = root / "bundle"
                write_bundle(target, fixture_bundle())
                file = target / "CONTEXT.md"
                if change == "body":
                    file.write_bytes(b"tampered")
                elif change == "extra":
                    (target / "extra").write_bytes(b"unmanaged")
                elif change in ("symlink", "hardlink"):
                    other = root / "outside"
                    other.write_bytes(file.read_bytes())
                    other.chmod(0o600)
                    file.unlink()
                    os.symlink(other, file) if change == "symlink" else os.link(other, file)
                elif change == "mode":
                    file.chmod(0o644)
                else:
                    (target / "manifest.json").write_bytes(b"{}")
                with self.assertRaises((Invalid, OSError)):
                    read_bundle(target)

    def test_existing_output_and_symlink_parent_never_overwritten(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp).resolve()
            bundle = fixture_bundle()
            write_bundle(root / "bundle", bundle)
            with self.assertRaises(FileExistsError):
                write_bundle(root / "bundle", fixture_bundle("# new"))
            self.assertEqual(read_bundle(root / "bundle"), bundle)
            (root / "link").symlink_to(root / "bundle", target_is_directory=True)
            with self.assertRaises(OSError):
                write_bundle(root / "link" / "new", bundle)

    def test_disk_reads_are_bounded_by_declared_file_sizes_before_accumulation(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp).resolve() / "bundle"
            bundle = fixture_bundle()
            write_bundle(path, bundle)
            captured = []
            original = projection_io.read_file

            def observe(fd, name, limit, *args, **kwargs):
                captured.append((name, limit))
                return original(fd, name, limit, *args, **kwargs)

            with patch.object(projection_io, "read_file", observe):
                read_bundle(path)
            sizes = {p.rsplit("/", 1)[-1]: len(raw) for p, raw in bundle["files"].items()}
            self.assertTrue(all(limit == sizes[name] for name, limit in captured if name != "manifest.json"))
            (path / "CONTEXT.md").write_bytes(b"x" * 10000)
            with self.assertRaises(Invalid):
                read_bundle(path)

    def test_static_skill_index_is_not_mistaken_for_project_blackboard(self):
        candidate = fixture_config()
        candidate["rules"] = ["_规则/skills/demo/references/index.md"]
        self.assertEqual(config(candidate), candidate)

    def test_rules_source_is_optional_closed_set(self):
        self.assertEqual(config(fixture_policy_config()), fixture_policy_config())
        explicit = dict(fixture_config(), rules_source="vault")
        self.assertEqual(config(explicit), explicit)
        for value in ("Policy", "", None, 1, ["policy"], "vault/policy"):
            with self.subTest(value=value), self.assertRaises(Invalid):
                config(dict(fixture_policy_config(), rules_source=value))
        with self.assertRaises(Invalid):
            config(dict(fixture_config(), rule_source="policy"))

    def test_policy_names_with_separator_dotdot_control_or_empty_are_rejected(self):
        for field, value in (
            ("rules", ["a/b.md"]), ("rules", ["../secret.md"]), ("rules", ["a..md"]), ("rules", [".."]),
            ("rules", ["规则\x07.md"]), ("rules", ["规则\n.md"]), ("rules", [""]), ("rules", [".hidden.md"]),
            ("rules", ["规则.txt"]), ("rules", ["A.md", "a.md"]), ("rules", [1]), ("rules", []),
            ("rules", ["_规则/通用工作规则.md"]), ("rules", ["x" * 126 + ".md"]),
            ("skills", ["a/b"]), ("skills", [".."]), ("skills", [""]), ("skills", ["Demo"]),
            ("skills", ["bad\nname"]), ("skills", ["-demo"]), ("skills", ["demo", "demo"]),
            ("skills", ["_规则/skills/demo"]),
        ):
            candidate = fixture_policy_config()
            candidate[field] = value
            with self.subTest(field=field, value=value), self.assertRaises(Invalid):
                config(candidate)

    def test_policy_bundle_roundtrip_and_manifest_shape(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp).resolve() / "bundle"
            bundle = fixture_policy_bundle()
            write_bundle(path, bundle)
            self.assertEqual(read_bundle(path), bundle)
            self.assertTrue((path / "sources/policy/skills/demo/SKILL.md").is_file())
        self.assertEqual(set(fixture_bundle()["manifest"]), MANIFEST_KEYS)
        self.assertEqual(set(fixture_bundle()["manifest"]["sources"][0]), {"path", "revision", "bytes", "sha256"})
        validate_bundle(resealed(lambda m: None))
        # skill 短摘要是服务端规范 JSON 的摘要，与落盘字节无关；规则短摘要必须等于正文摘要。
        validate_bundle(resealed(lambda m: m["sources"][1].update(policy_sha12="f" * 12)))
        for name, mutate in {
            "effective_case": lambda m: m.update(policy_effective="0123456789AB"),
            "effective_missing": lambda m: m.pop("policy_effective"),
            "source_flag": lambda m: m.update(rules_source="vault"),
            "config_vault": lambda m: m["config"].pop("rules_source"),
            "version_zero": lambda m: m["sources"][0].update(policy_version=0),
            "version_text": lambda m: m["sources"][0].update(policy_version="3"),
            "version_bool": lambda m: m["sources"][0].update(policy_version=True),
            "rule_sha12": lambda m: m["sources"][0].update(policy_sha12="f" * 12),
            "vault_revision": lambda m: m["sources"][0].update(revision="c" * 64),
            "unselected_path": lambda m: m["sources"][0].update(path="policy/rules/其他.md"),
        }.items():
            with self.subTest(mutation=name), self.assertRaises(Invalid):
                validate_bundle(resealed(mutate))
        with self.assertRaises(Invalid):
            validate_bundle(resealed(lambda m: m.update(policy_effective="0" * 12), fixture_bundle))

    def test_policy_projection_key_tracks_effective_not_vault_generation(self):
        base = fixture_policy_bundle()["manifest"]["projection_key"]
        self.assertEqual(fixture_policy_bundle(generation="d" * 32)["manifest"]["projection_key"], base)
        self.assertNotEqual(fixture_policy_bundle(effective="fedcba987654")["manifest"]["projection_key"], base)
        self.assertNotEqual(fixture_policy_bundle(body="# 新规则\n")["manifest"]["projection_key"], base)


if __name__ == "__main__":
    unittest.main()
