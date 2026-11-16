#!/usr/bin/env python3
"""上线前备份只读核查脚本的用例：规则和 Rust 源码逐字对照，逐类造出会挡封存的现存数据。

真实 Broker、真实控制库上「脚本说挡 ⇔ 封存真的拒绝」的对照在 Rust 用例里：
crates/vault-broker/tests/broker/preflight_script.rs、crates/control-db/tests/backup_preflight_script.rs。
"""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
SCRIPT = Path(__file__).resolve().with_name("check-backup-preflight.py")
spec = importlib.util.spec_from_file_location("check_backup_preflight", SCRIPT)
preflight = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preflight)

REPOSITORY = SCRIPT.parent.parent
TASK = "01J00000000000000000000000"
ATTEMPT = "01J00000000000000000000001"
ATTEMPT_DIR = f"sources/attempts/{TASK}/{ATTEMPT}/"
PROVENANCE = {"source_policy": {"id": "fixture"}, "retain_until_unix_seconds": 4_000_000_000}


def rust(relative):
    return (REPOSITORY / relative).read_text(encoding="utf-8")


def rust_constant(relative, name):
    """取 `const NAME: … = 值;` 的值：字符串原样，数字算出乘积。"""
    match = re.search(rf"const {name}: [^=]+= ([^;]+);", rust(relative))
    assert match, f"{relative} 里找不到常量 {name}"
    value = match.group(1).strip()
    if value.startswith('"'):
        return json.loads(value)
    product = 1
    for factor in value.split("*"):
        product *= int(factor.strip().replace("_", ""))
    return product


def string_literals(text):
    return [json.loads(literal) for literal in re.findall(r'"(?:[^"\\]|\\.)*"', text)]


def write(root, relative, data=b"x"):
    path = Path(root) / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data if isinstance(data, bytes) else data.encode("utf-8"))
    return path


def manifest(version=2, provenance=True, artifacts=(f"{ATTEMPT_DIR}result.md",)):
    document = {"format_version": version, "artifacts": [{"path": path} for path in artifacts]}
    if provenance:
        document["provenance"] = PROVENANCE
    return json.dumps(document)


def staged(provenance=True):
    document = {"task_id": TASK, "attempt_id": ATTEMPT, "artifacts": []}
    if provenance:
        document["provenance"] = PROVENANCE
    return json.dumps(document)


