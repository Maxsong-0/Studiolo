"""门铃部署模板契约：单元只做一件事、带齐加固项、命令行和程序入口对得上；不碰真实环境。"""
from pathlib import Path
import re
import unittest

HERE = Path(__file__).resolve().parent
SERVICE = (HERE / "systemd" / "studiolo-doorbell.service").read_text(encoding="utf-8")
TIMER = (HERE / "systemd" / "studiolo-doorbell.timer").read_text(encoding="utf-8")
MAIN = (HERE.parents[1] / "crates" / "local-server" / "src" / "doorbell_main.rs").read_text(encoding="utf-8")
ENDPOINT = (HERE.parents[1] / "crates" / "observability" / "src" / "doorbell" / "endpoint.rs").read_text(encoding="utf-8")
README = (HERE / "README.md").read_text(encoding="utf-8")


def values(text, key):
    return re.findall(rf"^{re.escape(key)}=(.*)$", text, flags=re.MULTILINE)


class DoorbellUnitTests(unittest.TestCase):
    def test_service_runs_one_fixed_command_with_explicit_delivery(self):
        command = values(SERVICE, "ExecStart")
        self.assertEqual(len(command), 1)
        parts = command[0].split()
        self.assertEqual(len(parts), 4)
        binary, audit, state, switch = parts
        self.assertTrue(binary.endswith("/bin/studiolo-doorbell"))
        self.assertTrue(audit.startswith("/") and state.startswith("/") and audit != state)
        self.assertEqual(switch, "--deliver")
        self.assertIn('Some("--deliver") => true', MAIN)
        self.assertEqual(values(SERVICE, "Type"), ["oneshot"])
        self.assertEqual(values(SERVICE, "ExecStartPre"), [])

    def test_service_can_only_write_its_two_private_directories(self):
        _, audit, state, _ = values(SERVICE, "ExecStart")[0].split()
        self.assertEqual(sorted(values(SERVICE, "ReadWritePaths")), sorted([audit, state]))
        for line in (
            "UMask=0077",
            "LimitCORE=0",
            "LimitMEMLOCK=infinity",
            "NoNewPrivileges=yes",
            "ProtectSystem=strict",
            "ProtectHome=read-only",
            "PrivateTmp=yes",
            "RestrictSUIDSGID=yes",
            "LockPersonality=yes",
            "SystemCallArchitectures=native",
            "StandardInput=null",
        ):
            self.assertIn(line, SERVICE.splitlines())
        self.assertNotIn("root", values(SERVICE, "User"))
        # 单次运行最多 32 条、每条 10 秒；时限要盖得住，又不能无限。
        self.assertGreaterEqual(int(values(SERVICE, "TimeoutStartSec")[0]), 32 * 10 + 30)

    def test_secrets_come_only_from_the_private_environment_file(self):
        files = values(SERVICE, "EnvironmentFile")
        self.assertEqual(len(files), 1)
        self.assertTrue(files[0].startswith("/") and not files[0].startswith("-"))
        self.assertEqual(values(SERVICE, "Environment"), [])
        names = re.findall(r'"(STUDIOLO_DOORBELL_[A-Z]+)"', ENDPOINT)
        self.assertEqual(sorted(names), ["STUDIOLO_DOORBELL_LINK", "STUDIOLO_DOORBELL_SECRET", "STUDIOLO_DOORBELL_URL"])
        for name in names:
            self.assertIn(f"`{name}`", README)
            self.assertNotIn(f"{name}=", SERVICE)
        for text in (SERVICE, TIMER, README):
            self.assertIsNone(re.search(r"whsec_[A-Za-z0-9+/=]{8,}", text))

    def test_timer_fires_every_minute_and_targets_the_service(self):
        self.assertEqual(values(TIMER, "Unit"), ["studiolo-doorbell.service"])
        self.assertEqual(values(TIMER, "OnCalendar"), ["*-*-* *:*:30"])
        self.assertEqual(values(TIMER, "WantedBy"), ["timers.target"])


if __name__ == "__main__":
    unittest.main()
