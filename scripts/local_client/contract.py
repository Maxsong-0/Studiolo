"""Precise local MCP board contract, matching the canonical Rust request types.

This describes wire shapes; the canonical server still owns authorization,
claim/fence/lease checks, receipt verification, byte limits and state transitions.
Keep in sync with local-server/board/{requests,content,execute}.rs,
task-engine/board_types.rs and vault-broker/{batch/model,memory_write}.rs.
"""
from copy import deepcopy
import re

OPERATIONS = (
    "list", "get", "claim", "heartbeat", "handoff", "return", "cancel",
    "number", "batch.preview", "batch.commit", "batch.rollback", "memory.preview",
)
_ULID = "[0-7][0-9A-HJKMNP-TV-Z]{25}"
_UINT = {"type": "integer", "minimum": 0, "maximum": 18446744073709551615}
_HASH = {"type": "string", "pattern": "^[0-9a-fA-F]{64}$"}
_TEXT = {"type": "string"}


def _object(properties, required=None):
    return {"type": "object", "properties": deepcopy(properties),
            "required": list(properties) if required is None else list(required),
            "additionalProperties": False}


def _array(items, minimum=0, maximum=None):
    value = {"type": "array", "items": deepcopy(items), "minItems": minimum}
    if maximum is not None:
        value["maxItems"] = maximum
    return value


def _schemas(project):
    path = {"type": "string", "pattern": "^" + re.escape(project) + "(?:/|$)",
            "description": "Canonical logical Vault path within the registered project."}
    text_list = _array({"type": "string", "minLength": 1, "maxLength": 2048}, maximum=64)
    revision = {"type": "string", "minLength": 1, "maxLength": 256,
                "description": "Exact canonical revision string, copied from a verified read."}
    artifact = _object({"path": path, "revision": revision, "sha256": _HASH})
    identity = _object({"task_id": {"type": "string", "pattern": "^" + _ULID + "$"},
                        "attempt_id": {"type": "string", "pattern": "^A-" + _ULID + "$"},
                        "fence": {**_UINT, "minimum": 1}})
    identity["description"] = "Copy task_id, attempt_id and fence from the successful claim."
    receipt = _object({
        "status": {"type": "string", "enum": ["completed", "failed", "stopped", "abandoned"]},
        "artifacts": _array(artifact), "conflicts": _array(path),
        "unlanded": _array(_object({"path": path, "sha256": _HASH})),
        "acceptance": _array(_object({"criterion": _TEXT,
            "result": {"type": "string", "enum": ["pass", "fail", "unverified"]},
            "evidence": _array(artifact)})),
        "remaining": text_list, "elapsed_ms": _UINT, "tool_calls": _UINT,
        "learned": text_list,
        "closeout": _object({"learned": text_list, "adr_required": {"type": "boolean"},
                             "rule_change_required": {"type": "boolean"},
                             "destinations": _array(path)}),
    })
    receipt["description"] = (
        "All fields are required, including empty arrays and closeout. Assess each exact "
        "task criterion in order. Evidence must reference canonical files, not prose or local paths. "
        "completed requires all criteria passed and verified evidence; do not fabricate pointers."
    )
    handoff = _object({
        "goal": {"type": "string", "minLength": 1, "maxLength": 2048},
        "constraints": _array(_object({"rule_id": {"type": "string", "minLength": 1,
                         "maxLength": 256}, "version_hash": _HASH}), 1, 64),
        "decided": text_list, "undecided": text_list,
        "required_files": _array(_object({"path": path, "revision": revision}), maximum=256),
        "acceptance_criteria": text_list,
        "forbidden": {**text_list, "minItems": 1},
    })
    handoff["description"] = "Preserve the task's acceptance criteria and frozen policy hash."
    # Revision's serde transparent inner type is [u8; 32], NOT its Display hex string.
    byte_revision = {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 255},
                     "minItems": 32, "maxItems": 32,
                     "description": "Revision wire value: exactly 32 bytes as integers, not hex text."}
    edit = {"oneOf": [
        _object({"kind": {"const": "create"}, "content": _TEXT}),
        _object({"kind": {"const": "patch"}, "expected": byte_revision,
                 "patches": _array(_object({"before": {"type": "string", "minLength": 1},
                                            "after": _TEXT}), 1, 128)}),
    ]}
    batch_id = {"type": "string", "pattern": "^[A-Za-z0-9_-]{1,64}$"}
    batch = _object({"batch_id": batch_id,
                     "changes": _array(_object({"path": path, "edit": edit}), 1, 32)})
    preview = _object({
        "batch_id": _TEXT, "fingerprint": _HASH,
        "diffs": _array(_object({"path": path, "before": {"type": ["string", "null"]},
                                 "after": _TEXT}, required=["path", "after"])),
    })
    preview["description"] = (
        "Copy the entire returned preview unchanged, including namespaced batch_id and diffs. "
        "For commit, keep batch.batch_id as the original caller ID, not preview.batch_id."
    )
    lease = {"type": "integer", "minimum": 0, "maximum": 9223372036854775807,
             "description": "Future Unix timestamp in milliseconds, at most now + 900000 (15 min)."}
    task = identity["properties"]["task_id"]
    operations = {
        "list": _object({}), "get": _object({"task_id": task}),
        "claim": _object({"task_id": task, "lease_until_ms": lease}),
        "heartbeat": _object({"identity": identity, "lease_until_ms": lease,
                              "tool_calls": _UINT, "edits": _UINT}),
        "handoff": _object({"identity": identity, "handoff": handoff}),
        "return": _object({"identity": identity, "receipt": receipt}),
        "cancel": identity,
        "number": _object({"identity": identity, "request_id": {"type": "string", "minLength": 1},
                           "scope": path, "prefix": {"type": "string", "minLength": 1,
                                                      "maxLength": 64, "pattern": "^[^/]+$"}}),
        "batch.preview": _object({"identity": identity, "batch": batch,
                                  "preview": {"anyOf": [preview, {"type": "null"}]}},
                                 required=["identity", "batch"]),
        "batch.commit": _object({"identity": identity, "batch": batch, "preview": preview}),
        "batch.rollback": _object({"identity": identity, "batch_id": batch_id}),
        "memory.preview": _object({"identity": identity, "entry": _object({
            "batch_id": batch_id,
            "scope": {**path, "pattern": "^" + re.escape(project + "/03-记忆") + "(?:/|$)"},
            "kind": {"type": "string", "minLength": 1},
            "summary": {"type": "string", "minLength": 1},
            "date": {"type": "string", "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$", "format": "date"},
            "body": {"type": "string", "minLength": 1},
        })}),
    }
    return operations


