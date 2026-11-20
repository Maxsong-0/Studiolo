#!/usr/bin/env python3
"""Mac 同步端的容量提醒（T-217，vault BUG-039 后续）。

三层：纯逻辑（四条上限各自 79% 不报、80% 报，越线、文字、去重）；副作用那层（去重状态文件、osascript），
用临时私有目录和 PATH 替身；入口 sync-agent-vault.sh 端到端（预演不发、到线只发一次、越线点名并通知）。
端到端要把 Markdown 合计真的堆到 32～40 MiB，所以放在这个文件里、不随 agent 运行包到生产主机上跑。
"""
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import textwrap
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
import replica_client
from replica_capacity import (CAPACITY_WARN_PERCENT, DAY_MS, GAUGES, LEVELS, LIMITS, MIB, REMIND_AFTER_MS,
                              LimitExceeded, dump_state, exceeded_notice, forget, levels, measure, parse_state,
                              plan, report, warn_notice, within)
from replica_common import MAX_MANIFEST, MAX_NOTES_TOTAL, Invalid, encode, open_root

_SPEC = importlib.util.spec_from_file_location("sync_tests", Path(__file__).with_name("test-sync-agent-vault.py"))
SYNC = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(SYNC)

NAMES = {key: name for key, name, _ in GAUGES}
IN_BYTES = {key: in_bytes for key, _, in_bytes in GAUGES}


