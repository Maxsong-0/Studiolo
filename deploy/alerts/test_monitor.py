"""告警状态与私有落盘契约；外部服务全部用本地假对象，无真实投递。"""
import email.message
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import resource
import subprocess
import sys
import urllib.request
import urllib.response

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

MIB = 1024 * 1024
# 内存样本：169 上 2026-11-24 实测的 memory.current、memory.max 与 memory.stat（只留判据用到的项）。
# 封存中：memory.current 顶到 2 GiB 上限，干净文件缓存约 1713 MiB、可回收 slab 约 91 MiB，匿名 239 MiB。
SEAL_PEAK = dict(current=2147262464, max=2147483648, inactive_file=1797238784, active_file=4096,
                 file_dirty=28672, file_writeback=688128, slab_reclaimable=95129984)
# 同一天另一次封存最坏的一秒（21:21:48Z，暂存区落盘）：没落盘加正在回写 256 MiB，回收不掉的共 537 MiB。
SEAL_WORST_DIRTY = dict(current=2145849344, max=2147483648, inactive_file=1754763264, active_file=4096,
                        file_dirty=234356736, file_writeback=33554432, slab_reclaimable=95753384)
# 平时（两次封存之间）：约 1513 MiB，其中匿名 277 MiB。
IDLE_HUMAN = dict(current=1585332224, max=2147483648, inactive_file=1220456448, active_file=12288,
                  file_dirty=53248, file_writeback=0, slab_reclaimable=72839064)
IDLE_AGENT = dict(current=170434560, max=536870912, inactive_file=122880, active_file=4096,
                  file_dirty=0, file_writeback=0, slab_reclaimable=21576)
# BUG-054：2026-11-24 07:10:55Z 内核 OOM 杀人侧时打进日志的原始数（上限还是 512 MiB）；匿名 294293504 没列。
BUG_054_OOM = dict(current=536870912, max=536870912, inactive_file=211218432, active_file=0,
                   file_dirty=146845696, file_writeback=64364544, slab_reclaimable=27538240)
