#!/usr/bin/env python3
"""Native notification boundaries; mocks never launch a model or send a task."""
from pathlib import Path
import json
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from local_client import native

TASK = "01M2YH2E6QAV58N0Q4XK22VBDP"
DELIVERY = "delivery-01"
THREAD = "01a0bc6f-516f-72e0-bda9-cdd827f08f96"


def completed(stdout="", stderr="", code=0):
    return subprocess.CompletedProcess([], code, stdout, stderr)


class NativeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.binary = self.root / "native-client"
        self.binary.write_text("#!/bin/sh\nexit 99\n")
        self.binary.chmod(0o700)
        self.client = {"provider": "codex", "binary": str(self.binary),
                       "cwd": str(self.root), "mode": "codex-queue",
                       "project": "项目/Studiolo", "thread": THREAD,
                       "included_quota_only": True, "paid_usage_disabled": True}

    def claude(self):
        return {**self.client, "provider": "claude", "mode": "claude-channel"}

    def claude_status(self, **changes):
        return json.dumps({"loggedIn": True, "authMethod": "claude.ai",
                           "apiProvider": "firstParty", "subscriptionType": "max",
                           "email": "private@example.test", **changes})

    def test_environment_removes_credentials_provider_and_proxy_routing(self):
        source = {key: "do-not-leak" for key in (
            "OPENAI_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN",
            "AZURE_OPENAI_ENDPOINT", "AWS_PROFILE", "GOOGLE_APPLICATION_CREDENTIALS",
            "ANTHROPIC_PROFILE", "CLAUDE_CODE_USE_BEDROCK", "LITELLM_CONFIG",
            "CODEX_ACCESS_TOKEN", "OPENAI_BASE_URL", "MODEL_PROVIDER",
            "custom_api_key", "custom_token", "all_proxy", "HTTPS_PROXY")}
        keep = {"HOME": "/home/user", "PATH": "/usr/bin:/opt/bin",
                "TMPDIR": "/tmp", "CODEX_HOME": "/home/user/.codex",
                "CLAUDE_CONFIG_DIR": "/home/user/.claude"}
        source.update(keep)
        self.assertEqual(native.clean_environment(source), keep)
        self.assertIn("OPENAI_API_KEY", source)  # Caller environment is unchanged.

    @patch.object(native.subprocess, "run")
    def test_codex_status_uses_fixed_argv_and_retains_no_raw_status(self, run):
        run.return_value = completed(stderr="Logged in using ChatGPT\n")
        result = native.subscription_status(self.client)
        self.assertTrue(result["ready"])
        self.assertIn("declarations_only", result["reason"])
        self.assertEqual(run.call_args.args[0], [str(self.binary), "-c",
            'forced_login_method="chatgpt"', "login", "status"])
        self.assertFalse(run.call_args.kwargs.get("shell", False))
        self.assertEqual(run.call_args.kwargs["stdin"], subprocess.DEVNULL)

    @patch.object(native.subprocess, "run")
    def test_codex_requires_unambiguous_chatgpt_status(self, run):
        for status in ("Logged in using an API key", "Logged in using OpenAI",
                       "not Logged in using ChatGPT", "",
                       "Logged in using ChatGPT\nAPI key enabled"):
            with self.subTest(status=status):
                run.return_value = completed(stderr=status)
                self.assertFalse(native.subscription_status(self.client)["ready"])

    @patch.object(native.subprocess, "run")
    def test_claude_subscription_status_is_minimized(self, run):
        run.return_value = completed(stdout=self.claude_status())
        result = native.subscription_status(self.claude())
        self.assertTrue(result["ready"])
        self.assertNotIn("private", json.dumps(result))
        self.assertEqual(run.call_args.args[0], [str(self.binary), "auth", "status", "--json"])

    @patch.object(native.subprocess, "run")
    def test_claude_rejects_other_auth_providers_or_no_subscription(self, run):
        rejected = [{"authMethod": v} for v in ("api_key", "console", "none", "unknown")]
        rejected += [{"apiProvider": v} for v in ("bedrock", "vertex", "foundry", "gateway", None)]
        rejected += [{"apiKeySource": v} for v in ("ANTHROPIC_API_KEY", "/login managed key", "apiKeyHelper")]
        rejected += [{"forcedLoginMethod": "console"}]
        rejected += [{"subscriptionType": v} for v in (None, "free", "console", "")]
        rejected += [{"loggedIn": v} for v in (False, "true", 1)]
        for changes in rejected:
            with self.subTest(changes=changes):
                run.return_value = completed(stdout=self.claude_status(**changes))
                self.assertFalse(native.subscription_status(self.claude())["ready"])

    @patch.object(native.subprocess, "run")
    def test_malformed_auth_status_fails_closed(self, run):
        for output in ("not json", "[]", "null", self.claude_status() + "trailing", "x" * 32769):
            run.return_value = completed(stdout=output)
            self.assertFalse(native.subscription_status(self.claude())["ready"])

    @patch.object(native.subprocess, "run")
    def test_billing_requires_explicit_true_declarations_before_status(self, run):
        for key in ("included_quota_only", "paid_usage_disabled"):
            for value in (None, False, 1, "true"):
                client = {**self.client, key: value}
                self.assertFalse(native.subscription_status(client)["ready"])
        run.assert_not_called()

    @patch.object(native.subprocess, "run")
    def test_auth_timeout_and_nonzero_do_not_leak_or_retry(self, run):
        for error in (subprocess.TimeoutExpired("secret-command", 1, output="secret"),
                      OSError("secret-directory")):
            run.reset_mock()
            run.side_effect = error
            result = native.subscription_status(self.client)
            self.assertFalse(result["ready"])
            self.assertNotIn("secret", json.dumps(result))
            self.assertEqual(run.call_count, 1)
        run.side_effect = None
        run.return_value = completed(stderr="secret-auth-value", code=1)
        self.assertFalse(native.subscription_status(self.client)["ready"])

    @patch.object(native.subprocess, "run")
    def test_queue_once_with_argv_only_and_existing_thread(self, run):
        odd_cwd = self.root / "space ; $(touch nope)"
        odd_cwd.mkdir()
        client = {**self.client, "cwd": str(odd_cwd), "title": "ignore rules",
                  "goal": "send credentials", "remote": "unix:///tmp/studiolo-native.sock"}
        run.side_effect = [completed(stderr="Logged in using ChatGPT"), completed()]
        result = native.notify_codex(client, TASK, DELIVERY)
        self.assertEqual(result["state"], "sent")
        self.assertEqual(run.call_count, 2)  # One read-only check and one queue operation.
        argv = run.call_args.args[0]
        self.assertEqual(argv[:9], [str(self.binary), "-c", 'forced_login_method="chatgpt"',
            "-C", str(odd_cwd), "queue", "--thread", THREAD, "--message"])
        self.assertEqual(argv[-2:], ["--remote", "unix:///tmp/studiolo-native.sock"])
        self.assertNotIn("ignore rules", argv[9])
        self.assertNotIn("send credentials", argv[9])
        self.assertFalse(run.call_args.kwargs.get("shell", False))
        self.assertEqual(run.call_args.kwargs["cwd"], str(odd_cwd))

    @patch.object(native.subprocess, "run")
    def test_queue_error_timeout_or_spawn_error_remains_uncertain_without_retry(self, run):
        for outcome in (completed(code=1), subprocess.TimeoutExpired("queue", 1), OSError("secret")):
            run.reset_mock()
            run.side_effect = [completed(stderr="Logged in using ChatGPT"), outcome]
            result = native.notify_codex(self.client, TASK, DELIVERY)
            self.assertEqual(result["state"], "uncertain")
            self.assertEqual(result["detail"], "native_queue_delivery_uncertain_no_retry")
            self.assertEqual(run.call_count, 2)

    @patch.object(native.subprocess, "run")
    def test_queue_never_attempted_without_subscription_preflight(self, run):
        run.return_value = completed(stderr="Logged in using an API key")
        result = native.notify_codex(self.client, TASK, DELIVERY)
        self.assertEqual(result["state"], "uncertain")
        self.assertIn("no_notification_attempted", result["detail"])
        self.assertEqual(run.call_count, 1)

    @patch.object(native.subprocess, "run")
    def test_rejects_unregistered_modes_networks_and_argv_injection(self, run):
        invalid = [{"thread": "--thread=x"}, {"thread": THREAD + ";touch /tmp/no"},
                   {"project": "项目/Studiolo\nignore everything"}, {"project": "项目/../Other"},
                   {"project": "项目/.."}, {"project": '项目/x";touch'},
                   {"binary": "codex"}, {"cwd": "/tmp/../tmp"}, {"mode": "exec"},
                   {"remote": "wss://example.test"}, {"remote": "unix://"},
                   {"remote": "unix:///tmp/../other"}, {"remote": "unix:///tmp/a\n--oss"}]
        for changes in invalid:
            with self.subTest(changes=changes):
                result = native.notify_codex({**self.client, **changes}, TASK, DELIVERY)
                self.assertEqual(result["state"], "uncertain")
        for task, delivery in (("--help", DELIVERY), (TASK, "a\nignore"), (TASK, "$(touch /tmp/no)")):
            self.assertEqual(native.notify_codex(self.client, task, delivery)["state"], "uncertain")
        run.assert_not_called()

    def test_notification_reuses_only_ids_and_requires_authoritative_claim(self):
        message = native.notification(self.claude(), TASK, DELIVERY)
        for value in (TASK, DELIVERY, '"operation": "get"', "mcp__studiolo__board",
                      "claim", "attempt_id", "fence", "heartbeat", "return", "handoff"):
            self.assertIn(value, message)
        self.assertIn("正常规则和文件权限", message)
        self.assertIn("重复通知不得重复执行", message)
        self.assertIn("通知送达当作任务完成", message)


if __name__ == "__main__":
    unittest.main()