def make_board_tool(upstream_tool: dict, project: str) -> dict:
    """Deep-copy one upstream board tool and supply its precise local schema.

    No input object is mutated. The local connector deliberately does not expose
    create. This schema is documentation/validation support, never authority.
    """
    if not isinstance(upstream_tool, dict) or upstream_tool.get("name") != "board":
        raise ValueError("expected_upstream_board_tool")
    if (not isinstance(project, str) or not project.startswith("项目/")
            or any(part in {"", ".", ".."} for part in project.split("/"))
            or "\\" in project or any(ord(c) < 32 for c in project)):
        raise ValueError("invalid_registered_project")
    operations = _schemas(project)
    combined = {}
    for schema in operations.values():
        combined.update(deepcopy(schema["properties"]))
    combined["preview"] = deepcopy(operations["batch.preview"]["properties"]["preview"])
    # Both the union properties and the operation-specific branches are explicit,
    # so clients that render only properties still display the nested structures.
    request = _object(combined, required=[])
    request["description"] = "Exact request shape is selected by operation below; do not guess fields."
    schema = _object({"project": {"type": "string", "enum": [project]},
                      "operation": {"type": "string", "enum": list(OPERATIONS)},
                      "request": request})
    schema["$schema"] = "https://json-schema.org/draft/2020-12/schema"
    schema["allOf"] = [
        {"if": {"properties": {"operation": {"const": operation}}, "required": ["operation"]},
         "then": {"properties": {"request": shape}}}
        for operation, shape in operations.items()
    ]
    tool = deepcopy(upstream_tool)
    tool["inputSchema"] = schema
    tool["description"] = (
        "Studiolo authoritative board. Read get before claim; copy claim identity, do not invent it. "
        "claim request={task_id,lease_until_ms}. "
        "heartbeat request={identity:{task_id,attempt_id,fence},lease_until_ms,tool_calls,edits}; "
        "never flatten identity for heartbeat. "
        "return request={identity:{task_id,attempt_id,fence},receipt}; receipt requires all fields "
        "in the schema including closeout. cancel alone uses flat {task_id,attempt_id,fence}. "
        "list request={}. batch.preview uses {identity,batch}; batch.commit uses "
        "{identity,batch,preview}, copying the exact returned preview without editing it. "
        "Batch patch expected is 32 integer bytes, whereas artifact revision is a string. "
        "The server checks permissions, current lease, frozen policy and actual evidence. "
        "Notification delivery is not task completion. "
        + str(upstream_tool.get("description", ""))
    )
    return tool
