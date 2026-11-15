"""备份 worker 单元模板只描述服务，不安装、不包含秘密或主机地址。"""
import pathlib
import unittest

UNIT = pathlib.Path(__file__).with_name("studiolo-backup-worker.service").read_text()


class UnitTemplate(unittest.TestCase):
    def test_resident_binary_and_hardening(self):
        self.assertIn("ExecStart=/usr/local/bin/studiolo-backup-worker\n", UNIT)
        self.assertNotIn("ExecStart=/usr/local/bin/studiolo-backup-worker ", UNIT)
        self.assertIn("User=studiolo-backup\n", UNIT)
        self.assertNotIn("User=root", UNIT)
        self.assertIn("NoNewPrivileges=yes", UNIT)
        self.assertIn("ProtectSystem=strict", UNIT)
        self.assertIn("StateDirectory=studiolo-backup-worker", UNIT)
        self.assertIn("ConditionPathExists=/etc/studiolo/backup-worker.json", UNIT)
        self.assertNotIn("password", UNIT.lower())
        self.assertNotIn("192.168.", UNIT)
        self.assertNotIn("/volume1", UNIT)


class ReclaimTimer(unittest.TestCase):
    def test_timer_only_starts_the_reclaim_oneshot(self):
        root = pathlib.Path(__file__).parent
        service = (root / "studiolo-backup-reclaim.service").read_text()
        timer = (root / "studiolo-backup-reclaim.timer").read_text()
        self.assertIn("Type=oneshot\n", service)
        self.assertIn("ExecStart=/usr/local/bin/studiolo-backup-worker reclaim\n", service)
        self.assertIn("Unit=studiolo-backup-reclaim.service\n", timer)
        self.assertIn("Persistent=true\n", timer)
        self.assertIn("OnUnitActiveSec=10min\n", timer)
        self.assertNotIn("password", service.lower())
        self.assertNotIn("password", timer.lower())
        self.assertNotIn("192.168.", service + timer)


if __name__ == "__main__":
    unittest.main()
