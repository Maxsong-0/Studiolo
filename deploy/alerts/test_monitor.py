"""告警状态与私有落盘契约；外部服务全部用本地假对象，无真实投递。"""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import resource
import subprocess
import sys

SPEC = importlib.util.spec_from_file_location("alert_run", Path(__file__).with_name("run.py"))
RUN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUN)
STATE = RUN.sys.modules["watch_state"]
IO = RUN.sys.modules["private_state"]
HB = RUN.sys.modules["heartbeat"]
PROBE = RUN.sys.modules["probe"]
BOUNDED = RUN.sys.modules["bounded_process"]
GATE_SPEC = importlib.util.spec_from_file_location("time_gate", Path(__file__).with_name("time_gate.py"))
GATE = importlib.util.module_from_spec(GATE_SPEC)
GATE_SPEC.loader.exec_module(GATE)


class MonitorTests(unittest.TestCase):
    def test_two_failures_delivery_cooldown_and_new_episode(self):
        state, fire = STATE.observe(STATE.initial(100), "A02", 100)
        self.assertFalse(fire)
        state, fire = STATE.observe(state, "A02", 160)
        self.assertTrue(fire)
        state["sent"] = 160
        state, fire = STATE.observe(state, "A02", 759)
        self.assertFalse(fire)
        state, fire = STATE.observe(state, "A02", 760)
        self.assertTrue(fire)
        prior_episode = state["episode"]
        state, fire = STATE.observe(state, None, 761)
        self.assertFalse(fire)
        state, fire = STATE.observe(state, "A02", 762)
        self.assertFalse(fire)
        state, fire = STATE.observe(state, "A02", 822)
        self.assertTrue(fire)
        self.assertGreater(state["episode"], prior_episode)

    def test_clock_rollback_fires_even_inside_cooldown(self):
        state, _ = STATE.observe(STATE.initial(1000), "A01", 1000)
        state, _ = STATE.observe(state, "A01", 1060)
        state["sent"] = 1060
        state, fire = STATE.observe(state, None, 900)
        self.assertTrue(fire)
        self.assertEqual(state["code"], "A06")
        self.assertIsNone(state["sent"])

    def test_failed_delivery_retries_after_restart_and_round_is_saved_first(self):
        with tempfile.TemporaryDirectory() as directory:
            with IO.locked_state(directory) as state:
                self.assertTrue(RUN.tick(state, "A01", 100, lambda value: self.fail("不应发送")))
                def failed(value):
                    self.assertEqual(state.load()["round"], value["round"])
                    return False
                self.assertFalse(RUN.tick(state, "A01", 160, failed))
            with IO.locked_state(directory) as state:
                old_round = state.load()["round"]
                self.assertTrue(RUN.tick(state, "A01", 220, lambda value: value["round"] == old_round + 1))
                self.assertEqual(state.load()["sent"], 220)

    def test_unsafe_or_corrupt_state_is_never_reset_to_healthy(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory, "state.json")
            target.write_text("not-json")
            target.chmod(0o600)
            with IO.locked_state(directory) as state:
                with self.assertRaises(ValueError):
                    RUN.tick(state, None, 100, lambda value: True)
            target.chmod(0o644)
            with IO.locked_state(directory) as state:
                with self.assertRaises(ValueError):
                    state.load()
            target.unlink()
            target.symlink_to("/etc/passwd")
            with IO.locked_state(directory) as state:
                with self.assertRaises(OSError):
                    state.load()

    def test_lock_excludes_concurrent_watchers(self):
        with tempfile.TemporaryDirectory() as directory:
            with IO.locked_state(directory):
                with self.assertRaises(BlockingIOError):
                    with IO.locked_state(directory):
                        self.fail("第二个写者不应获得锁")

    def test_duplicate_json_and_boolean_schema_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory, "state.json")
            target.write_text('{"schema":1,"schema":2}')
            target.chmod(0o600)
            with IO.locked_state(directory) as state:
                with self.assertRaises(ValueError): state.load()
        value = STATE.initial(100)
        value["schema"] = True
        with self.assertRaises(ValueError): STATE.validate(value)

    def test_probe_receipt_must_be_current_boot_and_fresh(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(PROBE.time, "time", return_value=1000):
            result = dict(services_ok=True, clock_ok=True, mcp_ok=True, replica_stale=False, memory_high=False,
                          sync_unaligned=False, sync_deletes_blocked=False)
            PROBE.save_receipt(result, directory)
            self.assertEqual(PROBE.read_receipt(directory), result)
            with patch.object(PROBE.time, "time", return_value=1121):
                with self.assertRaises(ValueError): PROBE.read_receipt(directory)
            with patch.object(PROBE, "boot_id", return_value="different-boot"):
                with self.assertRaises(ValueError): PROBE.read_receipt(directory)
            with patch.object(PROBE.time, "time", return_value=999):
                with self.assertRaises(ValueError): PROBE.read_receipt(directory)

    def test_bounded_commands_reject_large_and_hanging_output(self):
        command = [sys.executable, "-I", "-B", "-S", "-c"]
        self.assertEqual(BOUNDED.output(command + ["print('ok')"], 2), b"ok\n")
        with self.assertRaises(ValueError):
            BOUNDED.output(command + ["print('x' * 10000)"], 2)
        with self.assertRaises(TimeoutError):
            BOUNDED.output(command + ["import time; time.sleep(10)"], 0.1)

    def test_time_gate_waits_for_real_synchronization_and_times_out(self):
        now = [0]
        def sleep(seconds): now[0] += seconds
        self.assertEqual(GATE.wait_for_sync(lambda: "yes" if now[0] == 3 else "no", lambda: now[0], sleep, 5), 3)
        with self.assertRaises(TimeoutError): GATE.wait_for_sync(lambda: "no", lambda: now[0], sleep, 5)

    def test_residency_is_real_and_low_memlock_cannot_reach_secret_read(self):
        source = Path(__file__).with_name("residency.py").resolve()
        program = "import importlib.util,sys; s=importlib.util.spec_from_file_location('residency',sys.argv[1]); m=importlib.util.module_from_spec(s); s.loader.exec_module(m); m.harden(); print('protected')"
        command = [sys.executable, "-I", "-B", "-S", "-c", program, str(source)]
        success = subprocess.run(command, capture_output=True, timeout=5)
        self.assertEqual(success.returncode, 0, success.stderr.decode())
        self.assertEqual(success.stdout, b"protected\n")
        self.assertIn(b"not_dumpable", success.stderr)
        failure = subprocess.run(command, capture_output=True, timeout=5,
                                 preexec_fn=lambda: resource.setrlimit(resource.RLIMIT_MEMLOCK, (0, 0)))
        self.assertNotEqual(failure.returncode, 0)
        self.assertNotIn(b"protected", failure.stdout)

    def test_unknown_probe_is_not_healthy_and_priority_is_explicit(self):
        self.assertEqual(STATE.failure_code({}), "A01")
        probe = dict(services_ok=True, clock_ok=True, mcp_ok=True, replica_stale=False, memory_high=False,
                     sync_unaligned=False, sync_deletes_blocked=False)
        self.assertIsNone(STATE.failure_code(probe))
        # 从最低一级往上逐个打开：每打开一项，代号都换成更高的那一个（A08 盖过 A07，A04 盖过 A08）。
        for field, code in (("sync_unaligned", "A07"), ("sync_deletes_blocked", "A08"), ("memory_high", "A04"),
                            ("replica_stale", "A03"), ("mcp_ok", "A02"), ("clock_ok", "A06")):
            probe[field] = not probe[field]
            self.assertEqual(STATE.failure_code(probe), code)
        # 旧包的 5 字段回执、多字段、非布尔值都不能当成正常。
        old = dict(services_ok=True, clock_ok=True, mcp_ok=True, replica_stale=False, memory_high=False)
        self.assertEqual(STATE.failure_code(old), "A01")
        self.assertEqual(STATE.failure_code(dict(old, sync_unaligned=False, sync_deletes_blocked=False, extra=False)), "A01")
        self.assertEqual(STATE.failure_code(dict(old, sync_unaligned=0, sync_deletes_blocked=False)), "A01")

    def test_sync_codes_repeat_daily_while_others_keep_ten_minutes(self):
        for code, repeat in (("A07", 86400), ("A08", 86400), ("A03", 600), ("A02", 600)):
            state, fire = STATE.observe(STATE.initial(100), code, 100)
            self.assertFalse(fire)
            state, fire = STATE.observe(state, code, 160)
            self.assertTrue(fire, code)
            state["sent"] = 160
            state, fire = STATE.observe(state, code, 160 + repeat - 1)
            self.assertFalse(fire, code)
            state, fire = STATE.observe(state, code, 160 + repeat)
            self.assertTrue(fire, code)
            self.assertEqual(STATE.CODES[code], "critical" if code == "A02" else "warning")
        # 删除被挡转成没对上（例如人侧卡住、回执不再新鲜）是新情节：再连续两次就报，不受上一条的一天冷却。
        state, _ = STATE.observe(STATE.initial(100), "A08", 100)
        state, fire = STATE.observe(state, "A08", 160)
        self.assertTrue(fire)
        state["sent"] = 160
        episode = state["episode"]
        state, fire = STATE.observe(state, "A07", 220)
        self.assertFalse(fire)
        state, fire = STATE.observe(state, "A07", 280)
        self.assertTrue(fire)
        self.assertGreater(state["episode"], episode)

    def write_status(self, directory, raw, mode=0o600):
        target = Path(directory, "sync-status.json")
        target.write_bytes(raw)
        target.chmod(mode)
        return str(target)

    def test_sync_status_reader_accepts_only_the_closed_private_receipt(self):
        good = b'{"schema":1,"written_at":1000,"last_aligned_at":900,"deletes_blocked":1309}'
        with tempfile.TemporaryDirectory() as directory:
            path = self.write_status(directory, good)
            self.assertEqual(PROBE.read_sync_status(path), {"schema": 1, "written_at": 1000, "last_aligned_at": 900, "deletes_blocked": 1309})
            nulls = b'{"schema":1,"written_at":1000,"last_aligned_at":null,"deletes_blocked":null}'
            self.assertIsNone(PROBE.read_sync_status(self.write_status(directory, nulls))["last_aligned_at"])
            for raw in (
                b'{"schema":1,"written_at":1000,"last_aligned_at":900}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":900,"deletes_blocked":0,"extra":1}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":900,"last_aligned_at":901,"deletes_blocked":0}',
                b'{"schema":2,"written_at":1000,"last_aligned_at":900,"deletes_blocked":0}',
                b'{"schema":true,"written_at":1000,"last_aligned_at":900,"deletes_blocked":0}',
                b'{"schema":1,"written_at":true,"last_aligned_at":900,"deletes_blocked":0}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":900.0,"deletes_blocked":0}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":1001,"deletes_blocked":0}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":-1,"deletes_blocked":0}',
                b'{"schema":1,"written_at":-1,"last_aligned_at":null,"deletes_blocked":0}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":900,"deletes_blocked":-1}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":NaN,"deletes_blocked":0}',
                b'[1,1000,900,0]',
                b'',
                b' ' * 4097,
            ):
                with self.assertRaises(ValueError, msg=raw[:60]):
                    PROBE.read_sync_status(self.write_status(directory, raw))
            # 别人能读、是链接、不存在：都不能当成读到了。
            with self.assertRaises(ValueError):
                PROBE.read_sync_status(self.write_status(directory, good, 0o644))
            target = Path(directory, "sync-status.json")
            target.unlink()
            Path(directory, "elsewhere.json").write_bytes(good)
            Path(directory, "elsewhere.json").chmod(0o600)
            target.symlink_to(Path(directory, "elsewhere.json"))
            with self.assertRaises(OSError):
                PROBE.read_sync_status(str(target))
            target.unlink()
            with self.assertRaises(FileNotFoundError):
                PROBE.read_sync_status(str(target))

    def test_sync_flags_six_hours_and_fifteen_minute_freshness(self):
        now = 1_800_000_000
        def status(written, aligned, blocked):
            return {"schema": 1, "written_at": written, "last_aligned_at": aligned, "deletes_blocked": blocked}
        six = 6 * 3600
        self.assertEqual(PROBE.sync_flags(status(now, now - six + 1, 0), now), (False, False))
        self.assertEqual(PROBE.sync_flags(status(now, now - six, 0), now), (True, False))
        self.assertEqual(PROBE.sync_flags(status(now, None, None), now), (True, False))
        # 回执没写出来（读不到、坏了）或写出时刻在未来（时钟回拨）：按没对上报，不算被挡。
        self.assertEqual(PROBE.sync_flags(None, now), (True, False))
        self.assertEqual(PROBE.sync_flags(status(now + 1, now, 1309), now), (True, False))
        # 被挡只在 15 分钟内写过的回执里算数；0 和 null 不算被挡。
        self.assertEqual(PROBE.sync_flags(status(now - 900, now - 900, 1309), now), (False, True))
        self.assertEqual(PROBE.sync_flags(status(now - 901, now - 901, 1309), now), (False, False))
        self.assertEqual(PROBE.sync_flags(status(now, now - 7 * 3600, 1309), now), (True, True))
        self.assertEqual(PROBE.sync_flags(status(now, now, 0), now), (False, False))
        self.assertEqual(PROBE.sync_flags(status(now, now, None), now), (False, False))
        # 卡住的人侧：回执停在 20 分钟前、还记着被挡，最后对上在 7 小时前 → 不再报被挡，按没对上报。
        stuck = status(now - 1200, now - 7 * 3600, 1309)
        flags = PROBE.sync_flags(stuck, now)
        self.assertEqual(flags, (True, False))
        probe = dict(services_ok=True, clock_ok=True, mcp_ok=True, replica_stale=False, memory_high=False,
                     sync_unaligned=flags[0], sync_deletes_blocked=flags[1])
        self.assertEqual(STATE.failure_code(probe), "A07")

    def test_probe_folds_sync_status_in_and_fails_closed_when_unreadable(self):
        now = 1_800_000_000
        healthy = {"schema": 1, "written_at": now - 30, "last_aligned_at": now - 300, "deletes_blocked": 0}
        with patch.object(PROBE, "read_sync_status", return_value=healthy), patch.object(PROBE.time, "time", return_value=now):
            result = PROBE.probe()
        self.assertEqual(set(result), STATE.PROBE_FIELDS)
        self.assertEqual((result["sync_unaligned"], result["sync_deletes_blocked"]), (False, False))
        with patch.object(PROBE, "read_sync_status", side_effect=FileNotFoundError()), patch.object(PROBE.time, "time", return_value=now):
            result = PROBE.probe()
        self.assertEqual((result["sync_unaligned"], result["sync_deletes_blocked"]), (True, False))
        blocked = dict(healthy, deletes_blocked=1309, last_aligned_at=now - 8 * 3600)
        with patch.object(PROBE, "read_sync_status", return_value=blocked), patch.object(PROBE.time, "time", return_value=now):
            result = PROBE.probe()
        self.assertEqual((result["sync_unaligned"], result["sync_deletes_blocked"]), (True, True))

    def test_heartbeats_are_independent_empty_posts_and_bounded(self):
        environment = {"STUDIOLO_HEALTHCHECKS_URL": "https://hc-ping.com/" + "a" * 36,
                       "STUDIOLO_WORKER_HEARTBEAT_URL": "https://heartbeat.example.com/heartbeat/n108",
                       "STUDIOLO_WORKER_HEARTBEAT_TOKEN": "test-only"}
        class Reply:
            status = 204
            def __enter__(self): return self
            def __exit__(self, *args): return None
            def read(self, size):
                self.size = size
                return b""
        class Opener:
            def __init__(self): self.requests = []
            def open(self, request, timeout):
                self.requests.append(request)
                if "hc-ping.com" in request.full_url: raise TimeoutError()
                return Reply()
        opener = Opener()
        self.assertEqual(HB.ping("n108", environment, opener), [False, True])
        self.assertEqual(len(opener.requests), 2)
        self.assertTrue(all(request.get_method() == "POST" and request.data is None for request in opener.requests))
        environment["STUDIOLO_HEALTHCHECKS_URL"] = "http://bad.invalid/"
        self.assertEqual(HB.ping("n108", environment, Opener()), [False, True])


if __name__ == "__main__":
    unittest.main()
