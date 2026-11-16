"""备份 worker 部署模板契约：单元和程序的固定路径、子命令、凭据槽位对得上；不碰真实环境。"""
from pathlib import Path
import re
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
WORKER = (HERE / "systemd" / "studiolo-backup-worker.service").read_text(encoding="utf-8")
SCHEDULE = (HERE / "systemd" / "studiolo-backup-schedule.service").read_text(encoding="utf-8")
TIMER = (HERE / "systemd" / "studiolo-backup-schedule.timer").read_text(encoding="utf-8")
TMPFILES = (HERE / "systemd" / "studiolo-backup.tmpfiles.conf").read_text(encoding="utf-8")
README = (HERE / "README.md").read_text(encoding="utf-8")
SOURCE = ROOT / "crates" / "backup-worker" / "src"
MAIN = (SOURCE / "main.rs").read_text(encoding="utf-8")
SERVE = (SOURCE / "serve.rs").read_text(encoding="utf-8")
PACKAGE = (SOURCE / "package.rs").read_text(encoding="utf-8")
CREDENTIAL_ROOT = (SOURCE / "credentials" / "root.rs").read_text(encoding="utf-8")
CREDENTIAL_FILE = (SOURCE / "credentials" / "file.rs").read_text(encoding="utf-8")
TEMPORARY = (SOURCE / "temporary.rs").read_text(encoding="utf-8")
LISTENER = (ROOT / "crates" / "local-server" / "src" / "backup" / "listener.rs").read_text(encoding="utf-8")


def values(text, key):
    return re.findall(rf"^{re.escape(key)}=(.*)$", text, flags=re.MULTILINE)


class BackupWorkerUnitTests(unittest.TestCase):
    def test_each_unit_runs_one_fixed_subcommand_the_program_knows(self):
        for unit, command in ((WORKER, "run"), (SCHEDULE, "schedule")):
            started = values(unit, "ExecStart")
            self.assertEqual(len(started), 1)
            binary, argument = started[0].split()
            self.assertEqual(binary, "/usr/lib/studiolo-backup-worker/studiolo-backup-worker")
            self.assertEqual(argument, command)
            self.assertIn(f'Some("{command}") =>', MAIN)
            self.assertEqual(values(unit, "ExecStartPre"), [])
            self.assertEqual(values(unit, "Environment"), [])
            self.assertEqual(values(unit, "EnvironmentFile"), [])
        self.assertEqual(values(WORKER, "Type"), ["simple"])
        self.assertEqual(values(SCHEDULE, "Type"), ["oneshot"])
        # 固定包和程序放在同一个 root 目录里。
        self.assertIn('"/usr/lib/studiolo-backup-worker/restic-0.19.1"', PACKAGE)
        self.assertIn('Some("init") =>', MAIN)
        self.assertIn("`init`", README)

    def test_worker_identity_is_dedicated_and_shared_only_through_the_reader_group(self):
        for unit in (WORKER, SCHEDULE):
            self.assertEqual(values(unit, "User"), ["studiolo-backup"])
            self.assertEqual(values(unit, "Group"), ["studiolo-backup"])
            self.assertEqual(values(unit, "SupplementaryGroups"), ["studiolo-backup-read"])
            for line in (
                "UMask=0077",
                "LimitCORE=0",
                "NoNewPrivileges=yes",
                "ProtectSystem=strict",
                "ProtectHome=yes",
                "PrivateTmp=yes",
                "RestrictSUIDSGID=yes",
                "LockPersonality=yes",
                "SystemCallArchitectures=native",
                "StandardInput=null",
            ):
                self.assertIn(line, unit.splitlines())
        # 常驻进程要锁内存；一次性触发不读凭据，也不需要网络。
        self.assertIn("LimitMEMLOCK=infinity", WORKER.splitlines())
        self.assertEqual(values(SCHEDULE, "RestrictAddressFamilies"), ["AF_UNIX"])
        self.assertEqual(values(SCHEDULE, "LoadCredential"), [])

    def test_credentials_arrive_only_as_the_three_fixed_systemd_slots(self):
        # 程序只读这个固定目录，所以单元文件名不能改。
        self.assertIn('"/run/credentials/studiolo-backup-worker.service"', CREDENTIAL_ROOT)
        slots = sorted(re.findall(r'=> "([a-z-]+)"', CREDENTIAL_FILE))
        self.assertEqual(slots, ["repository-password", "rest-password", "rest-username"])
        loaded = values(WORKER, "LoadCredential")
        self.assertEqual(sorted(entry.split(":")[0] for entry in loaded), slots)
        for entry in loaded:
            name, path = entry.split(":")
            self.assertEqual(path, f"/etc/studiolo/backup-worker-credentials/{name}")
            self.assertIn(f"`{name}`", README)
        for text in (WORKER, SCHEDULE, TIMER, TMPFILES, README):
            self.assertIsNone(re.search(r"rest:https://[^\s`]*@", text))
            self.assertNotIn("RESTIC_PASSWORD=", text)

    def test_state_and_temporary_directories_match_the_fixed_paths(self):
        self.assertEqual(
            values(WORKER, "StateDirectory"),
            ["studiolo-backup-worker/state studiolo-backup-worker/tmp"],
        )
        self.assertEqual(values(WORKER, "StateDirectoryMode"), ["0700"])
        self.assertIn('"/var/lib/studiolo-backup-worker/state"', SERVE)
        self.assertIn('"/var/lib/studiolo-backup-worker/tmp"', TEMPORARY)
        self.assertEqual(values(WORKER, "Restart"), ["on-failure"])
        self.assertEqual(values(WORKER, "KillMode"), ["control-group"])

    def test_shared_directories_carry_the_modes_both_sides_check(self):
        rows = [line.split() for line in TMPFILES.splitlines() if line.startswith("d ")]
        self.assertEqual(len(rows), 2)
        for _, path, mode, owner, group, _ in rows:
            self.assertTrue(path.startswith("/"))
            self.assertEqual(mode, "0750")
            self.assertEqual(group, "studiolo-backup-read")
            self.assertNotIn(owner, ("root", "studiolo-backup"))
        # 主服务一侧核的就是 0750 父目录、0660 socket。
        self.assertIn("0o750", LISTENER)
        self.assertIn("0o660", LISTENER)

    def test_timer_only_asks_and_targets_the_schedule_unit(self):
        self.assertEqual(values(TIMER, "Unit"), ["studiolo-backup-schedule.service"])
        self.assertEqual(values(TIMER, "OnCalendar"), ["*-*-* *:0/5:00"])
        self.assertEqual(values(TIMER, "WantedBy"), ["timers.target"])
        self.assertGreaterEqual(int(values(SCHEDULE, "TimeoutStartSec")[0]), 10)


if __name__ == "__main__":
    unittest.main()