class SourceParity(unittest.TestCase):
    """脚本里的常量和 SQL 要和 Rust 源码逐字一致；Rust 改了规则，这里先红。"""

    def test_name_and_limit_constants_match_the_rust_sources(self):
        pairs = [
            ("crates/vault-broker/src/fs/rooted.rs", "TEMPORARY_MARKER", preflight.TEMPORARY_MARKER),
            ("crates/vault-broker/src/residue.rs", "QUARANTINE", preflight.QUARANTINE),
            ("crates/vault-broker/src/artifact_stage.rs", "NAME_PREFIX", preflight.STAGED_PREFIX),
            ("crates/vault-broker/src/artifact_stage.rs", "NAME_SUFFIX", preflight.STAGED_SUFFIX),
            ("crates/vault-broker/src/trusted_export.rs", "ATTEMPTS", preflight.ATTEMPTS),
            ("crates/vault-broker/src/trusted_export.rs", "MANIFEST_NAME", preflight.MANIFEST_NAME),
            ("crates/vault-broker/src/trusted_export.rs", "MANIFEST_LIMIT", preflight.MANIFEST_LIMIT),
            ("crates/vault-broker/src/trusted_export.rs", "STAGED_LIMIT", preflight.STAGED_LIMIT),
            ("crates/vault-broker/src/trusted_export.rs", "MAX_FILES", preflight.EXPORT_MAX_FILES),
            ("crates/vault-broker/src/trusted_export.rs", "MAX_DEPTH", preflight.MAX_DIRECTORY_NESTING + 1),
            ("crates/vault-broker/src/checkpoint/tree.rs", "MAX_NODES", preflight.TREE_MAX_NODES),
            ("crates/vault-broker/src/checkpoint/tree.rs", "MAX_BYTES", preflight.TREE_MAX_BYTES),
        ]
        for relative, name, value in pairs:
            with self.subTest(name=name):
                self.assertEqual(rust_constant(relative, name), value)
        residue = rust("crates/vault-broker/src/residue.rs")
        self.assertIn('VAULT_QUARANTINE: &[&str] = &[".workbench", QUARANTINE];', residue)
        self.assertIn("DATA_QUARANTINE: &[&str] = &[QUARANTINE];", residue)
        self.assertEqual(preflight.WORKBENCH, ".workbench")

    def test_limit_semantics_match_the_rust_sources(self):
        # check_pair 把两棵树合起来比，脚本的 check_totals 也合起来比（复审 A3-F1）。
        tree = rust("crates/vault-broker/src/checkpoint/tree.rs")
        pair = tree[tree.index("pub(super) fn check_pair"):]
        pair = pair[: pair.index("\n}\n")]
        self.assertIn(".checked_add(right.nodes)", pair)
        self.assertIn(".checked_add(right.bytes)", pair)
        self.assertIn("is_none_or(|v| v > MAX_NODES)", pair)
        self.assertIn("is_none_or(|v| v > MAX_BYTES)", pair)
        # 导出扫描在插入之前「已经到上限」才拒绝：恰好上限个是允许的，超过才算（复审 A3-F3）。
        export = rust("crates/vault-broker/src/trusted_export.rs")
        walk = export[export.index("fn walk("):]
        self.assertLess(walk.index("if files.len() >= MAX_FILES"), walk.index("files.insert(path, revision)"))
        # 开库清扫只往本用户私有的隔离区里挪（复审 A3-F6）。
        residue = rust("crates/vault-broker/src/residue.rs")
        ensure = residue[residue.index("fn ensure_directory"):]
        ensure = ensure[: ensure.index("\n}\n")]
        self.assertIn("metadata.uid() == rustix::process::geteuid().as_raw()", ensure)
        self.assertIn("metadata.mode() & 0o077 == 0", ensure)
        # 封存以只读打开每个文件：属主要有读权限（复审 A3-F5）。
        self.assertIn("OFlags::RDONLY", rust("crates/vault-broker/src/checkpoint/files.rs"))

    def test_combining_mark_ranges_match_vault_path(self):
        source = rust("crates/core/src/vault_path.rs")
        body = source[source.index("fn is_unicode_combining_mark"):]
        body = body[: body.index("\n}\n")]
        ranges = tuple((int(lo, 16), int(hi, 16)) for lo, hi in re.findall(r"0x([0-9a-f]+)\.\.=0x([0-9a-f]+)", body))
        self.assertEqual(ranges, preflight.COMBINING_MARKS)

    def test_control_queries_are_the_checkpoint_validation_verbatim(self):
        source = rust("crates/control-db/src/checkpoint/validate.rs")
        block = source[source.index("    for query in ["):]
        block = block[: block.index("    ] {")]
        self.assertEqual(string_literals(block), [query for _, _, query in preflight.CONTROL_ZERO_CHECKS])
        self.assertIn(json.dumps(preflight.CONTROL_FENCE[2]), source)
        policies = rust("crates/control-db/src/artifact_policies.rs")
        self.assertIn(json.dumps(preflight.CONTROL_POLICY_ORPHAN[2]), policies)

    def test_every_attempts_or_tasks_count_query_lists_its_rows(self):
        for kind, _, query in preflight.CONTROL_ZERO_CHECKS:
            table = re.match(r"SELECT count\(\*\) FROM (\w+)", query).group(1)
            if table in ("attempts", "tasks", "artifact_commits"):
                with self.subTest(kind=kind):
                    self.assertIsNotNone(preflight.listing_query(query))


class NameRules(unittest.TestCase):
    def test_exact_temporary_names_follow_the_rust_cases(self):
        # crates/vault-broker/tests/residue_private/mod.rs `only_the_exact_atomic_write_shape_counts_as_residue`
        for name in (".a.studiolo-tmp-1-1", ".note.md.studiolo-tmp-4242-18446744073709551615",
                     "..hidden.studiolo-tmp-1-2", ".a.studiolo-tmp-x.studiolo-tmp-1-2"):
            with self.subTest(name=name):
                self.assertTrue(preflight.is_exact_temporary_name(name))
                self.assertTrue(preflight.is_temporary_name(name))
        for name in (".studiolo-tmp-1-1", ".a.studiolo-tmp-1", ".a.studiolo-tmp-1-", ".a.studiolo-tmp--1",
                     ".a.studiolo-tmp-1-2-3", ".a.studiolo-tmp-x-1", ".a.studiolo-tmp-1-123456789012345678901",
                     "a.studiolo-tmp-1-1"):
            with self.subTest(name=name):
                self.assertFalse(preflight.is_exact_temporary_name(name))
        self.assertTrue(preflight.is_temporary_name(".a.studiolo-tmp-1"))
        self.assertFalse(preflight.is_temporary_name("a.studiolo-tmp-1-1"))

    def test_vault_path_rules(self):
        self.assertIsNone(preflight.vault_path_problem("canonical/笔记/今天.md"))
        for raw in ("", "a\\b", "a\x01b", "e\u0301.md", "C:/x", "a%2Eb", "a%2fb", "/abs", "a//b", "a/./b", "a/../b"):
            with self.subTest(raw=raw):
                self.assertIsNotNone(preflight.vault_path_problem(raw))

    def test_attempt_manifest_level(self):
        self.assertEqual(preflight.attempt_manifest_directory(f"{ATTEMPT_DIR}manifest.json"), ATTEMPT_DIR)
        self.assertIsNone(preflight.attempt_manifest_directory(f"{ATTEMPT_DIR}nested/manifest.json"))
        self.assertIsNone(preflight.attempt_manifest_directory(f"sources/attempts/{TASK}/manifest.json"))


class Fixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.vault = self.root / "vault"
        self.data = self.root / "data"
        write(self.vault, "notes/plain.md", "正文")
        write(self.vault, ".workbench/vault.json", "{}")
        write(self.vault, f"{ATTEMPT_DIR}manifest.json", manifest())
        write(self.vault, f"{ATTEMPT_DIR}result.md", "结果")
        write(self.data, "writer-state.json", "{}")
        write(self.data, f"artifact-candidate-{'ab' * 32}.json", staged())

    def tearDown(self):
        self.temp.cleanup()

    def check(self, *extra):
        arguments = ["--vault", str(self.vault), "--data", str(self.data), "--json", *extra]
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = preflight.main(arguments)
        return code, json.loads(output.getvalue())

    def kinds(self, *extra):
        code, result = self.check(*extra)
        kinds = sorted({finding["kind"] for finding in result["findings"]})
        self.assertEqual(code, 1 if kinds else 0, result)
        return kinds, result


class FileSystemChecks(Fixture):
    def test_current_layout_is_clean(self):
        kinds, result = self.kinds()
        self.assertEqual(kinds, [])
        self.assertEqual(len(result["checked"]), 2)

    def test_old_missing_and_unlisted_attempt_contents(self):
        write(self.vault, f"{ATTEMPT_DIR}manifest.json", manifest(version=1, provenance=False))
        self.assertEqual(self.kinds()[0], ["old_manifest"])
        write(self.vault, f"{ATTEMPT_DIR}manifest.json", manifest(provenance=False))
        self.assertEqual(self.kinds()[0], ["old_manifest"])
        write(self.vault, f"{ATTEMPT_DIR}manifest.json", manifest())
        write(self.vault, f"{ATTEMPT_DIR}extra.md")
        write(self.vault, f"sources/attempts/{TASK}/stray.md")
        kinds, result = self.kinds()
        self.assertEqual(kinds, ["unlisted_attempt_file"])
        self.assertEqual(len(result["findings"]), 2)
        (self.vault / f"{ATTEMPT_DIR}extra.md").unlink()
        (self.vault / f"sources/attempts/{TASK}/stray.md").unlink()
        write(self.vault, f"sources/attempts/{TASK}/other/a.md")
        write(self.vault, f"sources/attempts/{TASK}/other/b.md")
        kinds, result = self.kinds()
        self.assertEqual(kinds, ["missing_manifest"])
        self.assertIn("2 个文件", result["findings"][0]["detail"])

    def test_broken_manifests(self):
        write(self.vault, f"{ATTEMPT_DIR}manifest.json", "{not json")
        self.assertEqual(self.kinds()[0], ["manifest_unreadable"])
        write(self.vault, f"{ATTEMPT_DIR}manifest.json", manifest(artifacts=("notes/plain.md",)))
        self.assertEqual(self.kinds()[0], ["manifest_outside"])
        write(self.vault, f"{ATTEMPT_DIR}manifest.json",
              manifest(artifacts=(f"{ATTEMPT_DIR}result.md", f"{ATTEMPT_DIR}result.md")))
        self.assertEqual(self.kinds()[0], ["double_claim"])
        # 更深处叫 manifest.json 的是产物：清单列了它就不算问题。
        write(self.vault, f"{ATTEMPT_DIR}manifest.json",
              manifest(artifacts=(f"{ATTEMPT_DIR}result.md", f"{ATTEMPT_DIR}nested/manifest.json", f"{ATTEMPT_DIR}gone.md")))
        write(self.vault, f"{ATTEMPT_DIR}nested/manifest.json", "an artifact")
        self.assertEqual(self.kinds()[0], [])

    def test_staged_bodies(self):
        name = f"artifact-candidate-{'cd' * 32}.json"
        write(self.data, name, staged(provenance=False))
        self.assertEqual(self.kinds()[0], ["staged_legacy"])
        (self.data / name).unlink()
        write(self.data, "artifact-candidate-oops.json", staged())
        self.assertEqual(self.kinds()[0], ["staged_unreadable"])
        (self.data / "artifact-candidate-oops.json").unlink()
        write(self.data, f"nested/artifact-candidate-{'ef' * 32}.json", staged())
        self.assertEqual(self.kinds()[0], ["staged_misplaced"])

    def test_residue_hints_follow_where_the_sweep_reaches(self):
        swept = write(self.vault, ".workbench/.vault.json.studiolo-tmp-12-3")
        data_swept = write(self.data, ".writer-state.json.studiolo-tmp-12-4")
        stuck = write(self.vault, "notes/.plain.md.studiolo-tmp-12-5")
        near_miss = write(self.data, ".x.studiolo-tmp-12")
        kinds, result = self.kinds()
        self.assertEqual(kinds, ["residue"])
        hints = {finding["where"]: finding["detail"] for finding in result["findings"]}
        self.assertIn("挪进隔离区", hints[str(swept)])
        self.assertIn("挪进隔离区", hints[str(data_swept)])
        self.assertIn("要人看过", hints[str(stuck)])
        self.assertIn("要人看过", hints[str(near_miss)])

    def test_quarantine_is_skipped_only_at_its_two_places(self):
        write(self.vault, ".workbench/residue-quarantine/1-2-3/.a.studiolo-tmp-1-1")
        write(self.data, "residue-quarantine/1-2-3/.b.studiolo-tmp-1-1")
        kinds, result = self.kinds()
        self.assertEqual(kinds, [])
        self.assertEqual(len(result["notes"]), 2)
        write(self.vault, "notes/residue-quarantine/.c.studiolo-tmp-1-1")
        self.assertEqual(self.kinds()[0], ["residue"])

    def test_nodes_the_capture_refuses(self):
        os.symlink("plain.md", self.vault / "notes/link.md")
        os.link(self.vault / "notes/plain.md", self.vault / "notes/hard.md")
        os.mkfifo(self.data / "pipe")
        write(self.vault, "notes/a%2e.md")
        (self.vault / "notes/bad\x07dir").mkdir()
        kinds, result = self.kinds()
        self.assertEqual(kinds, ["bad_name", "hard_link", "special_node"])
        # 硬链接的两个名字都算。
        self.assertEqual(sum(f["kind"] == "hard_link" for f in result["findings"]), 2)

    def test_unsafe_quarantine_is_not_promised_as_a_sweep_target(self):
        swept = write(self.vault, ".workbench/.vault.json.studiolo-tmp-12-3")
        data_swept = write(self.data, ".writer-state.json.studiolo-tmp-12-4")
        for quarantine in (self.vault / ".workbench/residue-quarantine", self.data / "residue-quarantine"):
            quarantine.mkdir()
            quarantine.chmod(0o750)
        kinds, result = self.kinds()
        self.assertEqual(kinds, ["residue"])
        hints = {finding["where"]: finding["detail"] for finding in result["findings"]}
        for path in (swept, data_swept):
            self.assertIn("不会往里挪", hints[str(path)])
            self.assertNotIn("挪进隔离区", hints[str(path)])
        (self.vault / ".workbench/residue-quarantine").chmod(0o700)
        hints = {finding["where"]: finding["detail"] for finding in self.kinds()[1]["findings"]}
        self.assertIn("挪进隔离区", hints[str(swept)])
        self.assertIn("不会往里挪", hints[str(data_swept)])

    def test_files_and_directories_the_owner_cannot_read(self):
        locked = write(self.vault, "notes/locked.md")
        locked.chmod(0o200)
        closed = self.vault / "notes/closed"
        closed.mkdir()
        write(closed, "inside.md")
        closed.chmod(0o600)
        try:
            kinds, result = self.kinds()
        finally:
            closed.chmod(0o700)
        self.assertEqual(kinds, ["unreadable"])
        details = {finding["where"]: finding["detail"] for finding in result["findings"]}
        self.assertIn("读", details[str(locked)])
        self.assertIn("进入", details[str(closed)])
        self.assertNotIn(str(closed / "inside.md"), details, "进不去的目录不往下走")

    def test_limits_are_compared_across_both_trees(self):
        report = preflight.Report(uid=os.geteuid())
        vault_nodes = preflight.check_vault(self.vault, report).nodes
        data = preflight.check_data(self.data, report)
        total = vault_nodes + data.nodes
        total_bytes = preflight.check_vault(self.vault, report).total_bytes + data.total_bytes
        # 每棵单独都在上限以内、合起来超了：照样挡（tree.rs check_pair）。
        with mock.patch.object(preflight, "TREE_MAX_NODES", total - 1):
            kinds, result = self.kinds()
        self.assertEqual(kinds, ["too_many_nodes"])
        self.assertIn("至少", result["findings"][0]["detail"])
        with mock.patch.object(preflight, "TREE_MAX_NODES", total):
            self.assertEqual(self.kinds()[0], [])
        with mock.patch.object(preflight, "TREE_MAX_BYTES", total_bytes - 1):
            self.assertEqual(self.kinds()[0], ["too_many_bytes"])
        with mock.patch.object(preflight, "TREE_MAX_BYTES", total_bytes):
            self.assertEqual(self.kinds()[0], [])

    def test_the_walk_stops_at_the_node_budget(self):
        big = self.root / "big"
        for index in range(50):
            write(big, f"many/{index:02}.md")
        report = preflight.Report(uid=os.geteuid())
        listed = []
        original = preflight.listed_entries

        def counting(directory, room):
            entries = original(directory, room)
            listed.append(len(entries))
            return entries

        with mock.patch.object(preflight, "listed_entries", counting):
            tree = preflight.check_vault(big, report, node_budget=5)
        self.assertTrue(tree.stopped)
        self.assertEqual(tree.nodes, 6)
        self.assertEqual(listed, [1, 6], "50 个目录项的那一层只读进来 6 个")

    def test_export_file_limit_counts_only_above_it(self):
        report = preflight.Report(uid=os.geteuid())
        files = preflight.check_vault(self.vault, report).file_count + preflight.check_data(self.data, report).file_count
        with mock.patch.object(preflight, "EXPORT_MAX_FILES", files):
            kinds, result = self.kinds()
        self.assertEqual(kinds, [], "恰好上限个是允许的")
        self.assertTrue(any("可能已经超了" in note for note in result["notes"]))
        with mock.patch.object(preflight, "EXPORT_MAX_FILES", files - 1):
            kinds, result = self.kinds()
        self.assertEqual(kinds, ["too_many_files"])
        self.assertIn("只数了这两棵树", result["findings"][0]["detail"])

    def test_quarantine_count_stops_and_says_at_least(self):
        for index in range(3):
            write(self.data, f"residue-quarantine/1-2-3/.f{index}.studiolo-tmp-1-{index}")
        _, result = self.kinds()
        self.assertTrue(any("residue-quarantine 里有 4 个目录项" in note for note in result["notes"]), result["notes"])
        with mock.patch.object(preflight, "QUARANTINE_COUNT_LIMIT", 2):
            kinds, result = self.kinds()
        self.assertEqual(kinds, [])
        self.assertTrue(any("residue-quarantine 里有至少 2 个目录项" in note for note in result["notes"]), result["notes"])

    def test_quarantine_count_is_bounded_even_for_empty_directories(self):
        """隔离区里全是空目录时也数到上限就停，每层只读到上限为止，不整棵走完（复审 R2-F4）。"""
        quarantine = self.data / "residue-quarantine"
        for index in range(10):
            (quarantine / f"batch-{index}" / "deeper").mkdir(parents=True)
        read = []
        original = preflight.listed_entries

        def counting(directory, room):
            entries = original(directory, room)
            read.append(len(entries))
            return entries

        with mock.patch.object(preflight, "listed_entries", counting):
            self.assertEqual(preflight.count_quarantined(quarantine, 4), (4, True))
        self.assertLessEqual(sum(read), 4, "数到上限就停，不整棵走完")
        self.assertEqual(preflight.count_quarantined(quarantine, 1000), (20, False))
        os.symlink(self.root, quarantine / "batch-0" / "link")
        self.assertEqual(preflight.count_quarantined(quarantine, 1000), (21, False), "链接只算一项，不跟进去")

    def test_one_tree_only_is_noted(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(preflight.main(["--vault", str(self.vault), "--json"]), 0)
        self.assertTrue(any("两棵合计" in note for note in json.loads(output.getvalue())["notes"]))

    def test_unreadable_json_never_crashes_the_check(self):
        for body in ("[" * 100_000 + "]" * 100_000, '{"format_version": 1' + "0" * 5000 + "}"):
            write(self.vault, f"{ATTEMPT_DIR}manifest.json", body)
            self.assertEqual(self.kinds()[0], ["manifest_unreadable"])

    def test_entries_that_vanish_mid_scan_are_skipped(self):
        doomed = write(self.vault, ".workbench/.vault.json.studiolo-tmp-1-1")
        original = preflight.listed_entries

        def racing(directory, room):
            entries = original(directory, room)
            if Path(directory) == doomed.parent and doomed.exists():
                doomed.unlink()
            return entries

        with mock.patch.object(preflight, "listed_entries", racing):
            kinds, result = self.kinds()
        self.assertEqual(kinds, [])
        self.assertTrue(any("不见了" in note for note in result["notes"]), result["notes"])

    def test_unexpected_errors_exit_two_not_one(self):
        with mock.patch.object(preflight, "check_vault", side_effect=RuntimeError("boom")), \
                contextlib.redirect_stdout(io.StringIO()) as output, \
                contextlib.redirect_stderr(io.StringIO()) as error:
            self.assertEqual(preflight.main(["--vault", str(self.vault)]), 2)
        self.assertIn("意外错误 RuntimeError", error.getvalue())
        self.assertIn("没核成：Vault 根", output.getvalue())

    def test_a_tree_that_cannot_be_checked_does_not_hide_the_other(self):
        """一棵树核不成，另一棵照样核完、问题照样列出；退出码 2，合计只算核成的那棵（复审 R2-F5）。"""
        write(self.data, ".writer-state.json.studiolo-tmp-1-1")
        output = io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(io.StringIO()) as error:
            code = preflight.main(["--vault", str(self.root / "missing"), "--data", str(self.data), "--json"])
        self.assertEqual(code, 2)
        result = json.loads(output.getvalue())
        self.assertEqual(sorted({finding["kind"] for finding in result["findings"]}), ["residue"])
        self.assertEqual(len(result["incomplete"]), 1)
        self.assertIn("Vault 根", result["incomplete"][0])
        self.assertTrue(any("没核成" in note for note in result["notes"]), result["notes"])
        self.assertIn("核查没做完：Vault 根", error.getvalue())

    def test_a_read_error_after_opening_is_unreadable_not_a_crash(self):
        """打开了、读的时候出错（如磁盘 I/O 错）：和打不开一样报「读不出来」，别的照样核，不算没核成
        （2026-11-16 第二意见）。"""
        with mock.patch.object(preflight.os, "fstat", side_effect=OSError(5, "Input/output error")):
            kinds, result = self.kinds()
        self.assertEqual(kinds, ["manifest_unreadable", "staged_unreadable"])
        self.assertEqual(result["incomplete"], [])

    def test_a_tree_that_fails_after_its_walk_still_uses_up_the_shared_budget(self):
        """Vault 根走完了树、后面一步出错没核成：走过的目录项照样占两棵树共用的预算、照样进合计，写者数据根
        拿不到满额预算，合计超了照样报（2026-11-16 第二意见）。"""
        report = preflight.Report(uid=os.geteuid())
        vault_nodes = preflight.check_vault(self.vault, report).nodes
        data_nodes = preflight.check_data(self.data, report).nodes
        self.assertGreater(data_nodes, 0)
        output = io.StringIO()
        with mock.patch.object(preflight, "check_attempts", side_effect=RuntimeError("boom")), \
                mock.patch.object(preflight, "TREE_MAX_NODES", vault_nodes + data_nodes - 1), \
                contextlib.redirect_stdout(output), contextlib.redirect_stderr(io.StringIO()):
            code = preflight.main(["--vault", str(self.vault), "--data", str(self.data), "--json"])
        self.assertEqual(code, 2)
        result = json.loads(output.getvalue())
        self.assertEqual(len(result["incomplete"]), 1)
        self.assertIn("Vault 根", result["incomplete"][0])
        details = {finding["kind"]: finding["detail"] for finding in result["findings"]}
        self.assertIn("too_many_nodes", details, result)
        self.assertIn(f"至少 {vault_nodes + data_nodes} 个目录项", details["too_many_nodes"])
        self.assertIn("数到上限就停了", details["too_many_nodes"], "写者数据根只拿到剩下的预算，走到上限就停")

    def test_owner_is_compared_against_the_running_account_or_uid_flag(self):
        kinds, result = self.kinds("--uid", str(os.geteuid() + 1))
        self.assertEqual(kinds, ["foreign_owner"])
        self.assertEqual(result["uid"], os.geteuid() + 1)

    def test_bad_roots_and_arguments_exit_two(self):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(preflight.main(["--vault", str(self.root / "missing")]), 2)
            os.symlink(self.vault, self.root / "vault-link")
            self.assertEqual(preflight.main(["--vault", str(self.root / "vault-link")]), 2)
            with self.assertRaises(SystemExit) as raised:
                preflight.main([])
        self.assertEqual(raised.exception.code, 2)

    def test_nothing_under_the_roots_changes(self):
        before = snapshot(self.root)
        self.kinds()
        self.assertEqual(snapshot(self.root), before)

    def test_runs_from_standard_input_like_over_ssh(self):
        with SCRIPT.open("rb") as source:
            completed = subprocess.run(
                [sys.executable, "-I", "-", "--vault", str(self.vault), "--data", str(self.data)],
                stdin=source, capture_output=True, check=False)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("没发现会挡封存的现存数据", completed.stdout.decode("utf-8"))


def snapshot(root):
    """每个路径的类型、大小、修改时间；用来证明核查没写任何东西。"""
    result = {}
    for directory, directories, files in os.walk(root):
        for name in directories + files:
            info = os.lstat(os.path.join(directory, name))
            result[os.path.relpath(os.path.join(directory, name), root)] = (info.st_mode, info.st_size, info.st_mtime_ns)
    return result


CONTROL_SCHEMA = """
CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY);
CREATE TABLE vault_binding(singleton INTEGER PRIMARY KEY, vault_uuid TEXT NOT NULL);
CREATE TABLE fence_counter(singleton INTEGER PRIMARY KEY, next_fence INTEGER NOT NULL);
CREATE TABLE snapshots(snapshot_id TEXT PRIMARY KEY, vault_uuid TEXT NOT NULL);
CREATE TABLE tasks(task_id TEXT PRIMARY KEY, status TEXT, created_at_ms INTEGER, updated_at_ms INTEGER,
                   active_attempt TEXT, last_attempt TEXT);
CREATE TABLE attempts(attempt_id TEXT PRIMARY KEY, task_id TEXT, phase TEXT, fence INTEGER,
                      created_at_ms INTEGER, updated_at_ms INTEGER, terminal_json TEXT);
CREATE TABLE projection_outbox(sequence INTEGER PRIMARY KEY, delivered_at_ms INTEGER);
CREATE TABLE terminal_audit_outbox(attempt_id TEXT PRIMARY KEY, delivered_at_ms INTEGER);
CREATE TABLE runtime_audit_outbox(event_id TEXT PRIMARY KEY, action TEXT, delivered INTEGER, sequence INTEGER);
CREATE TABLE artifact_commits(attempt_id TEXT PRIMARY KEY);
CREATE TABLE artifact_policies(approval_event_id TEXT PRIMARY KEY);
INSERT INTO schema_migrations VALUES (1), (2);
INSERT INTO vault_binding VALUES (1, 'v');
INSERT INTO fence_counter VALUES (1, 10);
INSERT INTO tasks VALUES ('t1', 'needs_review', 1, 2, NULL, 'a1');
"""
TERMINAL = json.dumps({"report_json": {"format": 3}, "artifacts_json": {"id": "a", "sha256": "b"}})


class ControlChecks(unittest.TestCase):
    """手搭的最小结构只验脚本自己的逻辑；真实控制库上的对照在 control-db 的 Rust 用例里。"""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.directory = Path(self.temp.name)
        self.path = self.directory / "control.sqlite3"
        self.writer = sqlite3.connect(self.path)
        self.writer.execute("PRAGMA journal_mode=WAL")
        self.writer.executescript(CONTROL_SCHEMA)
        self.writer.execute("INSERT INTO attempts VALUES ('a1','t1','completed',3,1,2,?)", (TERMINAL,))
        self.writer.commit()

    def tearDown(self):
        self.writer.close()
        self.temp.cleanup()

    def kinds(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = preflight.main(["--control", str(self.path), "--json"])
        result = json.loads(output.getvalue())
        kinds = sorted({finding["kind"] for finding in result["findings"]})
        self.assertEqual(code, 1 if kinds else 0, result)
        return kinds, result

    def test_clean_committing_and_legacy_terminal(self):
        self.assertEqual(self.kinds()[0], [])
        self.writer.execute("UPDATE attempts SET phase='committing', terminal_json=NULL")
        self.writer.commit()
        kinds, result = self.kinds()
        self.assertEqual(kinds, ["control_committing"])
        self.assertEqual(result["findings"][0]["where"], "t1/a1")
        legacy = json.dumps({"report_json": {"format": 3}, "artifacts_json": []})
        self.writer.execute("UPDATE attempts SET phase='completed', terminal_json=?", (legacy,))
        self.writer.commit()
        self.assertEqual(self.kinds()[0], ["control_terminal_shape"])
        self.writer.execute("UPDATE attempts SET terminal_json=?", (TERMINAL,))
        self.writer.execute("UPDATE fence_counter SET next_fence=3")
        self.writer.commit()
        self.assertEqual(self.kinds()[0], ["control_fence"])

    def test_reading_never_creates_or_changes_files_with_or_without_the_server(self):
        # 写连接开着（主服务在跑）：-wal、-shm 在，只读模式共享它们。
        self.assertTrue(Path(f"{self.path}-shm").exists())
        before = snapshot(self.directory)
        self.kinds()
        self.assertEqual(snapshot(self.directory), before)
        # 关掉（主服务停了）：-wal、-shm 被收掉，按不可变方式读主文件，也不能再建出来。
        self.writer.close()
        self.writer = sqlite3.connect(":memory:")
        self.assertFalse(Path(f"{self.path}-shm").exists())
        before = snapshot(self.directory)
        self.kinds()
        self.assertEqual(snapshot(self.directory), before)

    def test_a_live_database_is_read_only_by_its_owner_account(self):
        self.assertTrue(Path(f"{self.path}-shm").exists())
        before = snapshot(self.directory)
        with mock.patch.object(preflight.os, "geteuid", return_value=os.stat(self.path).st_uid + 1), \
                contextlib.redirect_stdout(io.StringIO()), \
                contextlib.redirect_stderr(io.StringIO()) as error:
            self.assertEqual(preflight.main(["--control", str(self.path), "--uid", "0"]), 2)
        self.assertIn("属主账号", error.getvalue())
        self.assertEqual(snapshot(self.directory), before)

    def test_a_refused_control_check_keeps_what_the_trees_found(self):
        """控制库这一项因为账号不对没核成：前面两棵树查出的问题照样输出，退出码 2（复审 R2-F5）。"""
        vault = self.directory / "vault"
        write(vault, "notes/.a.md.studiolo-tmp-1-1")
        owner = os.stat(self.path).st_uid
        arguments = ["--vault", str(vault), "--control", str(self.path), "--uid", str(owner)]
        for extra in (["--json"], []):
            output = io.StringIO()
            with mock.patch.object(preflight.os, "geteuid", return_value=owner + 1), \
                    contextlib.redirect_stdout(output), \
                    contextlib.redirect_stderr(io.StringIO()) as error:
                self.assertEqual(preflight.main(arguments + extra), 2)
            self.assertIn("属主账号", error.getvalue())
            if extra:
                result = json.loads(output.getvalue())
                self.assertEqual([finding["kind"] for finding in result["findings"]], ["residue"])
                self.assertEqual(len(result["incomplete"]), 1)
                self.assertIn("属主账号", result["incomplete"][0])
            else:
                self.assertIn("会让每一次整批封存都失败", output.getvalue())
                self.assertIn("没核成：控制库", output.getvalue())

    def test_wal_without_shm_is_refused_untouched(self):
        copy = self.directory / "copy"
        copy.mkdir()
        self.writer.execute("PRAGMA wal_autocheckpoint=0")
        self.writer.execute("INSERT INTO tasks VALUES ('t2','queued',1,2,NULL,NULL)")
        self.writer.commit()
        shutil.copy(self.path, copy / "control.sqlite3")
        shutil.copy(f"{self.path}-wal", copy / "control.sqlite3-wal")
        before = snapshot(copy)
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()) as error:
            self.assertEqual(preflight.main(["--control", str(copy / "control.sqlite3")]), 2)
        self.assertIn("先让主服务正常开一次库", error.getvalue())
        self.assertEqual(snapshot(copy), before)


if __name__ == "__main__":
    unittest.main()
