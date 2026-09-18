# 人侧 canonical 单页读取（T-022）

## Broker 能力

`CanonicalReadVault::open_existing` 只打开可信装配明确给定的绝对根。
要求已有合法 format-v1 `.workbench/vault.json`，身份文件最多 4096 字节。
不创建或修复 marker、data、历史和索引；不提供写方法。

每次读取前后复核根目录身份与 marker UUID，拒绝观察到的根或身份替换。
内容安全复用既有只读内核：相对 VaultPath、隐藏路径保护、不跟随链接、
拒绝硬链接和特殊节点、有界读取、读取前后稳定性检查。
调用方必须传字节上限；增长超过上限拒绝，最多保留上限加一字节。
成功返回 `vault_uuid / path / content / revision`，revision 复用既有 Broker 算法，
绑定持久 UUID、路径与实际字节，可与同根 FsVaultBroker 的 revision 对照。

M1 的只读 namespace revision 保持原语义。共享读取内核将超限错误明确为
`ReadLimitExceeded`，不再与一般不安全节点混同。此能力只支持已验证的 Unix 平台。

## HTTP 与启动

新 `router_with_canonical_sessions` 同时接收同根只读视图和 canonical 句柄，
目录身份不相等就拒绝装配。旧三个组合入口均不注册新路由。
（**2026-09-17 T-056 核实**：`crates/local-server/src/routing.rs` 现有五个公开组合入口
——`router:20`、`router_with_access:40`、`router_with_sessions:64`、
`router_with_canonical_sessions:87`、`router_with_factors:114`。本段说的「旧三个」仍然成立：
前三个都传 `canonical = None`，不挂 `/api/v1/file`。但 T-024 后加的 `router_with_factors`
同样可以接收一个 `Option<Arc<CanonicalReadVault>>` 并在其为 `Some` 时挂上本路由，
装配前做同一条根身份相等检查（`routing.rs:114-133`、`:236-238`），不是「只有
`router_with_canonical_sessions` 能挂」。）
生产 CLI 仅显式 `--canonical` 且已有 `--session-config` 才启用；
缺失/损坏身份在外部取钥前失败，不初始化真实根。

`GET /api/v1/file?path=...` 仅接受唯一 path 字段，无未知字段或重复参数，
路径最多 4096 字节，限 Markdown；每请求真实 Access、L2 Cookie 配对、严格同源。
不接受 HEAD 或任何写方法。复用全局两个内容 blocking 槽，取消不会提早释放槽。
成功只返回四个字符串字段，revision 为十六进制；无 M1 generation。

| 情形 | 状态 |
|---|---|
| 无效/缺失/重复/未知查询 | 400 |
| 路径保护、链接或不安全节点 | 403 |
| 不存在 | 404 |
| 读取期间不稳定 | 409 |
| 内容超过 1 MiB | 413 |
| 内容非 UTF-8 | 422 |
| 根/身份/底层存储不可用 | 503 |

错误采用固定文案，不回显绝对路径或底层错误。内容读取失败不回退旧快照。

## 限制

单页有界稳定读取不代表整个索引新鲜，也不锁住带外写者。
`/api/document` 继续按 M1 generation 读取原快照；不能把它的代次当作编辑 revision。
本片没有 HTTP CAS、持久索引、watcher、bridge 或源码编辑；后续写入口必须单独登记
授权矩阵、唯一写者和新鲜度契约。测试结果见 M2 检查点，真实 Vault 未接入。
