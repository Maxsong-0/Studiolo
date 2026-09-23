# M4 策略平面与权限中枢（T-069，2026-09-17）

本目录记录 M4 的实现范围、运行方式与验收边界。契约在 [`contracts/policy-plane.md`](../contracts/policy-plane.md)；
产品依据在 vault `ADR-019 策略平面存储与写者归属`、`T-069-策略平面动作登记` 与 `roadmap` M4 行。

**状态口径（2026-09-17）**：代码已实现，macOS 全工作区 182 目标 2293 项通过，Linux 由 PR #10 的 CI（x86_64）验证；**线上未启用**——
服务器上的 `studiolo-agent` 仍以 M3 兼容模式运行（未给 `--policy`），人侧因子模式（发布所需的 L3 路径）
尚未部署。启用是部署动作，前置见文末。

## 做了什么

| 部分 | 内容 |
|---|---|
| `crates/policy` | 私有库 `policy.sqlite3`；三类母本（规则母本 Markdown、skill JSON、能力集 JSON）；desired 校验、diff 与权限扩大项；CAS 发布、回滚、首版 bootstrap；实例／类型／全局三级吊销；主人句柄与接入方句柄；审计事件 |
| `studiolo-agent`（接入方） | `serve --policy <目录>`：每次调用复核吊销与能力集；供给视图 ＝ `--allow` ∩ 能力集白名单；路牌第 3／5／6 项改自 effective；tools `policy_read`、`propose_desired`；`GET /read/policy`；admin `policy-init` / `policy-bootstrap` / `policy-status` / `instances` |
| `studiolo-local-server`（人侧） | `--policy-data <目录>`（会话模式）：`/api/v1/policy`（状态）、`/policy/desired`（GET 列、POST 提交）、`/policy/validate`、`/policy/withdraw`、`/policy/activate`（**L3**：发布／回滚／解除全局吊销）、`/policy/versions[/{n}]`、`/api/v1/agents/revoke`（L2 三级吊销） |
| `web/` | 「策略平面」视图：effective 与现行各项、提交 desired 并看报告（扩大项标红）、发布（就地弹 One-time Code 窗口，认证通过后自动继续这一次发布，不必去账户页再回来重来）、待处理 desired（含 AI 提案）、版本列表与回滚、三级吊销 |
| `auth` | `FactorRuntime::require_l3` 门面透出（T-026 复审 #19 的 M4 前置） |

## 本机跑一遍（全部虚构材料）

```bash
# 1. 接入方私有目录与策略库
mkdir -m 700 -p /private/tmp/sa/data/audit /private/tmp/sa/policy
cargo run --locked -p studiolo-agent-server -- admin init --data /private/tmp/sa/data
cargo run --locked -p studiolo-agent-server -- admin policy-init --policy /private/tmp/sa/policy
# 2. 在交互终端签发类型 token 并 bootstrap 首版能力集（正文文件见 contracts/policy-plane.md §2 的形状）
cargo run --locked -p studiolo-agent-server -- admin type-issue --data /private/tmp/sa/data --type claude
cargo run --locked -p studiolo-agent-server -- admin policy-bootstrap --policy /private/tmp/sa/policy \
  --data /private/tmp/sa/data --kind capability_set --subject claude --file /private/tmp/sa/claude.json
cargo run --locked -p studiolo-agent-server -- admin policy-status --policy /private/tmp/sa/policy
# 3. 接入方服务：能力集之外的目录从一开始不进入计算（本机 --vault 示例；生产白名单见 scripts/replica_common.py 的 ALLOW）
cargo run --locked -p studiolo-agent-server -- serve --vault /path/to/readonly-copy \
  --allow 项目/<示例项目> --allow _规则 --data /private/tmp/sa/data --policy /private/tmp/sa/policy --port 4400
```

人侧发布链路需要会话模式与因子模式（`--session-config` ＋ `--factor-socket` ＋ `--policy-data`），
虚构夹具下的完整链路见 `crates/local-server/tests/policy_http_contract.rs`：登录 → 提交 desired → 看 diff 与扩大项 →
`step-up` → `activate`。没有二次认证的 `activate` 一律 403 并进审计。

## 验收（二元）

