#!/usr/bin/env python3
"""Precise board schema regression tests; no network, model, or writes to Vault."""
from copy import deepcopy
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from local_client.contract import make_board_tool, OPERATIONS

try:
    from jsonschema import Draft202012Validator
except ImportError:
    Draft202012Validator = None

PROJECT = "项目/Studiolo"
TASK = "01M2YH2E6QAV58N0Q4XK22VBDP"
IDENTITY = {"task_id": TASK, "attempt_id": "A-01M2YH3BQ84032CNBEXTDR1FPJ", "fence": 1}
RECEIPT = {"status": "completed", "artifacts": [], "conflicts": [], "unlanded": [],
           "acceptance": [{"criterion": "criterion", "result": "pass", "evidence": []}],
           "remaining": [], "elapsed_ms": 10, "tool_calls": 3, "learned": [],
           "closeout": {"learned": [], "adr_required": False,
                        "rule_change_required": False, "destinations": []}}
BATCH = {"batch_id": "result-1", "changes": [{"path": PROJECT + "/result.md",
         "edit": {"kind": "create", "content": "# Result\n"}}]}
PREVIEW = {"batch_id": "instance-task-result-1", "fingerprint": "a" * 64,
           "diffs": [{"path": PROJECT + "/result.md", "before": None, "after": "# Result\n"}]}


def tool():
    return make_board_tool({"name": "board", "description": "Upstream authority.",
                            "inputSchema": {"type": "object"}}, PROJECT)


def branch(operation):
    return next(item["then"]["properties"]["request"]
                for item in tool()["inputSchema"]["allOf"]
                if item["if"]["properties"]["operation"]["const"] == operation)


class ContractStructureTests(unittest.TestCase):
    def test_deep_copy_preserves_metadata_and_never_mutates_source(self):
        source = {"name": "board", "annotations": {"readOnlyHint": False},
                  "_meta": {"x": ["original"]}, "inputSchema": {"old": True}}
        before = deepcopy(source)
        result = make_board_tool(source, PROJECT)
        result["_meta"]["x"].append("changed")
        result["annotations"]["readOnlyHint"] = True
        self.assertEqual(source, before)
        self.assertEqual(result["name"], "board")

    def test_registration_enum_and_all_operation_branches_match(self):
        schema = tool()["inputSchema"]
        self.assertEqual(schema["properties"]["project"]["enum"], [PROJECT])
        self.assertEqual(schema["properties"]["operation"]["enum"], list(OPERATIONS))
        self.assertNotIn("create", OPERATIONS)
        self.assertEqual(len(schema["allOf"]), len(OPERATIONS))
        self.assertEqual({x["if"]["properties"]["operation"]["const"] for x in schema["allOf"]},
                         set(OPERATIONS))
        self.assertNotIn("discriminator", json.dumps(schema))
        self.assertEqual(set(schema["required"]), {"project", "operation", "request"})

    def test_nested_heartbeat_and_return_identities_are_required(self):
        heartbeat = branch("heartbeat")
        self.assertEqual(set(heartbeat["required"]), {"identity", "lease_until_ms", "tool_calls", "edits"})
        self.assertEqual(set(heartbeat["properties"]["identity"]["required"]),
                         {"task_id", "attempt_id", "fence"})
        self.assertFalse(heartbeat["additionalProperties"])
        self.assertEqual(set(branch("return")["required"]), {"identity", "receipt"})
        self.assertEqual(set(branch("claim")["required"]), {"task_id", "lease_until_ms"})
        self.assertEqual(set(branch("cancel")["required"]), {"task_id", "attempt_id", "fence"})

    def test_complete_receipt_closeout_and_handoff_are_visible(self):
        request = tool()["inputSchema"]["properties"]["request"]["properties"]
        self.assertEqual(set(request["receipt"]["required"]), set(RECEIPT))
        closeout = request["receipt"]["properties"]["closeout"]
        self.assertEqual(set(closeout["required"]), set(RECEIPT["closeout"]))
        handoff = request["handoff"]
        self.assertEqual(set(handoff["required"]), {"goal", "constraints", "decided", "undecided",
                         "required_files", "acceptance_criteria", "forbidden"})
        acceptance = request["receipt"]["properties"]["acceptance"]["items"]
        self.assertEqual(acceptance["properties"]["result"]["enum"], ["pass", "fail", "unverified"])

    def test_batch_commit_requires_preview_and_patch_revision_is_bytes(self):
        self.assertEqual(set(branch("batch.preview")["required"]), {"identity", "batch"})
        commit = branch("batch.commit")
        self.assertEqual(set(commit["required"]), {"identity", "batch", "preview"})
        variants = commit["properties"]["batch"]["properties"]["changes"]["items"]["properties"]["edit"]["oneOf"]
        patch = next(x for x in variants if x["properties"]["kind"]["const"] == "patch")
        expected = patch["properties"]["expected"]
        self.assertEqual((expected["type"], expected["minItems"], expected["maxItems"]), ("array", 32, 32))
        self.assertEqual(expected["items"], {"type": "integer", "minimum": 0, "maximum": 255})
        self.assertEqual(set(branch("batch.rollback")["required"]), {"identity", "batch_id"})

    def test_descriptions_explain_shapes_instead_of_guessing(self):
        description = tool()["description"]
        for expected in ("heartbeat request={identity:", "return request={identity:",
                         "claim request={task_id,lease_until_ms}", "cancel alone uses flat",
                         "32 integer bytes", "exact returned preview", "closeout"):
            self.assertIn(expected, description)

    def test_invalid_tool_or_project_rejected(self):
        for name in ("other", None):
            with self.assertRaises(ValueError):
                make_board_tool({"name": name}, PROJECT)
        for project in ("", "项目/", "项目/../x", "项目/a\nx", "other"):
            with self.assertRaises(ValueError):
                make_board_tool({"name": "board"}, project)


