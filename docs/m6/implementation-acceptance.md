# M5–M6 初期实现与本地验收记录

> 本页是首次开发检查点，以下 API Gateway 选择及未提交状态已被后续决定覆盖。当前采用本机订阅客户端；真实模型闭环见 [本机派发验收](local-client-dispatch.md)，发布边界见 [部署装配](deployment.md)。禁止依据历史段落启用 API key-holder。

日期：2026-09-20 UTC（本地 2026-09-19）。任务：T-088～T-091。

本轮完成黑板、批量写回、两个原生 CLI Adapter、Gateway、监督界面及 Demo 的代码与本地契约验收。
用户明确选择「尚未准备，先完成代码和本地验收」。真实部署、两家模型调用和 RG-RUNNER 尚未验收。
**不能据此记录宣布 M5/M6 已上线或通过完整生产验收。**

工作树：`.claude/worktrees/codex-m5-m6-blackboard-runner`；分支：`codex/m5-m6-blackboard-runner`；
起点：`b42893fc43666b359a6dd551afefcac7f84dc852`。本轮源码仍为工作树改动，未提交、推送、合并或部署。
主检出及 T-086/T-087 的部署工作域保留。

## 用户决定与实际权限

- Claude、Codex 可写 `项目/Studiolo` 黑板，其他已登记 AI 只读，未登记类型仍无权限。
- 专用 API 凭据由独立 Gateway 身份持有；Runner 只拿短期 attempt/fence capability，不复用个人 CLI 登录目录。
- 首批能力集已写为[待发布示例](../m5/integration.md)，尚未激活生产写权限。
- 专用 Linux 执行环境、服务账号、CLI/rootfs 和密钥文件尚未准备；真实执行体保持冻结。

## 实现与验收对应

下表是 PRD-v2-09 的本地覆盖入口；最后一列不代表真实 Sync 或生产账号验收。

| PRD 用例 / 范围 | 实现 | 本地证据 |
|---|---|---|
| 38–41：认领、收尾、仲裁、规则版本 | 沿用 Control DB 事务与 fence；验收/交接/回执结构、租约与路径独占、终态仲裁、规则 hash 钉定 | `control-db/tests/blackboard_transactions.rs`、`task-engine/tests/blackboard_contract.rs` |
| 42：max+1 发号 | 真实文件名最大值结合持久占用，禁止填历史空号 | Broker numbering 与黑板事务测试 |
| 43：控制库无正文 | Broker 私有 staging；Control DB schema v3 仅保存可验证指针；旧 reservation 显式迁移 | `control-db/tests/schema_open_contract.rs`、TaskEngine 候选/恢复测试 |
| 44：文件投影、人写保留 | 批次投影、CAS、生成区带外改动迁移与审计 | `task-engine/tests/projection_idempotency.rs`、黑板投影测试；真实 Sync 未验 |
| 45–46：纯文本、实时服务不可用即停止 | MCP 认证写入口；HTTP fallback 仍只读；副本不能代替实时黑板 | `agent-server/tests/policy_contract.rs`、`local-server/tests/board_http_contract.rs` |
| 47：结构化工作项/回执 | 字段必填、逐项覆盖验收标准、冲突/未落地产物/closeout、旧 fence 拒绝 | 黑板、runner candidates、stopped candidates 契约 |
| 48–49：子项与子实例 | 深度/数量上限、父子范围、兄弟不相交、同类型父链、祖先吊销 | `supply/tests/child_instance_contract.rs`、MCP policy 与黑板事务测试 |
| 50：记忆写前查重 | fact key、类型/摘要/理由/适用方式、日期、行数上限、更新已有条目 | Broker memory_write 与 board HTTP 契约；不声称自动语义判断事实相同 |
| M5 前置：批量写回 | 预览、整批 CAS、耐久 journal、恢复屏障、manifest 最后写、整批回滚 | Broker batch 契约与 Linux 路径安全测试 |
| 51：真实 Runner 的代码契约 | Claude/Codex Adapter、固定模型与上游、持久预算、取消/停滞/重启、cgroup+bwrap 探针 | Runner/Gateway/launcher 契约通过；两家真实 API 和 RG-RUNNER 未验 |
| 图 1 / 图 3 | 子实例树、路径占用、待审批/旧规则、项目时间线与任务回放、候选查看/落地/验收 | Web 模型和交互测试、HTTP 契约、浏览器合成页面检查 |
| Demo | 创建全新 Demo Vault、真实回环 Web、只读 SQLite、Gateway → Stub → Broker 候选 | 4 项组合契约和 3 项实际 CLI 进程测试 |

父 claim 与子 claim 沿用保守独占规则：父仍持有相交路径时子项不得同时 claim，必须先释放/交接。
规则钉定仅保留工作依据；每次调用仍检查当前授权，不阻止立即吊销。

Runner 的 CLI 成功仅产生 `NeedsReview`。代码落地要求当前 L3、HEAD/tree 与 patch 摘要匹配，
使用仓库锁和私有 index；不会自动 commit/push。取消先撤权，再给 stopped 回执保留窗口；
没有有效回执不能伪造成功取消，候选/patch 保留后按契约进入 abandoned。

## 验证结果与准确边界

Rust 的 Linux 语义在仓库授权的专用 CI 主机 `githubci` 原生验证，使用本工作树独立同步树和 target，
未使用生产凭据、生产 Vault 或个人 CLI 配置。以下是全库回归加修复目标复验的组合证据，**不是一次全绿的全库命令**。

