#!/usr/bin/python3
import json
import pathlib
import sys

path = pathlib.Path("rewind-fixture.json")
state = json.loads(path.read_text())
for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    method = request["method"]
    params = request.get("params", {})
    state.setdefault("methods", []).append(method)
    result = {}
    error = None
    if method == "thread/resume":
        if state.get("mode") == "resumeError":
            error = {"code": -32000, "message": "Original conversation unavailable"}
        else:
            result = {"thread": {"id": params["threadId"]}}
    elif method == "thread/turns/list":
        turns = list(reversed(state["forkTurns"] if params["threadId"] == "reverted-session" else state["turns"]))
        offset = int(params.get("cursor") or 0)
        end = min(offset + min(params.get("limit", 2), 2), len(turns))
        result = {"data": [{"id": x} for x in turns[offset:end]], "nextCursor": str(end) if end < len(turns) else None}
    elif method == "thread/rollback":
        state["rollbackCount"] = params["numTurns"]
        if state.get("mode") in ["paginated", "forkUnchanged", "archiveError"]:
            error = {"code": -32600, "message": "paginated threads do not support thread/rollback"}
        elif state.get("mode") != "unchanged":
            state["turns"] = state["turns"][:-params["numTurns"]]
        result = {"thread": {"id": "existing-session", "turns": [{"id": x} for x in state["turns"]]}}
    elif method in ["thread/fork", "thread/start"]:
        end = state["turns"].index(params["lastTurnId"]) + 1 if method == "thread/fork" else 0
        state["forkTurns"] = state["turns"] if state.get("mode") == "forkUnchanged" else state["turns"][:end]
        result = {"thread": {"id": "reverted-session"}}
    elif method == "thread/archive":
        if state.get("mode") == "archiveError" and params["threadId"] == "existing-session":
            error = {"code": -32000, "message": "Archive failed"}
        else:
            state.setdefault("archived", []).append(params["threadId"])
    path.write_text(json.dumps(state))
    response = {"id": request["id"], "error": error} if error else {"id": request["id"], "result": result}
    print(json.dumps(response), flush=True)