CALM = ("some avg10=0.00 avg60=0.03 avg300=0.00 total=5443847\n"
        "full avg10=0.00 avg60=0.03 avg300=0.00 total=5411689\n")


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
                     sync_unaligned=False, sync_deletes_blocked=False, human_restarted=False)
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
            state = STATE.delivered(state, 160)
            state, fire = STATE.observe(state, code, 160 + repeat - 1)
            self.assertFalse(fire, code)
            state, fire = STATE.observe(state, code, 160 + repeat)
            self.assertTrue(fire, code)
            self.assertEqual(STATE.CODES[code], "critical" if code == "A02" else "warning")
        # 只有按天提醒的代号另记发送时刻；别的代号只记本情节的冷却起点。
        state, _ = STATE.observe(STATE.initial(100), "A03", 100)
        state, _ = STATE.observe(state, "A03", 160)
        state = STATE.delivered(state, 160)
        self.assertEqual((state["sent"], state["daily_sent"]), (160, {"A07": None, "A08": None}))

    def fire_twice(self, state, code, now):
        state, first = STATE.observe(state, code, now)
        state, second = STATE.observe(state, code, now + 60)
        return state, first or second

    def test_daily_codes_keep_their_cooldown_across_code_changes_until_recovery(self):
        day = 86400
        state, fire = self.fire_twice(STATE.initial(100), "A07", 100)
        self.assertTrue(fire)
        state = STATE.delivered(state, 160)
        self.assertEqual(state["daily_sent"], {"A07": 160, "A08": None})
        # 被更高优先级的 A04 打断两分钟（它照自己的 600 秒规矩报），回到 A07：一天内不再发（复查 F1）。
        state, fire = self.fire_twice(state, "A04", 220)
        self.assertTrue(fire)
        state = STATE.delivered(state, 280)
        state, fire = self.fire_twice(state, "A07", 340)
        self.assertFalse(fire)
        # A07 换成 A08：A08 今天还没发过，报一次；再来回换，两边都不再发。
        state, fire = self.fire_twice(state, "A08", 460)
        self.assertTrue(fire)
        state = STATE.delivered(state, 520)
        state, fire = self.fire_twice(state, "A07", 580)
        self.assertFalse(fire)
        state, fire = self.fire_twice(state, "A08", 700)
        self.assertFalse(fire)
        # A07 上次发送满一天、还没恢复：照常再发（差 1 秒不发）。
        state, fire = STATE.observe(state, "A07", 160 + day - 60)
        self.assertFalse(fire)
        state, fire = STATE.observe(state, "A07", 160 + day - 1)
        self.assertFalse(fire)
        state, fire = STATE.observe(state, "A07", 160 + day)
        self.assertTrue(fire)
        state = STATE.delivered(state, 160 + day)
        # 看到正常就清：之后再出事是新的一条，连续两次就报。
        state, fire = STATE.observe(state, None, 160 + day + 60)
        self.assertFalse(fire)
        self.assertEqual(state["daily_sent"], {"A07": None, "A08": None})
        state, fire = self.fire_twice(state, "A07", 160 + day + 120)
        self.assertTrue(fire)
        # 时钟回拨也清（照旧立刻报 A06）：记着的时刻在「现在」之后，说不清隔了多久。
        state = STATE.delivered(state, 160 + day + 180)
        state, fire = STATE.observe(state, "A07", 160 + day + 100)
        self.assertTrue(fire)
        self.assertEqual((state["code"], state["daily_sent"]), ("A06", {"A07": None, "A08": None}))

    def test_watch_state_daily_record_is_closed_and_bounded(self):
        good = STATE.initial(1000)
        STATE.validate(good)
        STATE.validate(dict(good, daily_sent={"A07": 1000, "A08": 0}))
        for bad in (
            dict(good, schema=1),
            {key: value for key, value in good.items() if key != "daily_sent"},
            dict(good, daily_sent={"A07": None}),
            dict(good, daily_sent={"A07": None, "A08": None, "A03": None}),
            dict(good, daily_sent={"A07": True, "A08": None}),
            dict(good, daily_sent={"A07": 1001, "A08": None}),
            dict(good, daily_sent={"A07": -1, "A08": None}),
            dict(good, daily_sent=[None, None]),
        ):
            with self.assertRaises(ValueError, msg=repr(bad)):
                STATE.validate(bad)

    def test_tick_records_the_daily_send_for_sync_codes(self):
        with tempfile.TemporaryDirectory() as directory:
            with IO.locked_state(directory) as state:
                self.assertTrue(RUN.tick(state, "A08", 100, lambda value: self.fail("不应发送")))
                self.assertTrue(RUN.tick(state, "A08", 160, lambda value: True))
                saved = state.load()
                self.assertEqual((saved["sent"], saved["daily_sent"]), (160, {"A07": None, "A08": 160}))
                self.assertTrue(RUN.tick(state, "A07", 220, lambda value: self.fail("不应发送")))
                self.assertTrue(RUN.tick(state, "A08", 280, lambda value: self.fail("一天内不应再发")))
                self.assertTrue(RUN.tick(state, "A08", 340, lambda value: self.fail("一天内不应再发")))

    def write_status(self, directory, raw, mode=0o600):
        target = Path(directory, "sync-status.json")
        target.write_bytes(raw)
        target.chmod(mode)
        return str(target)

    def shared_example(self):
        # 两侧共用的回执样本：Rust 写出的字节钉在这个文件上（sync_plane_private/status.rs 编译期读它），
        # 这里读同一个文件，并核契约第 2 节的示例行和它逐字节相同。三处改一处另两处就红。
        root = Path(__file__).resolve().parents[2]
        sample = (root / "crates/local-server/tests/sync_plane_private/sync-status-example.json").read_bytes().rstrip()
        contract = (root / "docs/contracts/sync-status-receipt.md").read_text(encoding="utf-8")
        line = next(line for line in contract.splitlines() if line.startswith("例：`"))
        self.assertEqual(line[len("例：`"):].split("`")[0].encode(), sample, "契约示例行要和共用样本逐字节相同")
        return sample

    def test_sync_status_reader_accepts_only_the_closed_private_receipt(self):
        good = b'{"schema":1,"written_at":1000,"last_aligned_at":900,"deletes_blocked":1309,"deletes_checked_at":700}'
        with tempfile.TemporaryDirectory() as directory:
            path = self.write_status(directory, good)
            self.assertEqual(PROBE.read_sync_status(path), {"schema": 1, "written_at": 1000, "last_aligned_at": 900,
                                                            "deletes_blocked": 1309, "deletes_checked_at": 700})
            nulls = b'{"schema":1,"written_at":1000,"last_aligned_at":null,"deletes_blocked":null,"deletes_checked_at":null}'
            self.assertIsNone(PROBE.read_sync_status(self.write_status(directory, nulls))["last_aligned_at"])
            example = PROBE.read_sync_status(self.write_status(directory, self.shared_example()))
            self.assertEqual(set(example), PROBE.SYNC_STATUS_FIELDS)
            # 补空白到正好 4096 字节仍是合法回执；多一个字节就拒：两侧都钉住上限。
            padded = good[:1] + b" " * (PROBE.SYNC_STATUS_MAX_BYTES - len(good)) + good[1:]
            self.assertEqual(PROBE.read_sync_status(self.write_status(directory, padded))["written_at"], 1000)
            tail = ',"deletes_blocked":0,"deletes_checked_at":900}'
            for raw in (
                b'{"schema":1,"written_at":1000,"last_aligned_at":900,"deletes_blocked":0}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":900,"deletes_checked_at":900}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":900,"deletes_blocked":0,"deletes_checked_at":900,"extra":1}',
                ('{"schema":1,"written_at":1000,"last_aligned_at":900,"last_aligned_at":901' + tail).encode(),
                ('{"schema":2,"written_at":1000,"last_aligned_at":900' + tail).encode(),
                ('{"schema":true,"written_at":1000,"last_aligned_at":900' + tail).encode(),
                ('{"schema":1,"written_at":true,"last_aligned_at":900' + tail).encode(),
                ('{"schema":1,"written_at":1000,"last_aligned_at":900.0' + tail).encode(),
                ('{"schema":1,"written_at":1000,"last_aligned_at":1001' + tail).encode(),
                ('{"schema":1,"written_at":1000,"last_aligned_at":-1' + tail).encode(),
                b'{"schema":1,"written_at":-1,"last_aligned_at":null,"deletes_blocked":null,"deletes_checked_at":null}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":900,"deletes_blocked":-1,"deletes_checked_at":900}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":900,"deletes_blocked":true,"deletes_checked_at":900}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":900,"deletes_blocked":0,"deletes_checked_at":1001}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":900,"deletes_blocked":0,"deletes_checked_at":-1}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":900,"deletes_blocked":0,"deletes_checked_at":true}',
                # 被挡数和证明时刻不成对。
                b'{"schema":1,"written_at":1000,"last_aligned_at":900,"deletes_blocked":1309,"deletes_checked_at":null}',
                b'{"schema":1,"written_at":1000,"last_aligned_at":900,"deletes_blocked":null,"deletes_checked_at":900}',
                ('{"schema":1,"written_at":1000,"last_aligned_at":NaN' + tail).encode(),
                b'[1,1000,900,0,900]',
                b'',
                padded[:1] + b" " + padded[1:],
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
            # 父目录不是本人 0700（组可写、别人可读）、属主不是本人：文件本身合法也不读。
            path = self.write_status(directory, good)
            for mode in (0o770, 0o755):
                os.chmod(directory, mode)
                try:
                    with self.assertRaises(ValueError, msg=oct(mode)):
                        PROBE.read_sync_status(path)
                finally:
                    os.chmod(directory, 0o700)
            with patch.object(IO.os, "getuid", return_value=os.getuid() + 1):
                with self.assertRaises(ValueError):
                    PROBE.read_sync_status(path)
            self.assertEqual(PROBE.read_sync_status(path)["deletes_blocked"], 1309)

    def test_sync_flags_six_hours_and_one_hour_proof_freshness(self):
        now = 1_800_000_000
        def status(written, aligned, blocked, checked):
            return {"schema": 1, "written_at": written, "last_aligned_at": aligned,
                    "deletes_blocked": blocked, "deletes_checked_at": checked}
        six = 6 * 3600
        self.assertEqual(PROBE.sync_flags(status(now, now - six + 1, 0, now), now), (False, False))
        self.assertEqual(PROBE.sync_flags(status(now, now - six, 0, now), now), (True, False))
        self.assertEqual(PROBE.sync_flags(status(now, None, None, None), now), (True, False))
        # 回执没写出来（读不到、坏了）或写出时刻在未来（时钟回拨）：按没对上报，不算被挡。
        self.assertEqual(PROBE.sync_flags(None, now), (True, False))
        self.assertEqual(PROBE.sync_flags(status(now + 1, now, 1309, now), now), (True, False))
        # 被挡只在守卫 1 小时内证明过才算数；0 和 null 不算被挡。
        self.assertEqual(PROBE.sync_flags(status(now, now - 3600, 1309, now - 3600), now), (False, True))
        self.assertEqual(PROBE.sync_flags(status(now, now - 3601, 1309, now - 3601), now), (False, False))
        # 慢轮次：证明在轮末才换新，两轮各 10 分钟时换新前已经旧了 25 分钟，仍算被挡，不一轮一掉（R2-F1）。
        self.assertEqual(PROBE.sync_flags(status(now - 5, now - 3000, 1309, now - 1500), now), (False, True))
        self.assertEqual(PROBE.sync_flags(status(now, now - 7 * 3600, 1309, now - 300), now), (True, True))
        self.assertEqual(PROBE.sync_flags(status(now, now, 0, now), now), (False, False))
        self.assertEqual(PROBE.sync_flags(status(now, now, None, None), now), (False, False))
        # 人侧还在每轮写回执、但轮次走不到守卫（观察阶段就失败）：写出时刻是新的、证明是 2 小时前的，
        # 不再报被挡，按没对上报（复查 F5）。
        stale_proof = status(now - 30, now - 7 * 3600, 1309, now - 7200)
        self.assertEqual(PROBE.sync_flags(stale_proof, now), (True, False))
        # 卡住的人侧：回执停在 2 小时前、还记着被挡，最后对上在 7 小时前 → 同样按没对上报。
        stuck = status(now - 7200, now - 7 * 3600, 1309, now - 7200)
        flags = PROBE.sync_flags(stuck, now)
        self.assertEqual(flags, (True, False))
        probe = dict(services_ok=True, clock_ok=True, mcp_ok=True, replica_stale=False, memory_high=False,
                     sync_unaligned=flags[0], sync_deletes_blocked=flags[1], human_restarted=False)
        self.assertEqual(STATE.failure_code(probe), "A07")

    def test_probe_folds_sync_status_in_and_fails_closed_when_unreadable(self):
        now = 1_800_000_000
        healthy = {"schema": 1, "written_at": now - 30, "last_aligned_at": now - 300,
                   "deletes_blocked": 0, "deletes_checked_at": now - 300}
        with patch.object(PROBE, "read_sync_status", return_value=healthy), patch.object(PROBE.time, "time", return_value=now):
            result = PROBE.probe()
        self.assertEqual(set(result), STATE.PROBE_FIELDS)
        self.assertEqual((result["sync_unaligned"], result["sync_deletes_blocked"]), (False, False))
        # 读不到（缺失、权限、内容坏）一律按没对上报，不让整个探针回执失败变成别的代号。
        for error in (FileNotFoundError(), PermissionError(), ValueError("sync_status_invalid")):
            with patch.object(PROBE, "read_sync_status", side_effect=error), patch.object(PROBE.time, "time", return_value=now):
                result = PROBE.probe()
            self.assertEqual((result["sync_unaligned"], result["sync_deletes_blocked"]), (True, False), repr(error))
        blocked = dict(healthy, deletes_blocked=1309, last_aligned_at=now - 8 * 3600)
        with patch.object(PROBE, "read_sync_status", return_value=blocked), patch.object(PROBE.time, "time", return_value=now):
            result = PROBE.probe()
        self.assertEqual((result["sync_unaligned"], result["sync_deletes_blocked"]), (True, True))

    def memory_unit(self, root, unit, sample, pressure=CALM):
        # 摆一份单元的 cgroup 目录：sample 是 memory.stat 里判据用到的几项，加上 current、max。
        directory = Path(root, "system.slice", unit)
        directory.mkdir(parents=True, exist_ok=True)
        stat_lines = [f"{key} {value}" for key, value in sample.items() if key not in ("current", "max")]
        for name, text in (("memory.current", f"{sample['current']}\n"), ("memory.max", f"{sample['max']}\n"),
                           ("memory.stat", "\n".join(["anon 1", *stat_lines, "pgfault 7"]) + "\n"), ("memory.pressure", pressure)):
            Path(directory, name).write_text(text, encoding="utf-8")
        return directory

    def metrics(self, root, fields=None):
        # 假的 systemctl：只回探针要的属性，别的属性（比如改前要的 MemoryCurrent）从 cgroup 文件照抄。
        def systemctl(command):
            unit = command[2]
            directory = Path(root, "system.slice", unit)
            known = {"ActiveState": "active", "ControlGroup": f"/system.slice/{unit}"}
            for name, source in (("MemoryCurrent", "memory.current"), ("MemoryMax", "memory.max")):
                path = Path(directory, source)
                known[name] = path.read_bytes().decode("latin-1").strip() if path.exists() else "[not set]"
            known.update((fields or {}).get(unit, {}))
            wanted = command[3].removeprefix("--property=").split(",")
            return "\n".join(f"{key}={known[key]}" for key in wanted)
        with patch.object(PROBE, "output", systemctl), patch.object(PROBE, "CGROUPS", Path(root)):
            return PROBE.service_metrics()

    def human_metrics(self, sample, pressure=CALM):
        with tempfile.TemporaryDirectory() as root:
            for unit in PROBE.UNITS:
                self.memory_unit(root, unit, IDLE_AGENT)
            self.memory_unit(root, "studiolo-human.service", sample, pressure)
            return self.metrics(root)

    def test_memory_alert_ignores_reclaimable_cache_on_real_169_samples(self):
        # 封存把 memory.current 顶到 2 GiB，绝大部分是干净的文件缓存：不报。
        for sample in (SEAL_PEAK, SEAL_WORST_DIRTY, IDLE_HUMAN):
            self.assertEqual(self.human_metrics(sample), (True, False), sample)
        # BUG-054 内核杀人侧的那一刻：匿名加没落盘的页占满 512 MiB，必须报。
        self.assertEqual(self.human_metrics(BUG_054_OOM), (True, True))

    def test_memory_alert_fires_when_any_unreclaimable_kind_nears_the_limit(self):
        limit = 2000 * MIB
        edge = 1600 * MIB  # 正好 80%
        def sample(pinned, cache=0, dirty=0, writeback=0, slab=0):
            return dict(current=pinned + cache + slab, max=limit, inactive_file=cache + dirty + writeback, active_file=0,
                        file_dirty=dirty, file_writeback=writeback, slab_reclaimable=slab)
        # 匿名、共享内存、内核内存、套接字都不在可回收的那几项里，合在 current 里算。
        self.assertEqual(self.human_metrics(sample(edge)), (True, True))
        self.assertEqual(self.human_metrics(sample(edge - 1)), (True, False))
        # 剩下的 20% 全是干净缓存和可回收 slab 也照报；不到线的话，缓存占满也不报。
        self.assertEqual(self.human_metrics(sample(edge, cache=limit - edge - MIB, slab=MIB)), (True, True))
        self.assertEqual(self.human_metrics(sample(edge - 1, cache=limit - edge + 1 - MIB, slab=MIB)), (True, False))
        # 脏页、正在回写的页在文件 LRU 上，但一时回收不掉：各自逼近上限都报。
        self.assertEqual(self.human_metrics(sample(edge, dirty=edge - 300 * MIB)), (True, True))
        self.assertEqual(self.human_metrics(sample(edge, writeback=edge - 300 * MIB)), (True, True))
        self.assertEqual(self.human_metrics(sample(edge - 1, dirty=edge - 300 * MIB)), (True, False))
        # 活跃链表上的干净缓存同样可回收。
        active = dict(sample(edge - 1, cache=limit - edge + 1), inactive_file=0, active_file=limit - edge + 1)
        self.assertEqual(self.human_metrics(active), (True, False))
        # 锁住的脏页不在文件 LRU 上、已经算在回收不掉的里：脏页比 LRU 还多时不许再多加一遍。
        self.assertEqual(self.human_metrics(dict(sample(edge - 1), file_dirty=400 * MIB)), (True, False))
        self.assertEqual(self.human_metrics(dict(sample(edge), file_dirty=400 * MIB)), (True, True))

    def test_memory_alert_fires_on_sustained_stall_without_nearing_the_limit(self):
        def pressure(value):
            return (f"some avg10=99.00 avg60=99.00 avg300=99.00 total=1\n"
                    f"full avg10=99.00 avg60=99.00 avg300={value} total=1\n")
        # 只认 5 分钟窗口的 full：some 和短窗口再高也不算持续卡住。
        for value, high in (("0.00", False), ("9.99", False), ("10.00", True), ("100.00", True)):
            self.assertEqual(self.human_metrics(IDLE_HUMAN, pressure(value)), (True, high), value)
        for bad in ("", "nan", "inf", "-1.00", "10", "1e1", "101.00", "9.999"):
            self.assertEqual(self.human_metrics(IDLE_HUMAN, pressure(bad)), (True, True), bad)
        # 没有 full、两行 full（第二行写法不对也算两行）、full 混在别的字里：都是坏证据。
        for text in ("", CALM.splitlines()[0] + "\n", CALM + CALM.splitlines()[1] + "\n",
                     CALM + "full avg10=0.00 avg60=0.00 avg300=nan total=1\n", CALM + "full\n",
                     CALM.replace("full ", "fuller ")):
            self.assertEqual(self.human_metrics(IDLE_HUMAN, text), (True, True), text)

    def test_missing_unbounded_or_contradictory_memory_evidence_stays_high(self):
        human = "studiolo-human.service"
        # 无界、为零、坏值、缺字段、可回收的比总量还多：都不能冒称在预算内，也不牵连服务在不在。
        for change in (dict(max="max"), dict(max=0), dict(max=""), dict(max="-1"), dict(max="2147483648 1"), dict(max="２１４７４８３６４８"),
                       dict(current=""), dict(current="max"), dict(current="１２"), dict(slab_reclaimable="x"),
                       dict(inactive_file="-1"), dict(slab_reclaimable=IDLE_HUMAN["current"] + 1)):
            self.assertEqual(self.human_metrics(dict(IDLE_HUMAN, **change)), (True, True), change)
        for key in ("inactive_file", "active_file", "file_dirty", "file_writeback", "slab_reclaimable"):
            sample = {name: value for name, value in IDLE_HUMAN.items() if name != key}
            self.assertEqual(self.human_metrics(sample), (True, True), key)
        with tempfile.TemporaryDirectory() as root:
            for unit in PROBE.UNITS:
                self.memory_unit(root, unit, IDLE_AGENT)
            directory = self.memory_unit(root, human, IDLE_HUMAN)
            self.assertEqual(self.metrics(root), (True, False))
            for name in ("memory.current", "memory.max", "memory.stat", "memory.pressure"):
                saved = Path(directory, name).read_text()
                Path(directory, name).unlink()
                self.assertEqual(self.metrics(root), (True, True), name)
                # 重复的字段、没值的行、超长、非 ASCII 的文件都按坏证据算。
                for raw in (b"", b"\xff\n", (saved + " " * PROBE.CGROUP_FILE_MAX_BYTES).encode()):
                    Path(directory, name).write_bytes(raw)
                    self.assertEqual(self.metrics(root), (True, True), (name, raw[:8]))
                Path(directory, name).write_text(saved)
            stat = Path(directory, "memory.stat")
            saved = stat.read_text()
            for extra in ("slab_reclaimable 0\n", "lonely\n", "three words here\n"):
                stat.write_text(saved + extra)
                self.assertEqual(self.metrics(root), (True, True), extra)
            stat.write_text(saved)
            # 单元没有 cgroup、或者给的不是它自己的 cgroup（比如人侧的换成 agent 的）。
            for group in ("", "/", "/system.slice", "/system.slice/studiolo-agent.service",
                          "system.slice/" + human, "/system.slice/../system.slice/" + human):
                self.assertEqual(self.metrics(root, {human: {"ControlGroup": group}}), (True, True), group)
            # 单元换了 slice 也跟得上。
            moved = Path(root, "other.slice")
            moved.mkdir()
            Path(root, "system.slice", human).rename(moved / human)
            self.assertEqual(self.metrics(root, {human: {"ControlGroup": "/other.slice/" + human}}), (True, False))

    def test_one_tight_unit_is_enough_and_service_state_stays_separate(self):
        for tight in PROBE.UNITS:
            with tempfile.TemporaryDirectory() as root:
                for unit in PROBE.UNITS:
                    self.memory_unit(root, unit, BUG_054_OOM if unit == tight else IDLE_AGENT)
                self.assertEqual(self.metrics(root), (True, True), tight)
        with tempfile.TemporaryDirectory() as root:
            for unit in PROBE.UNITS:
                self.memory_unit(root, unit, IDLE_AGENT)
            self.assertEqual(self.metrics(root), (True, False))
            # 停着的单元：内存证据照看，服务不在另算。
            self.assertEqual(self.metrics(root, {"studiolo-agent.service": {"ActiveState": "failed"}}), (False, False))
            stopped = {"studiolo-agent.service": {"ActiveState": "inactive", "ControlGroup": ""}}
            self.assertEqual(self.metrics(root, stopped), (False, True))
        # systemctl 本身跑不出来：整个内存与服务结论都不成立。
        with patch.object(PROBE, "output", side_effect=TimeoutError()):
            result = PROBE.probe()
        self.assertEqual((result["services_ok"], result["memory_high"]), (False, True))

    HUMAN_SHOW = ["systemctl", "show", "studiolo-human.service", "--property=NRestarts,ActiveEnterTimestampMonotonic"]
    UP_AT = 147_665_927_299  # 微秒；169 上 2026-11-24 人侧这一次起来的单调时刻
    HOUR = 3_600_000_000

    def restarted(self, restarts, entered, now):
        # 次序照 169 的 systemctl：和要的次序相反。
        return PROBE.recently_restarted(f"ActiveEnterTimestampMonotonic={entered}\nNRestarts={restarts}", now)

    def test_an_automatic_restart_is_flagged_for_its_first_hour_only(self):
        up = self.UP_AT
        for restarts in (1, 3, 4096):
            for age, flagged in ((0, True), (60_000_000, True), (self.HOUR - 1, True), (self.HOUR, False), (self.HOUR * 24, False)):
                self.assertIs(self.restarted(restarts, up, up + age), flagged, (restarts, age))
        # 人手工启停、重启的（次数是 0），刚起来也不报；机器刚开机、单元从没起来过或不存在同样不算。
        for age in (0, 60_000_000, self.HOUR, self.HOUR * 24):
            self.assertIs(self.restarted(0, up, up + age), False, age)
        self.assertIs(self.restarted(0, 0, 60_000_000), False)
        # 属性次序反过来也认。
        self.assertIs(PROBE.recently_restarted(f"NRestarts=1\nActiveEnterTimestampMonotonic={up}", up), True)

    def test_unreadable_or_contradictory_restart_evidence_is_flagged(self):
        up = self.UP_AT
        later = up + self.HOUR * 24  # 读得出的话早就满 1 小时了
        for restarts in ("", "-1", "+0", "0.0", "0x1", "x", "[not set]", "０", "1_0", " 0", "9" * 21):
            self.assertIs(self.restarted(restarts, up, later), True, repr(restarts))
        for entered in ("", "-1", "x", "[not set]", "１", "1.5", " 1", "9" * 21):
            for restarts in (0, 1):
                self.assertIs(self.restarted(restarts, entered, later), True, repr(entered))
        # 起来的时刻在现在之后；重启过却从没起来过：自相矛盾，不当成正常。
        for restarts in (0, 1):
            self.assertIs(self.restarted(restarts, up + 1, up), True, restarts)
        self.assertIs(self.restarted(1, 0, later), True)
        # 缺一项、多一项、重复、没有等号：不猜。
        good = f"ActiveEnterTimestampMonotonic={up}\nNRestarts=0"
        self.assertIs(PROBE.recently_restarted(good, later), False)
        for text in ("", "NRestarts=0", f"ActiveEnterTimestampMonotonic={up}", good + "\nNRestarts=0", good + "\nExtra=1",
                     good + "\nlonely", good.replace("NRestarts=0", "NRestarts"), good.replace("NRestarts", "nrestarts")):
            self.assertIs(PROBE.recently_restarted(text, later), True, repr(text))

    def test_probe_reads_the_human_unit_and_keeps_the_service_verdict_separate(self):
        human = "studiolo-human.service"
        now = [self.UP_AT + 60_000_000]
        asked = []

        def run(show):
            # 三个单元都在、内存都宽裕；人侧那条由 show 决定。
            def systemctl(command):
                if command == self.HUMAN_SHOW:
                    asked.append(command)
                    return show()
                return f"ActiveState=active\nControlGroup=/system.slice/{command[2]}"
            with tempfile.TemporaryDirectory() as root:
                for unit in PROBE.UNITS:
                    self.memory_unit(root, unit, IDLE_AGENT)
                with patch.object(PROBE, "output", systemctl), patch.object(PROBE, "CGROUPS", Path(root)), \
                        patch.object(PROBE.time, "clock_gettime_ns", lambda clock: {PROBE.time.CLOCK_MONOTONIC: now[0] * 1000 + 999}[clock]):
                    return PROBE.probe()

        shown = lambda restarts: f"ActiveEnterTimestampMonotonic={self.UP_AT}\nNRestarts={restarts}"
        result = run(lambda: shown(1))
        self.assertEqual(set(result), STATE.PROBE_FIELDS)
        self.assertEqual((result["human_restarted"], result["services_ok"], result["memory_high"]), (True, True, False))
        result = run(lambda: shown(0))
        self.assertEqual((result["human_restarted"], result["services_ok"], result["memory_high"]), (False, True, False))
        # 时钟在 systemctl 答完之后才读：答的那一刻人侧刚好起来，不能读成「在现在之后」。
        def restarted_while_asking():
            now[0] = self.UP_AT + self.HOUR
            return shown(1)
        now[0] = self.UP_AT - 1
        self.assertIs(run(restarted_while_asking)["human_restarted"], False)
        # systemctl 跑不出来：只报这一项，不牵连服务在不在、内存够不够。
        def broken():
            raise TimeoutError()
        result = run(broken)
        self.assertEqual((result["human_restarted"], result["services_ok"], result["memory_high"]), (True, True, False))
        self.assertEqual(len(asked), 4)
        # 反过来，服务那一项跑不出来也不牵连它。
        with patch.object(PROBE, "service_metrics", side_effect=TimeoutError()), \
                patch.object(PROBE, "output", lambda command: shown(0)), \
                patch.object(PROBE.time, "clock_gettime_ns", lambda clock: self.UP_AT * 1000):
            result = PROBE.probe()
        self.assertEqual((result["human_restarted"], result["services_ok"]), (False, False))

    def test_restart_code_ranks_after_service_loss_and_before_the_lingering_ones(self):
        healthy = dict(services_ok=True, clock_ok=True, mcp_ok=True, replica_stale=False, memory_high=False,
                       sync_unaligned=False, sync_deletes_blocked=False, human_restarted=False)
        self.assertEqual(set(healthy), STATE.PROBE_FIELDS)
        self.assertIsNone(STATE.failure_code(healthy))
        restarted = dict(healthy, human_restarted=True)
        self.assertEqual(STATE.failure_code(restarted), "A09")
        # 能挂好几天的几项盖不住它：它一次只占 1 小时，反过来就一直看不见。
        for field in ("replica_stale", "memory_high", "sync_unaligned", "sync_deletes_blocked"):
            self.assertEqual(STATE.failure_code(dict(restarted, **{field: True})), "A09", field)
        for field, code in (("services_ok", "A02"), ("mcp_ok", "A02"), ("clock_ok", "A06")):
            self.assertEqual(STATE.failure_code(dict(restarted, **{field: False})), code, field)
        # 上一版探针的 7 个字段、非布尔值都不当成正常。
        self.assertEqual(STATE.failure_code({key: value for key, value in healthy.items() if key != "human_restarted"}), "A01")
        self.assertEqual(STATE.failure_code(dict(healthy, human_restarted=0)), "A01")
        self.assertEqual(STATE.failure_code(dict(healthy, extra=False)), "A01")
        self.assertEqual(STATE.CODES["A09"], "warning")

    def test_restart_code_fires_after_two_rounds_and_repeats_hourly(self):
        state, fire = STATE.observe(STATE.initial(100), "A09", 100)
        self.assertFalse(fire)
        state, fire = STATE.observe(state, "A09", 160)
        self.assertTrue(fire)
        before = state
        state = STATE.delivered(before, 160)
        self.assertEqual((state["hourly_sent"], state["daily_sent"]), ({"A09": 160}, {"A07": None, "A08": None}))
        # 记账给出新状态，传进来的那一份不动。
        self.assertEqual((before["sent"], before["hourly_sent"]), (None, {"A09": None}))
        # 反复崩溃：一小时内被 A02 打断再回来不重报，一小时到了还在崩才再报。
        state, fire = self.fire_twice(state, "A02", 220)
        self.assertTrue(fire)
        state = STATE.delivered(state, 280)
        self.assertEqual(state["hourly_sent"], {"A09": 160})
        state, fire = self.fire_twice(state, "A09", 340)
        self.assertFalse(fire)
        state, fire = self.fire_twice(state, "A07", 460)
        self.assertTrue(fire)
        state = STATE.delivered(state, 520)
        self.assertEqual((state["hourly_sent"], state["daily_sent"]), ({"A09": 160}, {"A07": 520, "A08": None}))
        state, fire = self.fire_twice(state, "A09", 580)
        self.assertFalse(fire)
        state, fire = STATE.observe(state, "A09", 160 + 3599)
        self.assertFalse(fire)
        state, fire = STATE.observe(state, "A09", 160 + 3600)
        self.assertTrue(fire)
        # 没发出去的不记账，下一轮就重试。
        state, fire = STATE.observe(state, "A09", 160 + 3660)
        self.assertTrue(fire)
        state = STATE.delivered(state, 160 + 3660)
        # 看到正常（起来满 1 小时，或人清了次数）就清：再崩是新的一条，连续两轮就报。
        state, fire = STATE.observe(state, None, 160 + 3720)
        self.assertFalse(fire)
        self.assertEqual((state["hourly_sent"], state["daily_sent"]), ({"A09": None}, {"A07": None, "A08": None}))
        state, fire = STATE.observe(state, "A09", 160 + 3780)
        self.assertFalse(fire)
        state, fire = STATE.observe(state, "A09", 160 + 3840)
        self.assertTrue(fire)
        # 时钟回拨同样清。
        state = STATE.delivered(state, 160 + 3840)
        state, fire = STATE.observe(state, "A09", 100)
        self.assertTrue(fire)
        self.assertEqual((state["code"], state["hourly_sent"]), ("A06", {"A09": None}))

    def test_watch_state_hourly_record_is_closed_and_bounded(self):
        good = STATE.initial(1000)
        self.assertEqual((good["schema"], good["hourly_sent"]), (3, {"A09": None}))
        STATE.validate(dict(good, hourly_sent={"A09": 1000}))
        STATE.validate(dict(good, hourly_sent={"A09": 0}))
        for bad in (
            dict(good, schema=2),
            dict(good, schema=4),
            {key: value for key, value in good.items() if key != "hourly_sent"},
            dict(good, hourly_sent={}),
            dict(good, hourly_sent={"A09": None, "A07": None}),
            dict(good, daily_sent={"A07": None, "A08": None, "A09": None}),
            dict(good, hourly_sent={"A09": True}),
            dict(good, hourly_sent={"A09": 1001}),
            dict(good, hourly_sent={"A09": -1}),
            dict(good, hourly_sent=None),
        ):
            with self.assertRaises(ValueError, msg=repr(bad)):
                STATE.validate(bad)
        # 上一版的状态文件（没有按小时的记录）读进来就停，不当成刚重置。
        previous = {key: value for key, value in dict(good, schema=2).items() if key != "hourly_sent"}
        with self.assertRaises(ValueError):
            STATE.observe(previous, None, 1060)

    def test_watch_run_hands_the_restart_code_to_the_sender_once_an_hour(self):
        restarted = dict(services_ok=True, clock_ok=True, mcp_ok=True, replica_stale=False, memory_high=False,
                         sync_unaligned=False, sync_deletes_blocked=False, human_restarted=True)
        sent = []

        def sender(command, **kwargs):
            sent.append(command)
            return subprocess.CompletedProcess(command, 0, stdout=b"accepted\n")

        with tempfile.TemporaryDirectory() as directory:
            for now in (1000, 1060, 1120, 1060 + 3600):
                with patch.object(RUN, "harden"), patch.object(RUN, "remote_probe", return_value=restarted), \
                        patch.object(RUN, "ping", return_value=[True, True]), patch.object(RUN.subprocess, "run", sender), \
                        patch.object(RUN.time, "time", return_value=now), patch("builtins.print"):
                    self.assertEqual(RUN.main(["watch", "--node", "n108", "--deliver", "--state", directory,
                                               "--audit", "/audit", "--sender", "/sender"]), 0)
            with IO.locked_state(directory) as state:
                self.assertEqual(state.load()["hourly_sent"], {"A09": 1060 + 3600})
        # 等级、代号就是发送端要的那两个参数；第 1、3 轮不发，满一小时再发一次，轮次不重用。
        self.assertEqual([command[:4] for command in sent], [["/sender", "/audit", "warning", "A09"]] * 2)
        self.assertNotEqual(sent[0][5], sent[1][5])

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

    def test_heartbeats_name_themselves_instead_of_python_urllib(self):
        environment = {"STUDIOLO_HEALTHCHECKS_URL": "https://hc-ping.com/" + "a" * 36,
                       "STUDIOLO_WORKER_HEARTBEAT_URL": "https://heartbeat.example.com/heartbeat/n169",
                       "STUDIOLO_WORKER_HEARTBEAT_TOKEN": "test-only"}
        class Capture(urllib.request.BaseHandler):
            # 排在默认 HTTPSHandler（500）之前接管：请求已走完 urllib 自己补请求头的那一步，再截下来，不出网。
            handler_order = 100
            def __init__(self): self.sent = []
            def https_open(self, request):
                self.sent.append((request.full_url, dict(request.header_items())))
                reply = urllib.response.addinfourl(io.BytesIO(b""), email.message.Message(), request.full_url, 204)
                reply.msg = "No Content"  # urllib 的错误处理器要读 msg，真响应才有
                return reply
        capture = Capture()
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), HB.NoRedirect(), capture)
        self.assertEqual(HB.ping("n169", environment, opener), [True, True])
        self.assertEqual([url for url, _ in capture.sent], [environment["STUDIOLO_HEALTHCHECKS_URL"],
                                                           environment["STUDIOLO_WORKER_HEARTBEAT_URL"]])
        for _, headers in capture.sent:
            self.assertEqual(headers.get("User-agent"), HB.USER_AGENT)
            self.assertFalse(any("python-urllib" in value.lower() for value in headers.values()))
        self.assertNotIn("Authorization", capture.sent[0][1])
        self.assertEqual(capture.sent[1][1].get("Authorization"), "Bearer test-only")


if __name__ == "__main__":
    unittest.main()
