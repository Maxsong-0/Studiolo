# M6 Runner / Gateway implementation and acceptance boundary

> 当前任务约束已变更：用户仅允许订阅额度，禁止 API 计费。本文 API relay/key-holder 部分保留为历史实现说明，不能作为当前测试启用指令。订阅认证尚未适配，见 [subscription-only.md](subscription-only.md)。

T-091 adds code for two CLI launch profiles and a durable real-call gateway. **This is not a deployed M6 acceptance receipt.** Neither dedicated provider credential has been supplied to this task. The inspected CI host has systemd/cgroup v2 but no `bwrap`, `claude` or `codex`; it must not receive production credentials.

## Trusted assembly

`RunSpec` gains defaulted `budget` and optional `execution`. Old records remain Stub-only. `ExecutionSpec` binds provider, exact model, snapshot, policy hash, every supplied input label, and the complete source checkout tree plus its authenticated egress label. Classification must come from the policy authority, never source frontmatter. `Deny` and `LocalOnly` reject both real providers; Stub remains available.

The scheduler constructs one `RealRunner` per attempt with an administrator-mapped checkout, bounded private scratch filesystem, durable retained-candidate directory, pinned native CLI SHA-256, and a per-attempt short-lived capability file. `RunnerRef` uses the attempt/fence-specific systemd scope identity, not a reusable PID. `RealRunner` only runs the provider's pinned profile. `apply_code_patch` and `advance_code_ref` are separate trusted landing APIs, never invoked by the Runner.

`DurableRelay::open(database, raw_root, RelayServices)` starts frozen. Services require:

- Fresh `AttemptGuard` plus `RelayPolicyGuard`: the latter must read the authoritative live lease, active effective-policy hash, immutable snapshot membership/classification, and admin-pinned rate ceilings on **every** call and stream check. A permissive fixture is not a production implementation. When the Board-backed guard's authorization recheck cannot read the identity store, policy store or Board control storage, it answers `RelayError::Storage` (503, retry later); a definite authorization denial stays `RelayError::Denied` (403) (BUG-032). The claim-ownership check still maps Control DB read errors to `Denied`; that is tracked separately.
- Real append-only `AuditLedger`.
- Four-layer deployment/policy/role/capability permission intersection.
- `RemoteRelayTransport` to a separate-UID key-holder using `HttpsRelayTransport` with two **dedicated** Gateway-only keys; the canonical application process never reads those key files.
- `RelayPublisher::seal`: success is legal only after authoritative source classification and a durable Broker artifact/manifest marker. It must be idempotent by attempt/call. No permissive production default exists.

Issue the random `RelayTicket` only after trusted assembly is unfrozen, deliver it through a private attempt file, and mount it read-only at `/run/job-capability`. Never serialize it into `RunSpec`, receipts or logs. The Runner's provider-auth environment contains this short-lived ticket only. `serve_relay_connection` serves a bounded HTTP/1.1 request on an already accepted Unix connection; the owner must protect its parent directory and socket, cap accepted concurrency, and own the listener lifecycle. It exposes no TCP server.

## Linux containment

`LinuxIsolation` executes `systemd-run --user --scope` plus bubblewrap user, PID, mount, IPC, UTS and network namespaces. It maps only the runtime root, the attempt work copy, read-only declared inputs, the Gateway socket, and a read-only cgroup resource view. HOME and `/tmp` are separate bounded tmpfs mounts. Rootfs is read-only; capabilities are dropped; the CLI is UID/GID 65534 inside its private user namespace. The host service account is unprivileged.

Live `probe()` runs the exact embedded `rg_probe.py` in this profile. A runtime boolean or uploaded JSON receipt cannot make it Ready. It checks credentials/control paths are absent, root/input writes fail, direct public/private/metadata-network connects fail, namespace identity differs, Linux effective capabilities are zero, actual CPU/PID/memory ceilings apply, scratch filesystem capacity is bounded, and the only declared external transport is a Unix socket. Probe results describe these local checks; fence/revocation and real CLI interoperability remain separate gates.

Required administrator preparation: a minimal immutable rootfs with pinned **native** CLI binaries and Python, embedded scripts at `/usr/lib/studiolo/`, `/work`, `/inputs`, `/run`, `/home`, `/tmp`, `/proc`, `/dev`, `/sys/fs/cgroup`; a service-account user systemd manager with delegated CPU/memory/PID controllers; a private scratch tmpfs no larger than 512 MiB; a durable private retained directory; and Gateway-only raw storage excluded from backup and desktop sync. Do not bind the host root, user HOME, personal CLI directories, Docker socket, canonical Vault, audit, credentials or Git administrative directories. There is no installation script that silently downloads or enables these prerequisites.

