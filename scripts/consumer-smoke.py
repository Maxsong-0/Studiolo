#!/usr/bin/env python3
"""Interactive MCP client for a create-only consumer-smoke instance, not an assertion runner.

Usage: consumer-smoke.py ROOT SESSION IDENTITY METHOD < params.json
ROOT/connection.json contains the loopback URL; tokens remain in private files.
Each invocation is one caller-selected MCP action, recorded without bearer/session secrets.
"""
import datetime
import json
import os
from pathlib import Path
import re
import sys
import urllib.error
import urllib.parse
import urllib.request


def private_write(path, value):
    with open(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w") as stream:
        json.dump(value, stream, ensure_ascii=False)


def main():
    if len(sys.argv) != 5:
        raise SystemExit(__doc__)
    root = Path(sys.argv[1]).resolve(strict=True)
    session, identity, method = sys.argv[2:]
    if not re.fullmatch(r"[a-z0-9-]{1,48}", session) or identity not in {"codex", "claude", "grok"}:
        raise SystemExit("Invalid local smoke session or identity")
    descriptor = json.loads((root / "connection.json").read_text())
    endpoint = descriptor["mcp"]
    parsed = urllib.parse.urlparse(endpoint)
    if parsed.scheme != "http" or parsed.hostname != "127.0.0.1" or parsed.path != "/mcp":
        raise SystemExit("Only the private smoke instance's literal loopback endpoint is allowed")
    state_file = root / "clients" / (session + ".session.json")
    if method == "initialize":
        if state_file.exists():
            raise SystemExit("Session already exists; choose a new name")
        state = {"identity": identity, "counter": 1}
    else:
        state = json.loads(state_file.read_text())
        if state["identity"] != identity:
            raise SystemExit("Cannot change identity within a session")
        state["counter"] += 1
    params = json.load(sys.stdin)
    headers = {
        "Authorization": "Bearer " + (root / "clients" / (identity + ".token")).read_text().strip(),
        "Accept": "application/json, text/event-stream",
        "Content-Type": "application/json",
        "MCP-Protocol-Version": "2025-11-25",
    }
    if state.get("session_id"):
        headers["Mcp-Session-Id"] = state["session_id"]
    message = {"jsonrpc": "2.0", "id": state["counter"], "method": method, "params": params}
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def send(body):
        request = urllib.request.Request(endpoint, json.dumps(body).encode(), headers, method="POST")
        try:
            response = opener.open(request, timeout=20)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            raw = response.read(512 * 1024 + 1)
            if len(raw) > 512 * 1024:
                raise RuntimeError("MCP response exceeded smoke client bound")
            if response.headers.get("content-type", "").startswith("text/event-stream"):
                values = []
                for event in raw.decode().replace("\r\n", "\n").split("\n\n"):
                    data = "\n".join(line[5:].lstrip(" ") for line in event.splitlines() if line.startswith("data:"))
                    if data.strip():
                        values.append(json.loads(data))
                result = values[-1] if values else None
            else:
                result = json.loads(raw) if raw else None
            return response.status, result, response.headers.get("Mcp-Session-Id")

    status, result, returned_session = send(message)
    if method == "initialize" and status == 200 and returned_session and "result" in (result or {}):
        state["session_id"] = returned_session
        headers["Mcp-Session-Id"] = returned_session
        notify_status, _, _ = send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        if notify_status != 202:
            raise RuntimeError(f"initialized notification rejected: {notify_status}")
    private_write(state_file, state)
    record = {
        "time": datetime.datetime.now(datetime.timezone.utc).isoformat(), "session": session,
        "identity": identity, "http_status": status, "request": message, "response": result,
    }
    with open(os.open(root / "transcript.jsonl", os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600), "a") as stream:
        stream.write(json.dumps(record, ensure_ascii=False) + "\n")
    print(json.dumps(record, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
