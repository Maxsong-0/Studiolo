# M6 githubci test environment

> 最新目标已澄清为调用用户本机现有的 Codex / Claude Code，见 [local-client-dispatch.md](local-client-dispatch.md)。此页保留服务器隔离测试证据，不作为当前本机派发链路的验收或后续安装指令。

2026-09-20 UTC. The owner explicitly chose the existing `githubci` host, `maxsong@192.168.100.107`, for M6 real testing. No other network or new host is required. Installation and both real CLI containment probes are verified. The owner now requires subscription quota only and prohibits API billing; the previous API credential instructions are revoked. Subscription authentication and real model calls remain unverified.

## Current evidence

Completed without sudo:

- Connected to the real Linux x86_64 host; systemd user manager is running, cgroup v2 controllers are present, `user@1000.service` is delegated. At initial inspection no `Runner.Worker` was running; the existing CI listener was left alone.
- Approximately 28 GiB disk and 29 GiB available memory at preflight. Linux 7.0.0-31-generic, Python 3.14.4, glibc 2.43.
- Built `studiolo-runner-adapter --example probe` and `studiolo-gateway --bin gateway-model-relay` with `scripts/test-linux.sh`, using the existing per-worktree Linux cache. Build passed.
- Downloaded the official Codex **0.155.1** complete native package and Claude **2.1.236** native Linux binary. Verified their fixed SHA-256 values. Verified Claude's signed manifest in a temporary GnuPG home against primary fingerprint `31DDDE24DDFAB679F42D7BD2BAA929FF1A7ECACE`.
- Assembled the rootfs from those packages, explicitly selected system tools, Python standard library and 29 resolved system libraries. No user home, personal CLI config or credentials were copied. The complete Codex package layout, including sidecars, is preserved; optional voice components were not tested.
- Rechecked all **730** staged runtime/build files against the manifest. Real Linux CLI `--version` and required help flags passed with a newly created, empty HOME. These calls did not send model requests.
- Both preparation scripts passed Python syntax checks, and the installer `--plan` returned the expected fixed paths without mutation.

The owner then completed the sudo installation. Readback confirmed bubblewrap 0.11.1, all 730 installed files with the expected hashes and root ownership, the new key-holder UID **995** / GID **983**, config mode 0600, and both correctly sized, private, `noswap` tmpfs mounts. The raw-cleanup timer is active; Gateway remains inactive. The distribution bubblewrap works with the existing host user-namespace policy; no global AppArmor/sysctl relaxation was made.

`sudo -n /usr/bin/true` returned `sudo: interactive authentication is required`. The existing passwordless helper only manages CI runner services and is outside this installation's purpose. The user must enter the sudo password in their own SSH terminal; it is never requested in chat or handled by the agent.

## 安装后的真实隔离验收

首次运行两家编译后的探针均返回 `unavailable`。捕获同一命令的错误发现：`bwrap: Can't create file at /run/studiolo-gateway.sock: Read-only file system`。这是真实 rootfs 组装暴露的 Adapter 问题，不是模型认证失败。已修复 `crates/runner-adapter/src/isolation.rs`：在绑定唯一 Gateway socket 前，先挂载限额 **1 MiB** 的私有 `/run` tmpfs，不要求不可变 rootfs 预置 socket 文件。

修改后，原生 Linux 格式检查、Runner 全目标严格 Clippy、**26 项测试**及探针构建均通过。实际复跑两个固定版本的探针，均得到 `availability: ready`、真实版本和完整必需旗标，缺失项为空。探针实际断言：namespace 内 UID/GID 65534、能力集为零、网络 namespace 不同、三类直连目标失败、敏感路径不可见、根与输入只读、work 容量不超过 512 MiB，以及实际 cgroup 的内存不超过 1 GiB、进程数不超过 64、CPU 不超过 2 核。

此次使用的是**仅供隔离检查的本地 Unix socket 占位**，未启动 Gateway，没有上游请求、密钥或计费；`ready` 只证明本机隔离与 CLI 接口，不代表完整 RG-RUNNER 或真实模型协议验收。

独立清理验收：在 raw tmpfs 创建两个无敏感数据的夹具，一份已过期、一份未过期。Gateway 保持 inactive，系统 timer 在 **17 秒**内删除过期文件并保留未过期文件；清理 service 的 `Result=success`、`ExecMainStatus=0`。本次夹具、socket、attempt 目录和 systemd probe scope 均已收尾。

实际受测探针来自修复后构建目录：

```text
/home/maxsong/.cache/studiolo-target/codex-m5-m6-blackboard-runner-bdfe440acfbc/debug/examples/probe
SHA-256: 592690ee76459ce03958a3a43638e5c85994ffa5223fbc07c52da67c5dfa6507
```

`/opt/studiolo-m6-test/bin/probe` 仍是初次安装的旧辅助程序，本次通过结果不归属于它。rootfs、CLI 与 Gateway 二进制未改变；后续部署应用和探针必须使用包含本次修复的新构建。

远端证据：`/var/lib/studiolo-m6-test/state/containment-initial.json`、`containment-fixed.json`、`raw-timer-acceptance.json`。本地定向检查日志：`/private/tmp/studiolo-m6-containment-fix.log`。

## Exact paths and completed installation step

The already populated private preparation directory is:

```text
/home/maxsong/.cache/studiolo-m6-setup/20260920
```

The owner already ran this from the Mac terminal. It is create-only; do not repeat it on the installed host:

```sh
ssh -t maxsong@192.168.100.107 'sudo python3 /home/maxsong/.cache/studiolo-m6-setup/20260920/install-githubci.py'
```

The create-only installer reads no API credentials and does not start or enable Gateway. It verifies a private copy of the staged files before installation, installs the signed-distribution `bubblewrap` package, creates only the following test-specific resources and enables the two mounts plus raw-cleanup timer. Existing paths/accounts/units cause refusal, so inspect any partial installation before retrying; this is not an unattended upgrade script.

| Purpose | Fixed location / identity |
| --- | --- |
| Immutable runtime and compiled test binaries | `/opt/studiolo-m6-test/{runtime,bin}` |
| Trusted front / test driver | Existing `maxsong`, UID 1000; separate private test data |
| Key-holder identity | New system account `studiolo-m6-gateway`, no login/home |
| Per-attempt tmpfs | `/var/lib/studiolo-m6-test/attempts`, 512 MiB, no swap |
| Gateway raw tmpfs | `/var/lib/studiolo-m6-test/raw`, 128 MiB, no swap |
| Retained candidates / state / front transport | `/var/lib/studiolo-m6-test/{retained,state,transport}` |
| Gateway config | `/etc/studiolo-m6-test/gateway.json`, key-holder-owned 0600 |
| Dedicated test credentials, initially absent | `/var/lib/studiolo-m6-test/credentials/{anthropic,openai}` |
| Internal key-holder socket, absent while stopped | `/run/studiolo-m6-keyholder/model.sock` |
| Manual-start Gateway unit | `studiolo-m6-keyholder.service` |
| Independent raw expiry cleanup | `studiolo-m6-raw-clean.timer`, every 30 seconds |

### 计费约束已变更：只用订阅额度

用户明确「只允许用订阅额度，不使用API」，覆盖之前专用 API 凭据方案。API key-holder 已回读为 `inactive/dead`，没有发起模型调用。不得再执行 API 录入或启用步骤。

`configure-githubci-keys.py` 已在仓库和远端私有准备目录替换为停用入口：所有调用固定返回退出码 1，不读取凭据、不写文件、不启动服务。因此此前带 `&& systemctl start` 的命令会在第一步停止。原交互式录入工具的语法/门禁验证只属于已撤销的历史方案。

固定 rootfs、资源隔离、CLI 版本和 raw 清理证据继续有效，但它们不证明订阅登录或订阅模型执行。现有 Adapter 配置仍是 API relay，不能通过把 OAuth 放进 API key 字段或直接开放出网冒称已适配订阅。后续边界和官方依据见 [subscription-only.md](subscription-only.md)。

The installer leaves CI runner units, repository variables, sudo policy and global AppArmor/sysctls unchanged. It does not copy or run a vendor install script as root. New mount units are generated using `systemd-escape`; raw cleanup honors the application's filename deadline and remains independent of the Gateway process.

## Pinned evidence and source

| Artifact | SHA-256 |
| --- | --- |
| Codex complete package archive | `a65b895c6ac1a73629bbe4b864640c86133e94a43b4d67b3103044e1a306d5a2` |
| Extracted Codex native ELF (the Runner pin) | `0753dfe1d8b87a52436deb13eb1c549661ef4c84fee2c5aa688385eebeccb761` |
| Claude native ELF (the Runner pin) | `6c8818fa22187aa555c242be4abbacc44d6b71a32ac9631ee7b2b5d12f51f752` |
| Staged `install-githubci.py` | `a2fa23ec4665ba5b6c3be5fcb282c2d9d52d480e401379019c02be45d86d715d` |

Official assets: [Codex package](https://github.com/openai/codex/releases/download/rust-v0.155.1/codex-package-x86_64-unknown-linux-musl.tar.gz), [Claude binary](https://downloads.claude.ai/claude-code-releases/2.1.236/linux-x64/claude), [Claude manifest](https://downloads.claude.ai/claude-code-releases/2.1.236/manifest.json), [manifest signature](https://downloads.claude.ai/claude-code-releases/2.1.236/manifest.json.sig), [signing key](https://downloads.claude.ai/keys/claude-code.asc). Codex's archive digest was verified against official release metadata; its separate Sigstore signature was not verified and is not claimed as such.

Remote `files.json` and `preflight.json` retain per-file hashes and real CLI checks. Local build log: `/private/tmp/studiolo-m6-githubci-build.log`. To reproduce staging, obtain and verify those fixed assets in `downloads/`, place `launch.py` and `rg_probe.py` next to `stage-githubci.py`, then run it as the unprivileged test driver with the stage directory and compiled Linux debug directory. Assembly is create-only.

## Next acceptance gates

1. 完成官方原生 CLI 的订阅认证适配，明确账号登录、模型客户端与隔离工具进程的信任边界；保留已有来源授权、fence/lease、冻结、预算、审计及取消要求。未适配前保持真实执行关闭。
2. 在用户自己的官方登录流程内完成测试配置的订阅登录；不能复制个人登录目录或由自建 Gateway 收集订阅 OAuth 当 API 密钥使用。验证实际计费为订阅套餐额度，并阻止 API、credits、extra usage 付费回退。
3. 使用修复后的 Adapter 构建完成真实模型、取消、失效 fence/lease/policy、预算、冻结、候选留存及人侧 L3 验收。初次安装的旧 probe 程序不能代表新构建结果。


This preparation changes neither the existing M5 Consumer test receipt nor the distinction between local implementation checks, isolated Runner acceptance and production release. Work remains uncommitted/unmerged/undeployed to production.