## Call ordering and budgets

Real relay requests authenticate an opaque 256-bit nonce stored only as a digest, recheck current fence/cancellation/lease/policy/egress, validate a fixed path/provider/model, then reserve worst-case spend and a call slot in SQLite WAL with FULL sync. New processes begin frozen. Pending/unknown states block subsequent calls for that attempt, including a different call ID. No automatic replay can bypass an unknown external effect.

The request body cannot select an upstream base URL, background operation, stored conversation, remote-file target, hosted web/computer/MCP tools or native subagents. TLS endpoints are fixed to the two official API hosts, redirects and environment proxies are disabled, and **no incoming authentication headers are forwarded**. Provider authentication is injected only in the transport. The gateway reads SSE incrementally under byte, connect, idle and total-time ceilings. A completed protocol terminal event is required; truncation/errors are unknown outcomes.

The response remains bounded and withheld until egress recheck, model-emitted tool-count reservation, private raw fsync, Broker publication, audit outcome and durable completion. Replays revalidate current authorization and never repeat network I/O. Monetary charging is deliberately conservative: every call keeps its full worst-case reservation, including an unknown/failed call. Trusted input/output token rate ceilings must cover the exact model; raw request bytes conservatively bound input tokens. Tool actions are counted before model responses reach the CLI, enforcing the hard tool ceiling. Gateway request injection provides pacing/wrapup notices for both protocols, while the CLI supervisor also observes tool/edit progress and duration.

Raw filenames carry an expiry no later than one hour or source/capability retention. A separate 30-second cleanup thread expires them, with a startup sweep and a 128 MiB admission check. The deployment must also put raw storage on a bounded ephemeral filesystem and clean it while the Gateway is stopped; an in-process timer cannot enforce retention while the host is down.

## Artifacts and cancellation

Host-owned snapshot capture uses a temporary Git index, `add -A`, `write-tree`, anonymous `commit-tree -p HEAD`, and a detached worktree. It includes dirty/untracked nonignored content without changing the source checkout index or branch refs. Local filter/include configuration is rejected before host-side indexing. Git metadata and CLI config directories are removed from the mounted work copy. The final binary patch carries base/after tree IDs and SHA-256; candidates are fsynced outside scratch before reclamation. Conflict-marker candidates are retained and rejected. Invalid special files leave the work copy intact for explicit inspection.

Code landing checks expected HEAD and candidate digest, attempts `git apply`, then `--3way`, reports unmerged/marker paths and retains the candidate. The caller must hold an exclusive repository landing lease. The optional ref operation always passes expected-old to `git update-ref`.

Cancellation revokes Gateway authority in TaskEngine before Adapter cancellation. Adapter writes a host-owned cancellation marker, allowing the supervisor to request a stopped receipt while enforcing a 180-second scope reap ceiling. The complete final JSONL event is drained before launch-failure classification; launcher protocol/output errors are not retryable handshakes. **A full grace-window CLI receipt, orphan cleanup/restart adoption, shared snapshot caching and provider-specific CLI handshake compatibility have not been deployment-accepted.** The current code must not be advertised as completing those checks.

## Validation still required

The concrete application assembly is now implemented in `local-server/src/board/executor`; see [integration](integration.md). Root integration must run the Rust contract tests and clippy on Linux. This subtask did not start builds/tests under the coordination instruction. Deployment must separately demonstrate both pinned CLIs through the real Unix bridge, dedicated-key upstream calls, exact replay/unknown behavior, policy/fence/lease revocation during streaming, spend/tool/duration cutoff, escape attempts, post-cancel cgroup emptiness, retained patch correctness, and no personal login material. Until these pass, real adapters remain unavailable/disabled in the production composition.

Protocol references verified during implementation: [Anthropic SSE](https://platform.claude.com/docs/en/build-with-claude/streaming) and [OpenAI Responses SSE](https://platform.openai.com/docs/api-reference/responses-streaming). Local `claude --help`, `codex --help`, and `codex exec --help` were read without logging into either provider. Current Claude `--bare` help requires explicit API-key authentication/helper; the Runner uses its short-lived Gateway ticket in that protocol field.