| 检查 | 结果 |
|---|---|
| `cargo --locked fmt --all -- --check` | 通过 |
| `cargo --locked clippy --workspace --all-targets -- -D warnings` | Linux 通过；最终新增 Demo example 另行严格 Clippy 通过 |
| `cargo --locked test --workspace --exclude studiolo-vault-broker --no-fail-fast` | 176 个目标，1974 通过、3 失败；两个失败目标修复后全部复验通过，见下文 |
| 首轮受影响 9 包回归 | 96 个目标，1197 通过、13 失败、4 ignored；后续全库/定向复验覆盖全部失败目标 |
| 最终 Linux 收尾回路 | 9 个测试目标，100 通过、0 失败；包括 policy 6、Runner 26、架构 40、batch 20、Linux 路径安全 8 |
| local-server / auth-admin 生产 feature 构建 | 均通过；真实 auth-admin CLI 进程契约通过 |
| Web | 101 文件、819 测试通过；build / lint / format 全通过 |
| Python launcher | 9 项通过；覆盖记录截断、EOF、畸形输出、stopped 回执等 |
| check-agent | 12 项通过 |
| Demo example | 严格 Clippy、构建通过；3 项实际进程测试通过 |

首轮 Broker 全库执行过（库测试 315 通过、1 失败、4 ignored）；失败的精确 patch 回滚测试修复后，
批次相关 20 项与 Linux 路径安全 8 项全部通过。4 个既有 ignored 用例未声称通过。
后续完整回归为节省重复的大文件压力测试成本排除 Broker，最后复验其改动相关测试。

最终全库回归的三个失败分别为：MCP initialize 阶段的实时黑板路牌误用已完成握手检查，
以及两个 snapshot 并发测试暴露的文件锁生命周期问题。前者改为检查凭据/当前绑定及 stale 状态；
后者显式解锁，避免 Linux fork 继承同一 open-file-description 后锁被子进程继续持有。
增加持有重复 FD 的回归测试，修复后 policy、整个 Runner 包和架构守卫均通过。
未放宽 busy-lock、授权、路径或隔离检查，也未把失败改成跳过。

本机曾在 Linux 暂时不可达时做 macOS 补充回归；其夹具/平台路径问题已修复并由后续 Linux 回归覆盖。
这里不宣称最终 macOS 全库通过，也不把本机较新 Clippy 的既有警告当作正式 Linux 门槛结果。
Web 构建仍提示部分 chunk 超过 500 KiB；构建成功，本轮未处理独立的打包体积优化。

实际浏览器检查：合成数据页面 `http://127.0.0.1:5179/.m5-final-preview.html`，视口 1265×712，
检查子实例树、过期租约计数、时间窗口/滑轨/日志、落地确认与 HEAD 改动后确认失效。
该证据只证明 UI 交互；没有发起真实执行或生产落地。检查后预览文件、进程及预览标签已清理。

## 可复现入口

标准完整回路仍为仓库 `scripts/test-linux.sh`。本轮最后定向回路执行：

```sh
cargo --locked fmt --all -- --check
cargo --locked clippy --workspace --all-targets -- -D warnings
cargo --locked test -p studiolo-agent-server --test policy_contract
cargo --locked test -p studiolo-runner-adapter
cargo --locked test -p studiolo --test architecture
cargo --locked test -p studiolo-vault-broker --lib batch
cargo --locked test -p studiolo-vault-broker --test linux_path_security
cargo --locked build -p studiolo-local-server --features studiolo-auth/production
cargo --locked build -p studiolo-auth-admin --features production
python3 crates/auth-admin/tests/support/cli_process.py "$CARGO_TARGET_DIR/debug/studiolo-auth-admin"
```

Demo 最后单独执行：

```sh
cargo --locked clippy -p studiolo --example demo -- -D warnings
cargo --locked build -p studiolo --example demo
python3 scripts/test-demo-init.py "$CARGO_TARGET_DIR/debug/examples/demo"
```

Demo 检查实际 manifest SHA-256、两种源的候选内容、0700 根目录、含空格和引号的参数保真、
已有目录拒绝且原文件哈希不变、HTTP 200/404，以及按 Enter 关闭端口但保留候选。
使用方式见 [Demo 初始化](stub-gateway.md)。CLI 初始化入口已交付；生产安装入口的 GUI 未在本轮验收。

本轮原始日志位于执行机的 `/private/tmp/`，不作为永久发布凭据：
`studiolo-m5-all-tests.log`、`studiolo-m5-linux-final.log`、`studiolo-m5-linux-closeout.log`、
`studiolo-m6-demo-final.log`、`studiolo-m5-web-{tests,build,lint,format}-2.log`、
`studiolo-m6-launch-tests.log`、`studiolo-m5-script-tests.log`。

## 后续真实环境验收

按用户决定，本轮到代码与本地验收止。环境准备后用[真实验收记录模板](release-acceptance.md)逐项记录：

1. 人侧因子模式与 L3、身份库显式迁移、现行能力集发布；只给 Claude/Codex 的 Studiolo 项目写权限。
2. 两家专用 CLI、固定版本/rootfs、独立服务 UID、Gateway 密钥文件及费用/数据外发范围。
3. 真实调用、预算/吊销/冻结、取消后的完整 cgroup 清空、重启与孤儿清理、候选落地及人工逐项验收。
4. TASKS 和批准产物的真实 Sync 往返、备份范围；raw 响应和长期凭据不进入同步镜像。

以上尚未执行。只读 CI 合成验收、结构化 probe 输出或 Demo 均不能替代 RG-RUNNER。

后续同日用户要求当前 Codex 实际使用：已完成独立临时 MCP/Canonical 实例的 19 个真实动作，
含创建、认领、写入、读回、完整回执与 5 项预期拒绝。详见 [当前 Codex 使用记录](../m5/consumer-smoke.md)。
这补充了真实外部 Consumer 证据，不改变上述 M6 与生产边界。
