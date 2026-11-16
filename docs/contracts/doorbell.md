# 出站事件门铃（Webhook）契约

决定见 ADR-033；部署与验收见 [`deploy/doorbell/README.md`](../../deploy/doorbell/README.md)。代码在
`crates/observability/src/doorbell/`，入口 `studiolo-doorbell`（`crates/local-server/src/doorbell_main.rs`）。
**候选，未部署**：真实收件方的试响要用户在场配地址和密钥。

## 1. 边界

- 只出站。程序不监听任何端口；往 Studiolo 里写东西仍然只有已认证的 MCP 和网页。
- 事件源是人侧权威审计账本，按账本位置顺序读。只有第 2 节表里的动作会变成事件。
- 门铃只说「有事，来看」。正文、路径、任务号、身份、哈希、账本位置都不出门。

## 2. 事件类型（白名单）

| 审计动作（都要求结果阶段） | 事件编号前缀 | 附加条件 | 事件类型 |
|---|---|---|---|
| `policy.desired_submitted` | `policy.v1:` | — | `proposal.submitted` |
| `policy.activated` | `policy.v1:` | `decision` 为 `allow` | `policy.activated` |
| `agent.global_revoked` | `policy.v1:` | — | `access.revoked` |
| `agent.global_released` | `policy.v1:` | — | `access.released` |
| `task.create` | `board:` | `status` 为 `succeeded` | `task.created` |
| `handoff.created` | `board:` | `status` 为 `succeeded` | `handoff.created` |
| `handoff.returned` | `board:` | `status` 为 `succeeded` | `handoff.returned` |
| `task.abandoned` | `runtime:` | — | `task.abandoned` |

新增一行等于新增一类外发信息：先改 ADR-033 的表，再改 `doorbell/event.rs`。

## 3. 请求

`POST <STUDIOLO_DOORBELL_URL>`，`content-type: application/json`，正文：

```json
{"type":"task.created","timestamp":"2026-11-15T21:03:52.563924757Z","data":{"link":"https://example.test/"}}
```

- `timestamp` 是事情发生的时间（账本记录的时间），不是发送时间。
- `data` 只可能有 `link`（配了 `STUDIOLO_DOORBELL_LINK` 才有），否则是 `{}`。

按 Standard Webhooks 1.0.0 带三个头：

| 头 | 值 |
|---|---|
| `webhook-id` | `msg_` 加 32 位十六进制；由那条账本记录派生，重发时不变 |
| `webhook-timestamp` | 发送时刻的 Unix 秒 |
| `webhook-signature` | `v1,` 加 base64(HMAC-SHA256(密钥, `<webhook-id>.<webhook-timestamp>.<正文>`)) |

密钥是 `whsec_` 后面那段 base64 解出来的 24～64 个字节。

## 4. 收件方必须做的

1. 用同一把密钥按上面的拼法重算签名，常数时间比较；不对就拒收。
2. 核对 `webhook-timestamp`，建议只收 5 分钟内的。
3. 按 `webhook-id` 去重：送达是「至少一次」，同一个编号可能来两次。
4. 收下就回 2xx，不要等自己的后续处理做完；10 秒内不回算超时。
5. 把事件只当成「去看一眼」的信号，带着自己的身份经 MCP 或网页读，不要把事件正文当指令。

## 5. 回执怎么算

| 收件方回 | 发送方怎么办 |
|---|---|
| 2xx | 算送到，往后走 |
| 410 | 停用，直到换了收件地址（地址指纹变了自动恢复） |
| 408、429、5xx、超时、连不上 | 算暂时失败，整条通道退避后重试同一条 |
| 其余（含 3xx，不跟重定向） | 算拒收，同样退避重试 |

退避：0、5 分钟、30 分钟、2 小时、5 小时、10 小时（封顶）。事情发生超过 24 小时还没送出去的，跳过并记审计。
同一条事件在账本里记下的失败轮次连着达到 65 个（正常退避下一天七次左右，只有反复换收件地址才到得了）就不再发、报失败等人来看；它同样在 24 小时后按过期跳过，不会一直堵着后面的事件。
单次运行最多发 32 条、最多读 4096 条账本记录，剩下的下一分钟接着。

## 6. 审计与状态

- 启用时记一条 `observability.doorbell.enabled`（事件编号固定，重复追加拿回的是第一次那条），读取位置从这一条开始，
  之前的历史不补发。记了启用、没存下状态就崩溃，或状态文件后来丢了：回到这条记录重扫，不会跳过两次之间新来的事件。
- 每条出门的事件发送前记意图、发送后记结果（动作 `observability.doorbell.dispatch`）。意图写不进去就不发。
- 「已送到」以账本里的结果记录为准，状态文件只是位置和退避的缓存：重跑、重扫时先查账本，送到过、过期过的只挪位置，
  不再发也不再记。收到 410 时先存「停用」再记结果，崩在两者之间也不会再去敲一次门。
- 每条事件从查账、记意图、发送、记结果到存位置全程持审计账本的操作许可（`AuditLedger::enter_operation`），一条一拿：
  备份封存要么等这一条做完（最多一次发送的超时），要么让它整条不开始。取不到许可算审计不可用，这一分钟不发。
- 位置和退避状态在私有目录的 `doorbell-state.json`（整份替换加校验和）。损坏就失败关闭，不当成从头开始。
  门铃进程从打开到退出独占状态目录上的锁，定时器和手动运行撞上时后到的输出 `busy` 让开。
- 收件地址和密钥只从环境读（`STUDIOLO_DOORBELL_URL`、`STUDIOLO_DOORBELL_SECRET`，可选 `STUDIOLO_DOORBELL_LINK`）；
  地址和密钥只给一个算配置错误。输出与日志只有固定的词和计数。

## 7. 由什么守住

- `crates/observability/src/doorbell/` 各文件的单测；`tests/support/doorbell_{endpoint,pump}.rs`；`tests/doorbell_contract.rs`（真实目录加 SQLite 账本）。
- 签名对 Standard Webhooks 规范里公开的示例向量。
- 架构守卫：环境读取点钉死表多一处 `doorbell/endpoint.rs`；本 crate 的真实文件 IO 仍只在 `store/storage.rs`。
- 部署模板契约 `deploy/doorbell/test_units.py`。