def at(limit, percent):
    """用量刚好到某个百分比（向上取整），同 agent 那边的单测。"""
    return -(-limit * percent // 100)


def usage(**changes):
    value = {"manifest_bytes": 0, "entries": 0, "visible_entries": 0, "total_bytes": 0, "notes_bytes": 0}
    value.update(changes)
    return value


def shown(key, value):
    return f"{value / MIB:.1f} MiB" if IN_BYTES[key] else str(value)


class CapacityLogicTests(unittest.TestCase):
    def test_each_limit_warns_from_the_warning_percent_and_not_one_unit_before(self):
        self.assertIsNone(warn_notice(report(usage())))
        for key, name, _ in GAUGES:
            limit = LIMITS[key]
            with self.subTest(gauge=key):
                below = report(usage(**{key: at(limit, CAPACITY_WARN_PERCENT) - 1}))
                self.assertEqual(below[key]["percent"], CAPACITY_WARN_PERCENT - 1)
                self.assertIsNone(warn_notice(below), "差一点到线不提醒")
                self.assertEqual(levels(below), {k: None for k in LIMITS})
                used = at(limit, CAPACITY_WARN_PERCENT)
                hit = report(usage(**{key: used}))
                self.assertEqual(hit[key], {"used": used, "limit": limit, "percent": CAPACITY_WARN_PERCENT,
                                            **({"visible": 0} if key == "entries" else {})})
                self.assertEqual(levels(hit), {k: "warn" if k == key else None for k in LIMITS})
                title, body = warn_notice(hit)
                self.assertEqual(title, "Studiolo 副本快到上限")
                # 说清哪条、用了多少、上限多少、百分之几，以及该怎么办。
                self.assertTrue(body.startswith(
                    f"{name} {shown(key, used)}，上限 {shown(key, limit)} 的 {CAPACITY_WARN_PERCENT}%。"), body)
                for words in ("Mac 同步整批失败", "BUG-039", "调高两端上限", "agent 先部署、Mac 同步包紧跟",
                              "精简白名单", "docs/m3/replica-freshness.md"):
                    self.assertIn(words, body)
                full = warn_notice(report(usage(**{key: limit})))[1]
                self.assertIn("的 100%", full)

    def test_every_reached_limit_is_listed_in_one_notice(self):
        both = warn_notice(report(usage(notes_bytes=MAX_NOTES_TOTAL, manifest_bytes=MAX_MANIFEST)))[1]
        self.assertTrue(both.startswith(
            f"Markdown 合计 {shown('notes_bytes', MAX_NOTES_TOTAL)}，上限 {shown('notes_bytes', MAX_NOTES_TOTAL)}"
            f" 的 100%；清单 {shown('manifest_bytes', MAX_MANIFEST)}，"), both)

    def test_the_worst_case_result_fits_the_summary_file_read_back_by_finish(self):
        # finish 只读回 summary.json 的前 8193 字节；四条都满额、提醒最长时也要留足余量。
        full = report(usage(**LIMITS, visible_entries=LIMITS["entries"]))
        summary = {"source_digest": "a" * 64, "source_observed_at_ms": 2 ** 62, "files": 10 ** 6,
                   "bytes": 10 ** 12, "directories": 10 ** 6, "capacity": full,
                   "capacity_notice": warn_notice(full)[1], "capacity_alert": "unavailable",
                   "mode": "delta", "sent_bytes": 10 ** 12}
        self.assertLess(len(encode(summary)), 4096)

    def test_usage_is_counted_from_the_manifest_like_the_agent(self):
        entries = [{"path": "P/a.md", "kind": "file", "bytes": 10, "sha256": "x"},
                   {"path": "P/B.MD", "kind": "file", "bytes": 3, "sha256": "x"},
                   {"path": "P/c.md.txt", "kind": "file", "bytes": 7, "sha256": "x"},
                   {"path": "P/d.bin", "kind": "file", "bytes": 5, "sha256": "x"},
                   {"path": "P/dir.md", "kind": "dir", "bytes": 0, "sha256": None}]
        self.assertEqual(measure(b"0123456789", entries, inspected=9), {
            "manifest_bytes": 10, "entries": 9, "visible_entries": 5, "total_bytes": 25, "notes_bytes": 13})
        # 条目按连隐藏项一起数的那个数提醒；可见条目另报。
        capacity = report(measure(b"", entries, inspected=at(LIMITS["entries"], CAPACITY_WARN_PERCENT)))
        self.assertEqual(capacity["entries"]["visible"], 5)
        self.assertEqual(levels(capacity)["entries"], "warn")
        self.assertIn("条目（含隐藏项）", warn_notice(capacity)[1])

    def test_crossing_a_limit_names_it_and_stays_a_validation_failure(self):
        for key, name, _ in GAUGES:
            with self.subTest(gauge=key):
                within(key, LIMITS[key], LIMITS[key])
                with self.assertRaises(LimitExceeded) as caught:
                    within(key, LIMITS[key] + 1, LIMITS[key])
                self.assertEqual(caught.exception.gauge, key)
                self.assertIsInstance(caught.exception, Invalid)
                self.assertEqual(str(caught.exception), "validation_failed")
                title, body = exceeded_notice(key)
                self.assertEqual(title, "Studiolo 副本超上限，同步已停")
                self.assertTrue(body.startswith(f"{name}超过上限 {shown(key, LIMITS[key])}，Mac 同步整批失败"), body)
                for words in ("BUG-039", "agent 先部署、Mac 同步包紧跟", "精简白名单"):
                    self.assertIn(words, body)

    def test_reminder_intervals_are_a_week_at_the_line_and_a_day_over_it(self):
        # docs/m3/replica-operations.md「T-217」写明的间隔；改这里要连文档一起改。
        self.assertEqual(REMIND_AFTER_MS, {"warn": 7 * DAY_MS, "exceeded": DAY_MS})
        self.assertEqual(DAY_MS, 86_400_000)

    def test_a_line_notifies_once_then_again_only_after_its_reminder_interval(self):
        now = 10 * DAY_MS
        due, state = plan({}, {"notes_bytes": "warn"}, now)
        self.assertTrue(due)
        self.assertEqual(state, {"notes_bytes": {"level": "warn", "notified_at_ms": now}})
        for later in (now + 1, now + REMIND_AFTER_MS["warn"] - 1):
            self.assertEqual(plan(state, {"notes_bytes": "warn"}, later), (False, state))
        due, again = plan(state, {"notes_bytes": "warn"}, now + REMIND_AFTER_MS["warn"])
        self.assertTrue(due)
        self.assertEqual(again["notes_bytes"]["notified_at_ms"], now + REMIND_AFTER_MS["warn"])
        # 越线每天再提醒一次。
        exceeded = {"notes_bytes": {"level": "exceeded", "notified_at_ms": now}}
        self.assertFalse(plan(exceeded, {"notes_bytes": "exceeded"}, now + DAY_MS - 1)[0])
        self.assertTrue(plan(exceeded, {"notes_bytes": "exceeded"}, now + DAY_MS)[0])
        self.assertLess(REMIND_AFTER_MS["exceeded"], REMIND_AFTER_MS["warn"])

    def test_escalation_clock_rollback_and_new_lines_notify_at_once(self):
        now = 10 * DAY_MS
        warned = {"notes_bytes": {"level": "warn", "notified_at_ms": now}}
        self.assertEqual(plan(warned, {"notes_bytes": "exceeded"}, now + 1),
                         (True, {"notes_bytes": {"level": "exceeded", "notified_at_ms": now + 1}}))
        self.assertTrue(plan(warned, {"notes_bytes": "warn"}, now - 1)[0], "时钟往回拨算满")
        # 另一条新到线：马上发，这一轮到线的每一条都记成现在。
        due, state = plan(warned, {"notes_bytes": "warn", "total_bytes": "warn", "entries": None}, now + 1)
        self.assertTrue(due)
        self.assertEqual(state, {"notes_bytes": {"level": "warn", "notified_at_ms": now + 1},
                                 "total_bytes": {"level": "warn", "notified_at_ms": now + 1}})

    def test_falling_back_forgets_or_downgrades_without_notifying(self):
        now = 10 * DAY_MS
        state = {"notes_bytes": {"level": "exceeded", "notified_at_ms": now},
                 "entries": {"level": "warn", "notified_at_ms": now}}
        # 调高上限后从越线回到到线：不发，级别换成到线、提醒时间照旧；退到线下的那条删掉。
        self.assertEqual(plan(state, {"notes_bytes": "warn", "entries": None}, now + 1),
                         (False, {"notes_bytes": {"level": "warn", "notified_at_ms": now}}))
        self.assertEqual(plan(state, {key: None for key in LIMITS}, now + 1), (False, {}))

    def test_forgetting_drops_only_the_lines_seen_below_the_threshold(self):
        state = {"notes_bytes": {"level": "warn", "notified_at_ms": 1},
                 "total_bytes": {"level": "warn", "notified_at_ms": 2},
                 "entries": {"level": "exceeded", "notified_at_ms": 3}}
        self.assertEqual(forget(state, {"notes_bytes": None, "total_bytes": "warn"}),
                         {"total_bytes": state["total_bytes"], "entries": state["entries"]})
        self.assertEqual(forget(state, {}), state)

    def test_a_capture_stopped_at_one_limit_leaves_the_others_as_they_were(self):
        now = 10 * DAY_MS
        state = {"total_bytes": {"level": "warn", "notified_at_ms": now - 1}}
        due, after = plan(state, {"notes_bytes": "exceeded"}, now)
        self.assertTrue(due)
        self.assertEqual(after, {"total_bytes": {"level": "warn", "notified_at_ms": now - 1},
                                 "notes_bytes": {"level": "exceeded", "notified_at_ms": now}})

    def test_state_round_trips_and_malformed_state_is_rejected(self):
        gauges = {"notes_bytes": {"level": "warn", "notified_at_ms": 5},
                  "entries": {"level": "exceeded", "notified_at_ms": 0}}
        self.assertEqual(parse_state(json.loads(encode(dump_state(gauges)))), gauges)
        self.assertEqual(parse_state(dump_state({})), {})
        for bad in (None, [], {"schema_version": 2, "gauges": {}}, {"schema_version": True, "gauges": {}},
                    {"schema_version": 1}, {"schema_version": 1, "gauges": {}, "extra": 1},
                    {"schema_version": 1, "gauges": []},
                    dump_state({"unknown": {"level": "warn", "notified_at_ms": 1}}),
                    dump_state({"entries": {"level": "loud", "notified_at_ms": 1}}),
                    dump_state({"entries": {"level": "warn", "notified_at_ms": -1}}),
                    dump_state({"entries": {"level": "warn", "notified_at_ms": True}}),
                    dump_state({"entries": {"level": "warn", "notified_at_ms": 1.5}}),
                    dump_state({"entries": {"level": "warn"}}),
                    dump_state({"entries": {"level": "warn", "notified_at_ms": 1, "extra": 1}})):
            with self.subTest(state=bad), self.assertRaises(Invalid):
                parse_state(bad)
        self.assertEqual(LEVELS, ("warn", "exceeded"))


class Sender:
    """发通知的替身：记下每次调用，返回事先给定的结果。"""

    def __init__(self, outcome="sent"):
        self.outcome, self.calls = outcome, []

    def __call__(self, title, body):
        self.calls.append((title, body))
        return self.outcome


class CapacityAlertTests(unittest.TestCase):
    NOTICE = ("标题", "正文")
    NOW = 10 * DAY_MS

    def setUp(self):
        self.addCleanup(os.umask, os.umask(0o022))
        temp = tempfile.TemporaryDirectory(dir="/private/tmp" if sys.platform == "darwin" else None)
        self.addCleanup(temp.cleanup)
        self.dir = Path(temp.name) / "state"
        self.fd = open_root(str(self.dir), create=True, private=True)
        self.addCleanup(os.close, self.fd)
        self.file = self.dir / "capacity-test.json"

    def alert(self, observed, notice=NOTICE, now=NOW, dry_run=False, send=None):
        return replica_client.alert(self.fd, self.file.name, observed, notice, now, dry_run,
                                    send=send or Sender())

    def saved(self):
        return parse_state(json.loads(self.file.read_bytes()))

    def test_nothing_reached_sends_and_writes_nothing(self):
        sender = Sender()
        self.assertIsNone(self.alert({key: None for key in LIMITS}, notice=None, send=sender))
        self.assertEqual(sender.calls, [])
        self.assertFalse(self.file.exists())

    def test_dry_run_only_says_whether_it_would_send(self):
        sender = Sender()
        for _ in range(2):
            self.assertEqual(self.alert({"notes_bytes": "warn"}, dry_run=True, send=sender), "would_send")
        self.assertEqual(sender.calls, [])
        self.assertFalse(self.file.exists())
        self.alert({"notes_bytes": "warn"})
        before = self.file.read_bytes()
        self.assertEqual(self.alert({"notes_bytes": "warn"}, now=self.NOW + 1, dry_run=True, send=sender),
                         "suppressed")
        self.assertEqual(self.alert({"notes_bytes": None}, notice=None, dry_run=True, send=sender), None)
        self.assertEqual(self.file.read_bytes(), before, "预演不改去重状态")
        self.assertEqual(sender.calls, [])

    def test_sends_once_and_stays_quiet_until_the_reminder_is_due(self):
        sender = Sender()
        self.assertEqual(self.alert({"notes_bytes": "warn"}, send=sender), "sent")
        self.assertEqual(sender.calls, [self.NOTICE])
        self.assertEqual(self.file.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.saved(), {"notes_bytes": {"level": "warn", "notified_at_ms": self.NOW}})
        self.assertEqual(self.alert({"notes_bytes": "warn"}, now=self.NOW + 1, send=sender), "suppressed")
        self.assertEqual(len(sender.calls), 1)
        later = self.NOW + REMIND_AFTER_MS["warn"]
        self.assertEqual(self.alert({"notes_bytes": "warn"}, now=later, send=sender), "sent")
        self.assertEqual(len(sender.calls), 2)
        # 退到线下就忘掉，再到线马上提醒。
        self.assertIsNone(self.alert({"notes_bytes": None}, notice=None, now=later + 1, send=sender))
        self.assertEqual(self.saved(), {})
        self.assertEqual(self.alert({"notes_bytes": "warn"}, now=later + 2, send=sender), "sent")

    def test_an_unsent_notification_is_not_remembered_and_is_retried(self):
        for outcome in ("failed", "unavailable"):
            with self.subTest(outcome=outcome):
                self.assertEqual(self.alert({"notes_bytes": "warn"}, send=Sender(outcome)), outcome)
                self.assertFalse(self.file.exists())
        self.assertEqual(self.alert({"notes_bytes": "warn"}), "sent")

    def test_an_unsent_notification_still_forgets_lines_that_fell_back(self):
        # Markdown 昨天提醒过、今天退到线下；条目今天新到线但通知没发出去。退线那条照样要忘掉，
        # 否则 Markdown 明天再到线会被当成「刚提醒过」压掉（Codex 第二意见指出）。
        self.alert({"notes_bytes": "warn"})
        today = self.NOW + DAY_MS
        self.assertEqual(self.alert({"notes_bytes": None, "entries": "warn"}, now=today, send=Sender("failed")),
                         "failed")
        self.assertEqual(self.saved(), {})
        sender = Sender()
        self.assertEqual(self.alert({"notes_bytes": "warn", "entries": None}, now=today + DAY_MS, send=sender),
                         "sent")
        self.assertEqual(len(sender.calls), 1)

    def test_unreadable_state_counts_as_never_notified(self):
        deep = b"[" * 2000 + b"0" + b"]" * 2000  # 放得进 4096 字节，嵌套深到 json 解析递归超限
        for raw in (b"not json", b'{"schema_version":1,"gauges":{},"schema_version":1}', b"\xff", deep,
                    encode({"schema_version": 1, "gauges": {"notes_bytes": {"level": "loud"}}})):
            with self.subTest(raw=raw):
                self.file.unlink(missing_ok=True)
                self.file.write_bytes(raw)
                self.file.chmod(0o600)
                self.assertEqual(self.alert({"notes_bytes": "warn"}), "sent")
                self.assertEqual(self.saved(), {"notes_bytes": {"level": "warn", "notified_at_ms": self.NOW}})

    def test_a_recursion_error_while_parsing_counts_as_never_notified(self):
        self.alert({"notes_bytes": "warn"})
        with patch("replica_client.load_json", side_effect=RecursionError):
            self.assertEqual(self.alert({"notes_bytes": "warn"}, now=self.NOW + 1), "sent")

    def test_unsafe_state_file_fails_closed(self):
        self.alert({"notes_bytes": "warn"})
        self.file.chmod(0o644)
        with self.assertRaises(Invalid):
            self.alert({"notes_bytes": "warn"}, now=self.NOW + REMIND_AFTER_MS["warn"])
        self.file.unlink()
        self.file.symlink_to(self.dir / "elsewhere.json")
        with self.assertRaises(OSError):
            self.alert({"notes_bytes": "warn"})


class NotifyTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(dir="/private/tmp" if sys.platform == "darwin" else None)
        self.addCleanup(temp.cleanup)
        self.bin = Path(temp.name)
        self.log = self.bin / "argv.json"

    def stub(self, body):
        # PATH 只剩替身目录，解释器写绝对路径（/usr/bin/env python3 在这里找不到 python3）。
        program = self.bin / "osascript"
        program.write_text(f"#!{sys.executable}\nimport json, sys, time\n" + textwrap.dedent(body))
        program.chmod(0o755)

    def notify(self, title, body):
        with patch.dict(os.environ, {"PATH": str(self.bin)}):
            return replica_client.notify(title, body)

    def test_title_and_body_are_arguments_never_part_of_the_script(self):
        self.stub(f"open({str(self.log)!r}, 'w').write(json.dumps(sys.argv[1:]))\n")
        sneaky = '" & (do shell script "touch /tmp/pwned") & "'
        self.assertEqual(self.notify("Studiolo 副本快到上限", sneaky), "sent")
        script = [part for line in replica_client.NOTIFY_SCRIPT for part in ("-e", line)]
        self.assertEqual(json.loads(self.log.read_text()), [*script, "Studiolo 副本快到上限", sneaky])
        self.assertEqual(replica_client.NOTIFY_SCRIPT, (
            "on run argv", "display notification (item 2 of argv) with title (item 1 of argv)", "end run"))

    def test_failure_timeout_and_absence_are_reported_not_raised(self):
        self.stub("sys.exit(1)\n")
        self.assertEqual(self.notify("t", "b"), "failed")
        self.stub("time.sleep(5)\n")
        with patch("replica_client.NOTIFY_TIMEOUT_SECS", 0.2):
            self.assertEqual(self.notify("t", "b"), "failed")
        (self.bin / "osascript").unlink()
        self.assertEqual(self.notify("t", "b"), "unavailable")


class CapacitySyncTests(unittest.TestCase):
    """经入口 sync-agent-vault.sh 跑完整一轮；夹具与 SSH、osascript 替身同 test-sync-agent-vault.py。"""
    setUp = SYNC.SyncAgentVaultTests.setUp
    run_sync = SYNC.SyncAgentVaultTests.run_sync
    write_ssh_stub = SYNC.SyncAgentVaultTests.write_ssh_stub
    last_status = SYNC.SyncAgentVaultTests.last_status
    # 夹具有几十 MiB，双读加打包比默认 10 秒的预算慢。
    TIMEOUT = 120

    def fill_notes(self, total):
        """把白名单里的 Markdown 合计补到正好 total 字节，每篇不超过单篇上限。"""
        remaining, n = total - (self.project / "note.md").stat().st_size, 0
        while remaining > 0:
            size = min(remaining, MAX_MANIFEST)
            (self.project / f"filler-{n:03}.md").write_bytes(b"a" * size)
            remaining, n = remaining - size, n + 1

    def ready_receipt_ssh(self):
        """回一张合格的回执，不落盘：经替身落盘受 ulimit -f 16 所限，几十 MiB 的正文过不了真接收器。"""
        self.write_ssh_stub("""
            import hashlib, io, json, sys, tarfile
            with tarfile.open(fileobj=io.BytesIO(sys.stdin.buffer.read())) as tar:
                manifest = tar.extractfile("manifest.json").read()
                observed = json.loads(tar.extractfile("request.json").read())["source_observed_at_ms"]
            print(json.dumps({"status": "ready", "reused": False, "receipt": {
                "generation": "a" * 32, "source_digest": hashlib.sha256(manifest).hexdigest(),
                "source_observed_at_ms": observed, "received_at_ms": observed}}))
        """)

    def notifications(self):
        if not self.notify_log.exists():
            return []
        return [json.loads(line) for line in self.notify_log.read_text(encoding="utf-8").splitlines()]

    def alert_files(self):
        return sorted(self.state.glob("capacity-*.json")) if self.state.exists() else []

    def test_dry_run_previews_the_notification_without_sending_or_remembering(self):
        self.fill_notes(at(MAX_NOTES_TOTAL, CAPACITY_WARN_PERCENT))
        for _ in range(2):
            value = json.loads(self.run_sync(["--dry-run"], timeout=self.TIMEOUT).stdout)
            self.assertEqual(value["status"], "dry_run")
            self.assertEqual(value["capacity"]["notes_bytes"]["percent"], CAPACITY_WARN_PERCENT)
            self.assertEqual(value["capacity_alert"], "would_send")
            self.assertTrue(value["capacity_notice"].startswith("Markdown 合计 "), value["capacity_notice"])
        self.assertEqual(self.notifications(), [])
        self.assertEqual(self.alert_files(), [])
        self.assertFalse(self.log.exists())

    def test_reaching_the_line_notifies_once_and_later_rounds_stay_quiet(self):
        self.ready_receipt_ssh()
        self.fill_notes(at(MAX_NOTES_TOTAL, CAPACITY_WARN_PERCENT))
        first = json.loads(self.run_sync(timeout=self.TIMEOUT).stdout)
        self.assertEqual((first["status"], first["capacity_alert"]), ("ready", "sent"))
        self.assertEqual(first["capacity"]["notes_bytes"]["used"], at(MAX_NOTES_TOTAL, CAPACITY_WARN_PERCENT))
        sent = self.notifications()
        self.assertEqual(len(sent), 1)
        script = [part for line in replica_client.NOTIFY_SCRIPT for part in ("-e", line)]
        self.assertEqual(sent[0], [*script, "Studiolo 副本快到上限", first["capacity_notice"]])
        self.assertEqual([path.stat().st_mode & 0o777 for path in self.alert_files()], [0o600])
        second = json.loads(self.run_sync(timeout=self.TIMEOUT).stdout)
        self.assertEqual((second["status"], second["mode"], second["capacity_alert"]),
                         ("ready", "confirm", "suppressed"))
        preview = json.loads(self.run_sync(["--dry-run"], timeout=self.TIMEOUT).stdout)
        self.assertEqual(preview["capacity_alert"], "suppressed")
        self.assertEqual(len(self.notifications()), 1)

    def test_a_failed_notification_does_not_fail_the_sync(self):
        self.ready_receipt_ssh()
        self.fill_notes(at(MAX_NOTES_TOTAL, CAPACITY_WARN_PERCENT))
        self.env["STUDIOLO_TEST_NOTIFY_EXIT"] = "1"
        value = json.loads(self.run_sync(timeout=self.TIMEOUT).stdout)
        self.assertEqual((value["status"], value["capacity_alert"]), ("ready", "failed"))
        self.assertEqual(self.alert_files(), [])
        self.env["STUDIOLO_TEST_NOTIFY_EXIT"] = "0"
        self.assertEqual(json.loads(self.run_sync(timeout=self.TIMEOUT).stdout)["capacity_alert"], "sent")
        self.assertEqual(len(self.notifications()), 2)

    def test_crossing_a_limit_stops_the_batch_names_the_limit_and_notifies(self):
        self.fill_notes(MAX_NOTES_TOTAL + 1)
        preview = self.last_status(self.run_sync(["--dry-run"], code=1, timeout=self.TIMEOUT))
        self.assertEqual(preview["capacity_alert"], "would_send")
        self.assertEqual(self.notifications(), [])
        status = self.last_status(self.run_sync(code=1, timeout=self.TIMEOUT))
        title, body = exceeded_notice("notes_bytes")
        self.assertEqual(status, {"status": "failed", "last_error": "capacity_exceeded", "exceeded": "notes_bytes",
                                  "capacity_notice": body, "capacity_alert": "sent"})
        self.assertEqual([sent[-2:] for sent in self.notifications()], [[title, body]])
        self.assertFalse(self.log.exists(), "越线时不连 SSH")
        again = self.last_status(self.run_sync(code=1, timeout=self.TIMEOUT))
        self.assertEqual(again["capacity_alert"], "suppressed")
        self.assertEqual(len(self.notifications()), 1)

    def test_crossing_a_limit_is_still_named_when_the_alert_state_cannot_be_used(self):
        # 去重状态文件不安全：越线的原因和点名照旧，不退回笼统的 validation_failed；通知记成 state_error。
        self.fill_notes(MAX_NOTES_TOTAL + 1)
        first = self.last_status(self.run_sync(code=1, timeout=self.TIMEOUT))
        self.assertEqual(first["capacity_alert"], "sent")
        (state_file,) = self.alert_files()
        state_file.chmod(0o644)
        _, body = exceeded_notice("notes_bytes")
        for args in ([], ["--dry-run"]):
            with self.subTest(args=args):
                status = self.last_status(self.run_sync(args, code=1, timeout=self.TIMEOUT))
                self.assertEqual(status, {"status": "failed", "last_error": "capacity_exceeded",
                                          "exceeded": "notes_bytes", "capacity_notice": body,
                                          "capacity_alert": "state_error"})
        self.assertEqual(len(self.notifications()), 1, "状态用不了就没法去重，这两轮都不发")
        # 对照：文件改回私有的，同一份状态照常去重——上面的 state_error 确实出在文件不安全上。
        state_file.chmod(0o600)
        again = self.last_status(self.run_sync(code=1, timeout=self.TIMEOUT))
        self.assertEqual(again["capacity_alert"], "suppressed")


if __name__ == "__main__":
    unittest.main()