@unittest.skipUnless(Draft202012Validator, "jsonschema not installed; structural checks still run")
class ContractValidationTests(unittest.TestCase):
    def setUp(self):
        schema = tool()["inputSchema"]
        Draft202012Validator.check_schema(schema)
        self.validator = Draft202012Validator(schema)

    def valid(self, operation, request):
        return self.validator.is_valid({"project": PROJECT, "operation": operation, "request": request})

    def test_claim_heartbeat_and_return_valid_shapes(self):
        self.assertTrue(self.valid("claim", {"task_id": TASK, "lease_until_ms": 1800000000000}))
        self.assertTrue(self.valid("heartbeat", {"identity": IDENTITY, "lease_until_ms": 1800000000000,
                                               "tool_calls": 4, "edits": 0}))
        self.assertTrue(self.valid("return", {"identity": IDENTITY, "receipt": RECEIPT}))

    def test_flattened_missing_or_incomplete_heartbeat_identity_rejected(self):
        base = {"identity": IDENTITY, "lease_until_ms": 1800000000000, "tool_calls": 4, "edits": 0}
        for key in base:
            invalid = deepcopy(base)
            del invalid[key]
            self.assertFalse(self.valid("heartbeat", invalid))
        flat = {**IDENTITY, "lease_until_ms": 1800000000000, "tool_calls": 4, "edits": 0}
        self.assertFalse(self.valid("heartbeat", flat))
        incomplete = deepcopy(base)
        del incomplete["identity"]["fence"]
        self.assertFalse(self.valid("heartbeat", incomplete))

    def test_return_requires_identity_all_receipt_fields_and_closeout(self):
        self.assertFalse(self.valid("return", {"receipt": RECEIPT}))
        self.assertFalse(self.valid("return", {"identity": IDENTITY}))
        for key in RECEIPT:
            invalid = deepcopy(RECEIPT)
            del invalid[key]
            self.assertFalse(self.valid("return", {"identity": IDENTITY, "receipt": invalid}))
        invalid = deepcopy(RECEIPT)
        del invalid["closeout"]["adr_required"]
        self.assertFalse(self.valid("return", {"identity": IDENTITY, "receipt": invalid}))

    def test_operation_specific_fields_and_project_are_enforced(self):
        self.assertTrue(self.valid("list", {}))
        self.assertFalse(self.valid("list", {"task_id": TASK}))
        self.assertTrue(self.valid("cancel", IDENTITY))
        self.assertFalse(self.valid("cancel", {"identity": IDENTITY}))
        self.assertFalse(self.valid("create", {}))
        self.assertFalse(self.validator.is_valid({"project": "项目/Other", "operation": "list", "request": {}}))

    def test_batch_preview_commit_and_patch_wire_shapes(self):
        base = {"identity": IDENTITY, "batch": BATCH}
        self.assertTrue(self.valid("batch.preview", base))
        self.assertTrue(self.valid("batch.preview", {**base, "preview": None}))
        self.assertFalse(self.valid("batch.commit", base))
        self.assertFalse(self.valid("batch.commit", {**base, "preview": None}))
        self.assertTrue(self.valid("batch.commit", {**base, "preview": PREVIEW}))
        patch = deepcopy(base)
        patch["batch"]["changes"][0]["edit"] = {"kind": "patch", "expected": [0] * 32,
                                                  "patches": [{"before": "x", "after": "y"}]}
        self.assertTrue(self.valid("batch.preview", patch))
        patch["batch"]["changes"][0]["edit"]["expected"] = "0" * 64
        self.assertFalse(self.valid("batch.preview", patch))

    def test_handoff_and_memory_have_exact_required_fields(self):
        handoff = {"goal": "Next", "constraints": [{"rule_id": "rule-1", "version_hash": "a" * 64}],
                   "decided": [], "undecided": [], "required_files": [],
                   "acceptance_criteria": ["criterion"], "forbidden": ["do not deploy"]}
        self.assertTrue(self.valid("handoff", {"identity": IDENTITY, "handoff": handoff}))
        del handoff["constraints"]
        self.assertFalse(self.valid("handoff", {"identity": IDENTITY, "handoff": handoff}))
        entry = {"batch_id": "memory-1", "scope": PROJECT + "/03-记忆", "kind": "lesson",
                 "summary": "One thing", "date": "2026-09-20", "body": "Content"}
        self.assertTrue(self.valid("memory.preview", {"identity": IDENTITY, "entry": entry}))
        entry["scope"] = PROJECT + "/04-协调"
        self.assertFalse(self.valid("memory.preview", {"identity": IDENTITY, "entry": entry}))


if __name__ == "__main__":
    unittest.main()