| 用例 | 证据 |
|---|---|
| PRD-v2-09 用例 36：AI 提交 desired 永不自动生效；发布须经 L3 并展示 diff 与扩大项；直接改文件不产生授权 | `policy_contract.rs::policy_read_is_audited_and_proposals_never_change_effective`（接入方半）＋ `policy_http_contract.rs::desired_is_validated_diffed_and_activation_is_gated_behind_l3`（人侧半）；effective 只由 `PolicyStore::activate` 改变，vault 文件不在任何写路径上 |
| 用例 37：实例吊销即刻；类型吊销使其全部实例即刻失效 | `policy_contract.rs::three_level_revocation_is_immediate_for_running_sessions` |
| RG-CONSUMER 1：未登记类型零权限 | `policy_contract.rs::a_type_without_a_capability_set_has_zero_rights_until_one_is_published` |
| RG-CONSUMER 5／6：能力集与 skills 走 L3；实例不能自我提权、三级吊销即刻 | 同上两条 ＋ `policy_store_contract.rs` |
| PRD-v2-01 2.4 第 7 条：effective 哈希与各 AI 拿到的能力集哈希不一致次数恒为 0 | `handshake.served.effective_hash` 与策略库当前值逐条相等（`policy_contract.rs` 断言） |

## 接入方线上验收（2026-09-20，T-086 / T-087）

正式 4400 已给 `serve --policy`，`claude` 首版能力集生效；公网 MCP/HTTP、策略版本一致、白名单拒绝、
匿名/坏凭据 401 与副本 ready 通过。实例/类型/全局吊销在 4401 隔离副本中逐项验证，未吊销生产身份。
启用前与含策略的两个一致恢复包均已保存，分别在独立系统服务中恢复并通过供给验收；恢复包保留在生产服务器私有目录。
详见 [操作包与恢复边界](policy-operations.md)。程序仍为 `m4-t084-20260919-audit03`，本次没有升级二进制。

## 明确未做（不得据本目录宣称）

- **人侧发布链路**：因子模式未部署，因此真实 One-time Code 发布未在生产验证。默认发布模板仍是兼容模式；
  未来升级必须保留正式单元现有 `--policy` 和对应可写目录，不能直接覆盖。
- 推模式（T-038 客户端投影）缺省仍导出 vault 规则文件；2026-09-22 T-111 起配置 `rules_source: "policy"`
  可改从策略平面 effective 导出（见 `m3/client-projection.md`），真实客户端尚未切换。
- 人侧界面没有实例列表（吊销按 id）；`admin instances` 提供列表。
- 策略库已在契约与类型层纳入一致 checkpoint 清单（2026-09-22 T-110：`CheckpointOwner::Policy`，
  恢复含策略时须先核对吊销）；但策略库的真实 checkpoint 参与者与常态／异机备份仍未实施，
  T-086 的同机恢复点不替代异机灾备。
- M5 的规则版本钉定已由 T-089 在黑板工作项中实现（`task-engine/src/board_types.rs` 按 effective 版本钉定规则引用）。

## 上线验收探针（T-073，2026-09-18）

M4 之后**任何**从 main 打出的运行包，`tools/list` 都是六个工具——即使部署时不启用策略平面。`deploy/agent/upgrade-read-supply.sh.in` 在 4401 阶段实例上跑的
`scripts/check-agent.py`／`check-agent-replica.py` 原先写死 M3 的四个只读工具，会把新二进制判为「有额外工具」而中止升级（失败关闭，不会带病上线，但也上不了线）。
现在探针期望恰为六个，并按路牌做一次**只读**的策略平面探测：路牌写「策略平面：未接入」→ `policy_read` 必须以固定码与文案拒绝；否则必须读得到带「# 策略版本」头的 skills 索引。
探针永不调用 `propose_desired`。握手被策略拒绝（类型被吊销或没有能力集）时单独报错，不再误报成审计降级。

根因是这组探针此前只在单测里面对假服务器，从未在 CI 里遇到真实二进制。`scripts/check-replica-pipeline.py` 现在对它自己拉起的真实 `studiolo-agent` 加跑同一组探针
（check-agent 三种凭据、副本回执、HTTP 外壳），CI 与运行包自检（`run-agent-tests.sh`）都会经过这一步。

## 启用前置（部署动作，登记在 vault TASKS ③）

1. 服务器 `mkdir -m 700 …/policy`，`admin policy-init`，交互终端 `admin policy-bootstrap` 写各类型首版能力集（至少 `claude`）。
2. `deploy/agent/` 两个单元加 `--policy <目录>`；重启后核对启动日志「策略平面：已接入」与 `handshake.served.policy_plane=present`。
3. 人侧发布链路等 M2 因子模式部署后再启用 `--policy-data`。
