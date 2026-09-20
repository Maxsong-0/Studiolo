# Trusted RunnerPlane assembly

> 当前任务约束已变更：用户仅允许订阅额度，禁止 API 计费。本文 API relay/key-holder 部分保留为历史实现说明，不能作为当前测试启用指令。订阅认证尚未适配，见 [subscription-only.md](subscription-only.md)。

`local-server::board::executor` is the production composition boundary. The application must expose it only inside the existing authenticated owner session and L3 approval path. The Consumer identity in `StartRequest` identifies who initiated the claim; the isolated process receives a separate attempt/fence-scoped capability, never that Consumer's machine token.

## Required application calls

1. Read `RunnerConfig` from an owner-controlled, canonical absolute, 0600 private configuration file. There are **no provider-key fields** in this config. Omitted config means unavailable.
2. After ControlDb opens, call `isolate_persisted(&control, &config, &isolated_roots)` before `reconcile_startup`. It first calls public `validate_isolated_roots(&config, &isolated_roots)`, before any process signal; the same validation includes runtime roots at open. Include all Vault, Broker, control, auth, audit, policy, agent state, host config and assets roots. It accepts only exact persisted Claude/Codex scope names derived from attempt/fence, kills those scopes and confirms inactivity/cgroup emptiness. Individual systemctl subprocesses have two-second limits; the whole unit check has a bounded deadline. Unknown/legacy unsafe references reject startup.
3. `RunnerPlane::open(board: Arc<BoardPlane>, config) -> Result<Arc<RunnerPlane>, String>` creates the protected Unix listener and opens the frozen durable relay. It rejects source/scratch/raw/retained/state roots that overlap the canonical Vault, Broker data or blackboard Control DB.
4. Call `.serve()` once. `.is_frozen()` and `.replay()` are read-only status calls and do not run probes.
5. Owner L3 approval calls `.freeze(false)`. Every configured pinned CLI/runtime must pass its live containment probe and the key-holder peer UID must match a **different unprivileged UID**. A failed check leaves execution closed. `.freeze(true)` revokes the shared gate immediately.
6. Owner-approved `.start_task(StartRequest { task_id, agent_type, instance_id })` performs fresh lineage/project write authorization, explicit claim, source classification checks, and persists the exact RunnerRef before issuing a short ticket or starting the process. `.cancel(task_id)` commits cancellation before process signaling.
7. Call `.tick()` in `spawn_blocking` every 5–15 seconds, outside the Board engine lock. It validates live authority, advances counters/leases, stops revoked runs, and calls `stage_board_runner_candidates` on completed CLI output. This yields `NeedsReview`, never task success. A staging rejection or size bound failure leaves the local candidate retained, revokes the grant/ticket, isolates the scope and reports `candidate_retained`; remaining attempts continue supervision. Any existing T1 reservation remains fenced for engine roll-forward. Cleanup failures are retried without renewing a cancelled lease. Actual owner acceptance uses the separate L3 engine review API; no synthetic passing acceptance assessments are generated.
8. Call `.shutdown()` before `workbench.shutdown()`. It freezes admissions, revokes/unlinks short tickets, attempts isolation for every scope even when one fails, joins the Unix serving thread and removes its closed front socket. A stopped Gateway/host needs its independent raw-cleanup timer; in-process TTL cleanup alone is insufficient while the host is down.

## Strict host schema

All nested schemas reject unknown fields. `RunnerConfig` contains:

