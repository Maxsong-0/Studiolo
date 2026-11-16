# 出站事件门铃（Webhook）

决定见 ADR-033。本目录是候选发布源，**尚未安装到生产**。真实收件方的试响、密钥注入和安装分别留证，不能用测试结果代签。

## 它做什么

- `studiolo-doorbell` 是一次性程序：定时器每分钟拉起一次，从上次的位置往后读人侧审计账本，只把下表里的动作变成事件发出去，其余一律不出门。

  | 事件类型 | 什么时候 |
  |---|---|
  | `proposal.submitted` | 有人（主人或接入方）提交了一份待审的策略提案 |
  | `policy.activated` | 一个策略版本生效了 |
  | `access.revoked`／`access.released` | 全局吊销／解除 |
  | `task.created` | 黑板上新建了一个任务 |
  | `handoff.created`／`handoff.returned` | 新建了交接包／交回了回执 |
  | `task.abandoned` | 一次认领被放弃，任务等人重新认领 |

- 事件是瘦的：正文只有 `type`、`timestamp`、`data`（最多一个 `link`）。不带正文、路径、任务号、身份。对方要内容，带着自己的身份经 MCP 或网页来取。
- 按 Standard Webhooks 1.0.0 签名：请求头 `webhook-id`、`webhook-timestamp`、`webhook-signature`（`v1`＝HMAC-SHA256）。`webhook-id` 重发时不变，**收件方必须按它去重**，并核对签名和时间戳（建议只收 5 分钟内的）。
- 送达是「至少一次、24 小时内」：按账本顺序发，前一条没送到后面的不发；失败后按 0、5 分钟、30 分钟、2 小时、5 小时、10 小时退避；超过 24 小时没送出去的跳过并记审计。回执（2xx）只表示对方的服务收下了。
- 对方回 410 就停用，单元报失败，直到换了收件地址。3xx 不跟，算拒收。
- 每条出门的事件在审计账本里有一条意图、一条结果（动作 `observability.doorbell.dispatch`）；启用时记一条 `observability.doorbell.enabled`，**启用之前的历史不补发**。审计写不进去就不发。

## 不做什么

没有入站：这个程序不监听任何端口，也不接受别人往 Studiolo 里推东西。只有一个收件地址，要分给多家由收件方自己转。

## 候选包与安装前检查

在 108 用仓库标准回路构建 `cargo --locked build --release -p studiolo-local-server --bin studiolo-doorbell`。候选包根保留：

```text
bin/studiolo-doorbell
deploy/doorbell/systemd/*
```

代码装到 root 拥有、不可组写的 `/opt/studiolo-doorbell/releases/<提交号>/`，`current` 指向它。回滚只切回前一包，不删状态和审计。

由用户在人侧那台机器上建两个 maxsong 拥有的私有目录（0700），不要改已有目录的权限：

- `~/.config/studiolo-doorbell/`：放 `doorbell.env`（0600）。
- `~/.local/share/studiolo-doorbell/`：程序自己写 `doorbell-state.json`（读到账本第几条、退避到什么时候）。

`doorbell.env` 由用户经安全输入写入，不从聊天或日志里复制：

| 名称 | 必填 | 写什么 |
|---|---|---|
| `STUDIOLO_DOORBELL_URL` | 是 | 收件方的 HTTPS 地址；不带用户名口令和 `#` 片段 |
| `STUDIOLO_DOORBELL_SECRET` | 是 | `whsec_` 开头的签名密钥（24～64 字节随机数的 base64），收件方用同一把验签 |
| `STUDIOLO_DOORBELL_LINK` | 否 | 放进事件的链接：要登录才能打开的 HTTPS 地址，不带查询串 |

地址和密钥只给一个算配置错误，程序直接报错，不猜另一半。

## 验收顺序

1. 不带 `--deliver` 跑一次，只应输出 `dry_run_no_delivery`。
2. `systemd-analyze verify` 两个单元；核对两个私有目录的属主和权限。
3. 带 `--deliver` 手动跑第一次，应输出 `enabled`（记下启用位置，什么都不发）。
4. 在网页上做一件表里的事（例如建一个任务），再跑一次，应输出 `caught_up delivered=1 expired=0`，收件方收到一条能验签的请求。
5. 把收件方停掉再做一件事，应输出 `deferred unavailable …`；恢复后下一次补上，`webhook-id` 不变。
6. 以上都对，才启用定时器。

输出只有固定的词和计数，不含地址、密钥和事件内容。定时器和手动运行撞上时，后到的那个输出 `busy` 后直接退出（同一个状态目录同一时间只有一个门铃进程在跑），不算失败。

状态文件只是「读到第几条」和退避的缓存，「送到没有」以审计账本为准：状态文件丢了，下一次运行回到最初启用的位置重扫一遍，送到过和过期过的不会再发、再记，只补上真正没送出去的。状态文件**损坏**则失败关闭，要人看过再处理。

## 收件方怎么接

- Claude Code routine 的 API 触发、Cursor Automations 的 Webhook 触发都能收这种 POST，但它们各有自己的鉴权方式（各自的令牌放在各自那边），通常要在中间放一个很小的转发器：验签、去重，再带上那一家的令牌去触发。转发器和那些令牌都不在本仓库。
- 被叫醒的一方拿到的只有事件类型。它应当把这当成「去看一眼」的信号，带着自己的身份经 MCP 读黑板或策略，不要把事件正文当指令。
