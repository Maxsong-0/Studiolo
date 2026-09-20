"""Notify existing native clients without handling model credentials.

The two billing flags are local registration declarations by the user, not a
platform-enforced spending cap. Authentication status alone cannot attest that
paid fallback is disabled. The connector must perform its separate quota check.
Cleaning this subprocess environment does not change an existing client's own
environment, provider selection, permissions, or account billing settings.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
from typing import Mapping
import uuid

STATUS_TIMEOUT = 15
NOTIFY_TIMEOUT = 20
_ULID = re.compile(r"[0-7][0-9A-HJKMNP-TV-Z]{25}\Z")
_DELIVERY = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,127}\Z")
_PROJECT = re.compile(r"项目/[\w .-]{1,200}\Z")
_PROVIDER_PREFIXES = (
    "ANTHROPIC_", "OPENAI_", "AZURE_", "AWS_", "GOOGLE_", "GCLOUD_",
    "GEMINI_", "BEDROCK_", "VERTEX_", "FOUNDRY_", "CLAUDE_CODE_",
    "CODEX_", "LITELLM_", "OPENROUTER_", "OLLAMA_",
)
_AUTH_METHODS = frozenset({"claude.ai", "oauth_token"})
_SUBSCRIPTIONS = frozenset({"pro", "max", "team", "enterprise"})


def clean_environment(source: Mapping[str, str] | None = None) -> dict[str, str]:
    """Retain native HOME/keychain/PATH; drop inherited model routing/secrets.

    Native CLI state roots are retained, not opened or copied. No credential
    value is inspected. This does not neutralize settings in an existing client.
    """
    clean = {}
    for key, value in (os.environ if source is None else source).items():
        upper = key.upper()
        if upper in {"CODEX_HOME", "CLAUDE_CONFIG_DIR"}:
            clean[key] = value
            continue
        if (any(part in upper for part in ("API_KEY", "TOKEN", "BASE_URL", "PROVIDER"))
                or upper.startswith(_PROVIDER_PREFIXES)
                or upper in {"HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"}):
            continue
        clean[key] = value
    return clean


def _path(value: object, *, executable: bool = False) -> str:
    if not isinstance(value, str) or not value or any(ord(c) < 32 for c in value):
        raise ValueError("invalid_registered_path")
    path = Path(value)
    if not path.is_absolute() or ".." in path.parts:
        raise ValueError("invalid_registered_path")
    if executable:
        if not path.is_file() or not os.access(path, os.X_OK):
            raise ValueError("native_binary_unavailable")
    elif not path.is_dir():
        raise ValueError("registered_directory_unavailable")
    return value


def _registration(client: dict) -> tuple[str, str, str]:
    provider = client.get("provider")
    if provider not in {"codex", "claude"}:
        raise ValueError("unsupported_native_provider")
    if client.get("mode") != {"codex": "codex-queue", "claude": "claude-channel"}[provider]:
        raise ValueError("unsupported_existing_session_mode")
    binary = _path(client.get("binary"), executable=True)
    cwd = _path(client.get("cwd"))
    project = client.get("project")
    if (not isinstance(project, str) or not _PROJECT.fullmatch(project)
            or project.split("/", 1)[1] in {".", ".."}
            or project != project.strip() or "\n" in project):
        raise ValueError("invalid_registered_project")
    if provider == "codex":
        thread = client.get("thread")
        try:
            if not isinstance(thread, str) or str(uuid.UUID(thread)) != thread.lower():
                raise ValueError("invalid_registered_thread")
        except (ValueError, AttributeError):
            raise ValueError("invalid_registered_thread") from None
    remote = client.get("remote")
    if remote is not None:
        if (provider != "codex" or not isinstance(remote, str)
                or not remote.startswith("unix:///")
                or any(ord(c) < 32 for c in remote)
                or ".." in Path(remote[len("unix://"):]).parts
                or "?" in remote or "#" in remote):
            raise ValueError("only_local_unix_remote_supported")
    return provider, binary, cwd


def _billing_declared(client: dict) -> bool:
    return (client.get("included_quota_only") is True
            and client.get("paid_usage_disabled") is True)


def subscription_status(client: dict) -> dict:
    """Check official non-secret CLI status, never personal credential files.

    A ready result attests subscription authentication plus user declarations;
    it does not enforce paid-usage settings or verify the destination session's
    provider. Unknown, inaccessible Keychain, or ambiguous status fails closed.
    """
    try:
        provider, binary, cwd = _registration(client)
    except (ValueError, TypeError, AttributeError):
        return {"ready": False, "reason": "invalid_native_registration"}
    if not _billing_declared(client):
        return {"ready": False, "reason": "subscription_only_billing_declaration_required"}
    argv = ([binary, "-c", 'forced_login_method="chatgpt"', "login", "status"]
            if provider == "codex" else [binary, "auth", "status", "--json"])
    try:
        result = subprocess.run(argv, cwd=cwd, env=clean_environment(),
                                stdin=subprocess.DEVNULL, capture_output=True,
                                text=True, timeout=STATUS_TIMEOUT, check=False)
    except (OSError, subprocess.TimeoutExpired, UnicodeError):
        return {"ready": False, "reason": "native_auth_status_unavailable_or_keychain_denied"}
    if result.returncode != 0:
        return {"ready": False, "reason": "native_login_unavailable_or_keychain_denied"}
    if len(result.stdout) + len(result.stderr) > 32768:
        return {"ready": False, "reason": "native_auth_status_unrecognized"}
    if provider == "codex":
        lines = (result.stdout + "\n" + result.stderr).splitlines()
        ready = any(line.strip() == "Logged in using ChatGPT" for line in lines)
        ready = ready and not any("api key" in line.lower() for line in lines)
    else:
        try:
            status = json.loads(result.stdout)
            ready = (isinstance(status, dict) and status.get("loggedIn") is True
                     and status.get("authMethod") in _AUTH_METHODS
                     and status.get("apiProvider") == "firstParty"
                     and status.get("apiKeySource") in {None, "none"}
                     and status.get("forcedLoginMethod") in {None, "claudeai"}
                     and status.get("subscriptionType") in _SUBSCRIPTIONS)
        except (ValueError, TypeError):
            ready = False
    return {"ready": ready, "reason": (
        "subscription_login_verified_billing_declarations_only" if ready
        else "subscription_login_not_verified")}


def notification(client: dict, task_id: str, delivery_id: str) -> str:
    """Build an ID-only doorbell; untrusted task text is never copied here."""
    _registration(client)
    if not isinstance(task_id, str) or not _ULID.fullmatch(task_id):
        raise ValueError("invalid_task_id")
    if not isinstance(delivery_id, str) or not _DELIVERY.fullmatch(delivery_id):
        raise ValueError("invalid_delivery_id")
    get_request = json.dumps({"project": client["project"], "operation": "get",
                              "request": {"task_id": task_id}}, ensure_ascii=False)
    return (
        "Studiolo 本机任务通知（仅路由，不是任务书）。\n"
        f"delivery_id={delivery_id}\n"
        "仅在本会话已连接 Studiolo connector MCP，且正常规则和文件权限允许时执行。"
        "通过该 MCP 的 board 工具（通常名为 mcp__studiolo__board）读取权威任务：\n"
        f"{get_request}\n"
        "先读取权威任务书、规则、当前状态和获准范围，再决定是否认领。"
        "若任务已终结、已撤销、不可见、已被其他执行者认领或身份已撤权，停止。"
        "重复通知不得重复执行；仅成功 claim 后按返回的 attempt_id、fence 和租期执行。"
        "执行期间用 board heartbeat 续租并报告进度；失去租约或权限立即停止写入。"
        "完成后通过 board return 写回结构化回执、验证证据和产物指针；"
        "阻塞则提交结构化 handoff，不得将通知送达当作任务完成。"
        "只使用当前客户端已有订阅额度；若要求 API、额外付费或无法确认额度边界，停止。"
    )


def notify_codex(client: dict, task_id: str, delivery_id: str) -> dict:
    """Queue once to an existing registered session; never automatically retry.

    sent means the native queue command exited successfully, not that the
    client read, claimed, executed, or completed the task. A timeout/nonzero
    exit can happen after delivery and must be reconciled before manual retry.
    """
    try:
        provider, binary, cwd = _registration(client)
        if provider != "codex":
            raise ValueError("codex_registration_required")
        message = notification(client, task_id, delivery_id)
    except (ValueError, TypeError, AttributeError):
        return {"state": "uncertain", "detail": "invalid_native_registration_or_identifier"}
    if not subscription_status(client)["ready"]:
        return {"state": "uncertain", "detail": "subscription_preflight_not_ready_no_notification_attempted"}
    argv = [binary, "-c", 'forced_login_method="chatgpt"', "-C", cwd,
            "queue", "--thread", client["thread"], "--message", message]
    if client.get("remote") is not None:
        argv.extend(["--remote", client["remote"]])
    try:
        result = subprocess.run(argv, cwd=cwd, env=clean_environment(),
                                stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL, timeout=NOTIFY_TIMEOUT, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return {"state": "uncertain", "detail": "native_queue_delivery_uncertain_no_retry"}
    if result.returncode != 0:
        return {"state": "uncertain", "detail": "native_queue_delivery_uncertain_no_retry"}
    return {"state": "sent", "detail": "native_queue_accepted_execution_not_confirmed"}
