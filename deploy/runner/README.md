# M6 isolated runtime assets

The runtime assets `launch.py` and `rg_probe.py` contain no provider keys and do not install or enable services. Their presence is not proof that a machine passed RG-RUNNER.

For the explicitly selected githubci test host, `stage-githubci.py` assembles a credential-free runtime and `install-githubci.py` is a separate, explicit sudo installation step. Its `--plan` mode is read-only. It installs bubblewrap, a dedicated test key-holder account, bounded tmpfs mounts and independent raw cleanup; it leaves the key-holder stopped. See `docs/m6/githubci-acceptance.md` for exact paths, checked assets and the remaining real acceptance gates.

**当前用户只允许订阅额度，不使用 API。** `configure-githubci-keys.py` 已停用，任何调用都返回 1，不读凭据、不启服务。下述 API key-holder 为先前实现记录，当前测试不得启用；订阅适配边界见 `docs/m6/subscription-only.md`。

The administrator creates an immutable minimal runtime root, installs pinned native `claude` and `codex` binaries under `/usr/local/bin`, Python at `/usr/bin/python3`, and copies `launch.py` and `rg_probe.py` to `/usr/lib/studiolo/`. The root and binaries must not be writable by the Runner service account. Compute and pin each native binary SHA-256 in trusted deployment configuration. Do not copy personal CLI login directories or user config.

Run the compiled `runner-adapter` `probe` example under the actual unprivileged service identity:

```text
probe claude ROOTFS CLAUDE_BINARY_SHA256 SCRATCH RETAINED GATEWAY_SOCKET
probe codex ROOTFS CODEX_BINARY_SHA256 SCRATCH RETAINED GATEWAY_SOCKET
```

Prerequisites and exact assembly boundaries are in `docs/m6/runner-gateway.md`. A missing binary, user namespace, cgroup limit, bounded scratch filesystem, socket, or fixed probe asset keeps the adapter unavailable. The scratch root must be a dedicated private tmpfs of at most 512 MiB; do not point it at a general-purpose disk. Mount the Gateway raw area separately with a bounded capacity, no backup/sync, and a system cleanup timer that operates even if the Gateway service is stopped.

After local containment passes, separately record negative Gateway fence/lease/policy/budget/freeze probes, cancellation/reaping, patch retention, and real dedicated-key protocol runs for both CLIs. A local `Ready` containment probe is not the complete release-gate receipt. No production credential belongs on the shared CI host.

The dedicated upstream process is the `studiolo-gateway` binary `gateway-model-relay`; its private config and the credential-free application configuration are described in `docs/m6/integration.md`. Neither key file is read by local-server.