- `schema: 1`, `state_root`, `raw_root`, `upstream_socket`, `upstream_uid`.
- `deployment_tools`: normally only `model.relay`.
- `policies`: each exact `hash`, `role`, `policy_tools`, `role_tools`. The hash is checked against the live effective policy and every initiating instance ancestor on every relay boundary. A changed policy requires reapproval/reconfiguration.
- `providers`: `provider` (`claude` or `codex`), exact `model`, `runtime_root`, native `cli_sha256`, `bubblewrap`, `systemd_run`, private bounded `attempt_root`, durable private `retained_root`, `budget`, and `relay_limits`. The model/provider and rates cannot come from a Runner request.
- `projects`: exact logical `project`, administrator-mapped `repository`, full dirty/untracked `repository_tree`, `repository_sha256`, `repository_egress`, `retain_until_unix_seconds`, `sources`, `approved_instruction_sha256`, `instruction_egress`, `allow_full_artifact_backup`, `allow_artifact_desktop_sync`.
- Each `sources` entry pins logical `path`, exact `revision`, `sha256`, `egress`, and `retain_until_unix_seconds`. An unlisted/new revision, unclassified task instruction, missing data label, or expired source rejects cloud execution. `deny` and `local_only` never become cloud-allowed. Task instructions are separately hashed and classified because manually pasted prose is also data leaving the trust domain.

The two explicit artifact permissions must be true before generated code can be staged into canonical history and desktop sync; allowing model egress alone does not silently authorize those additional destinations. No default configuration grants any of these permissions.

`budget` contains `pacing_tools`, `wrapup_tools`, `max_tools`, `wrapup_seconds`, `max_seconds`. `relay_limits` contains request/response byte ceilings, `response_tokens`, `calls`, the same tool thresholds, `budget_microusd`, `reservation_microusd`, conservative positive `input_token_microusd` and `output_token_microusd`, and `timeout_seconds`. Reservation must cover the worst-case bound derived from these limits; the Gateway conservatively retains the full reservation for failed or unknown requests.

## Independent key-holder

Build `studiolo-gateway` binary `gateway-model-relay`. Run it as a dedicated non-root user which cannot read Vault, Control DB or authoritative audit files. Its canonical 0600 config has exactly:

```json
{
  "schema": 1,
  "socket": "/run/studiolo-model-relay/model.sock",
  "client_uid": 1001,
  "anthropic_key_file": "/run/credentials/studiolo-model-relay/anthropic",
  "openai_key_file": "/run/credentials/studiolo-model-relay/openai"
}
```

These are illustrative paths/UIDs, not a deployment instruction or real secret values. The socket directory belongs to the key-holder, grants the front process group traversal only, and is not group-writable (for example 0750); the socket is 0660. Key/config files are 0600 and owned by the key-holder. The main app's `upstream_uid` must match this account and differ from its own non-root UID. Both sides enforce Linux SO_PEERCRED. The Runner mount contains only the front relay socket; it never contains this upstream socket or either long-lived key file.

Before reading config or either key, the key-holder calls `studiolo_process_safety::harden()` and records only its fixed residency classifications; any supported hardening failure rejects startup. The key-holder accepts only the pinned front UID and fixed two-provider protocol, injects provider auth, and streams internal frames. The credential-free front checks current policy/fence/lease/freeze on every received chunk, preserves the durable intent/unknown protocol, and releases SSE to the Runner only after metadata-only `GatewayPublisher::seal_gateway_call` and authoritative outcome. Raw stays in Gateway ephemeral storage. Per-call manifests are private Broker metadata and never consume the final attempt's artifact manifest.

## Exact unverified boundaries

This code does not supply provider keys, install CLIs/rootfs, delegate cgroup controllers, configure actual dedicated users or launch production services. Linux contract/clippy acceptance is owned by the root integration pass. Real two-provider model execution, actual provider CLI handshake compatibility, escape/resource probes on the production account, complete cancellation receipt behavior, restart/orphan cleanup and end-to-end owner candidate acceptance remain separate deployment checks. No boolean config field or fixture can stand in for these checks.

The CLI bridge's idempotency key is SHA-256 of the fixed provider path, a zero separator and the exact request body. The same body within an attempt is explicitly a retry of the same logical operation and replays its sealed result; it cannot request a fresh identical completion. A changed body after an unresolved upstream intent is denied as indeterminate as well. This conservative behavior can reject a legitimate repeated prompt and is part of the adapter contract, not a provider exactly-once guarantee.
