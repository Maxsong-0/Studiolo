# 人侧正文编辑契约

T-046，2026-09-12；当前在开发验收。总体进度见[M2/M3台账](m2-m3-closeout.md)。

## 可信范围

私有0600主人配置可显式增加 `human_content` 对象；缺省保持只读，不由HTTP或正文授予权限。
该配置沿用原配置的文件身份、字节与根复核；运行中替换配置会拒绝后续操作，需要受信重新装配。

```json
{
  "version": 1,
  "vault_uuid": "00000000-0000-4000-8000-000000000001",
  "folders": ["notes"],
  "protected": ["notes/private"],
  "documents": {
    "project/INDEX.md": [["<!-- generated:start -->", "<!-- generated:end -->"]]
  }
}
```

此示例只说明对象形状，UUID须来自已批准canonical；不能直接用于真实库授权。
目录按路径组件匹配，固定拒绝隐藏路径、规则/skills/policies/capabilities、ops、sources、
私有运行根，以及CLAUDE/AGENTS/SKILL文件。额外策略路径必须由可信 `protected` 登记。
INDEX/TASKS/dashboard须由 `documents` 显式分类；空哨兵列表表示可信层确认全篇为人写正文。
哨兵必须整行唯一、不交叠、首尾完备；比较包括哨兵和区内换行的完整字节。

## HTTP与保存

GET `/api/v1/file?path=...` 保留 `vault_uuid/path/content/revision`，增加：

- `editing:{allowed,reason,max_bytes:262144}`，仅为提示，不是缓存授权。
- `freshness`：现有完整新鲜度对象或null。

网页编辑上限256KiB，单页阅读仍为1MiB；大附件须走后续流式通道，不能改称不支持同步。
POST `/api/v1/mutations` 只接受JSON对象：

```json
{
  "kind": "replace",
  "vault_uuid": "00000000-0000-4000-8000-000000000001",
  "path": "notes/example.md",
  "expected_revision": "64位小写十六进制版本",
  "content": "完整UTF-8 Markdown",
  "idempotency_key": "客户端生成的16到128位字母数字或连字符标识"
}
```

客户端不能传主体、任意precondition、force或忽略stale。正文读取有字节、帧数、时间和并发预算。
每次请求验证Access、应用会话、同源与CSRF；真正阻塞作业持并发许可直到结束。
唯一owner中复核会话、可信配置/范围、VaultUUID、完整新鲜度、水位、Markdown及生成区，
然后由Broker实际CAS、保存历史和原子安装。普通L2保存不要求每次重新输入TOTP。

成功200返回 `status:committed` 和 `receipt:{path,previous_revision,new_revision}`。
这是该幂等请求的持久历史回执，不证明当前磁盘仍是该版本；随后从canonical重新观察。
409 `revision_conflict` 只返回预期/实际revision和路径，不复制正文；前端保留基线/本地稿并独立重读磁盘。
`outcome_unknown` 保留原key与稿件，不用新key自动重试，也不按当前哈希猜原请求成功。

## 草稿与审计

`owner_scope` 是真实已验证issuer/audience/subject/email加域分隔的SHA-256，
不含令牌，不授予权限；它仅把同一浏览器Boundary的内存草稿绑定到主人身份。
锁定或401隐藏内容；重新登录后只有相同主人、同源、VaultUUID和路径可人工恢复。
后台更换主人不能显示前一主人的草稿；没有scope的旧服务器保持只读兼容。

普通人写入按矩阵允许审计故障降级；审计只含主体、路径、版本和内容摘要，不含正文。
Broker先固定事件ID与主体/操作描述符，outbox保留到权威事件回执逐项核对后才确认。
普通保存可以先落盘再补记；未知HTTP结果不生成永久失败终态。

## 页历史与恢复

GET `/api/v1/history?vault_uuid=...&path=...&limit=20&cursor=...` 返回有界历史页，
包含不透明条目ID、revision、字节数，以及分开的原作者/创作时间与归档主体/归档时间。
初始或旧格式未记载的作者与时间保留null，不能拿mtime或归档主体代填。
游标绑定路径和当前历史代次；列表变化返回409 `history_changed`，客户端从第一页重读。
GET `/api/v1/history/file?vault_uuid=...&path=...&revision=...` 返回精确历史UTF-8正文，
网页预览仍限制256KiB；大版本可列出，但网页不加载或恢复其正文。

### 逐行归属（T-210，2026-11-15）

GET `/api/v1/history/blame?path=...`（可带 `vault_uuid` 核对当前绑定库）返回当前正文每一行最后是哪一版写的。
只读、L2，和历史页同一套范围检查；只算 Markdown，保护路径不提供；结果不缓存，每次请求先后重验会话与根绑定。

算法：从当前正文沿版本链（和历史页同一份、最新优先，最多回看 50 版）往回走，相邻两版按行比对，
一行在哪一版第一次出现就归那一版。**不猜**：

- `origin: "version"`：带 `history_id`、`revision`、`author`、`authored_at_ms`；那一版作者未知时 `author` 是 null，
  不拿归档主体或磁盘时间顶上。
- `origin: "untracked"`：当前正文不在历史里（库外直接改的文件），这些行是那次改动带来的。
- `origin: "beyond_history"`：一直存在到可查历史的最早一版，而那一版不是创建——更早的版本没有记录或已按保留策略清理。
  只有 Broker 的原请求或原批次终态证明某一版是装到当时不存在的路径上（`FsVaultBroker::history_lineage` 的 `created`），
  留到那一版的行才归它的主体；删除、改名、从墓碑恢复、旧格式和「之前的版本」都不算创建。

响应封闭字段：`vault_uuid, path, revision, content_sha256, lines, versions, more_history, runs`；`runs` 从第 1 行起首尾相接、
盖满全部行，每段 `from_line, to_line, origin, history_id, revision, author, authored_at_ms`。行按 `\n` 切、行尾的 `\r` 不算内容。
`content_sha256` 是当前正文的普通 SHA-256：网页的阅读器看的是工作台快照，两边摘要相同才画，不同就说明「不是同一版」。

失败关闭，不给半截结果：当前正文或任一历史版本超过 256 KiB、超过 2 万行、比对量超出上限时回 413 `too_large`；
不是有效 UTF-8 回 422；算的过程中正文被换掉回 409 `history_changed`；写入在途时和历史页一样不可读。
计算放在专用派生线程上（`deriver`，BUG-034：几十版正文的临时分配不上阻塞线程池），一次只留两版正文在内存里。

网页：阅读器工具条上的「显示每行是谁改的」，默认关，打开才请求一次；行号左边一条窄带，每段第一行写缩写
（人、AI、Ob、?、外、早）并可聚焦，整句在提示里，来源靠线型加缩写区分、不靠颜色。

没做的：从窄带点进那一版的历史；附件的逐行归属；开发假数据接口没有这个端点（预览里显示「不提供」）。

恢复使用同一POST路径的封闭JSON：

```json
{
  "kind": "restore",
  "vault_uuid": "00000000-0000-4000-8000-000000000001",
  "path": "notes/example.md",
  "expected_revision": "当前版本的64位小写十六进制值",
  "source_revision": "待恢复历史版本的64位小写十六进制值",
  "idempotency_key": "客户端生成的16到128位字母数字或连字符标识"
}
```

服务端只从同路径历史取正文，再验当前CAS、配置、会话和生成区。客户端不能提交替代正文。
恢复先持久固定Intent、真实追加并核验Audit回执，最终复核准入后才CAS安装。
初次503 `audit_unavailable` 仅用于已证明没有安装且取消已持久化的情况。
可能已经安装或Outcome尚未确认时返回 `outcome_unknown`，保留原key并冻结新写入，
直至相同请求获权威确认；幂等绑定包括主体与操作种类，不能改成replace领取恢复回执。
已完成请求可在源历史字节清理后领取原回执，不读当前版本猜测成功，也不重新安装。

默认保留策略采取保守解释：仅当某条历史既不在最新50版、又早于30天时才可清理；
未知时间不按年龄清理。该选择避免原“50版或30天”表述的歧义导致过早删除，仍需真实恢复验证。
当前恢复仅覆盖已有页面；删除后的恢复随墓碑/创建单元接入。L0不等于独立灾备。

## 编辑器与浏览器

Markdown正文使用CodeMirror，预览按Markdown AST渲染，HTML正文不直接执行。
编辑器和预览按需加载。生产HTML模板每次响应获得随机样式nonce，只有该nonce可授权
CodeMirror动态样式；脚本策略仍为 `script-src 'self'`，无 `unsafe-inline`。
nonce不写回磁盘、不随API或静态资产返回。除 `/assets/` 下带 Vite 内容哈希、成功送出的
JS/CSS/SVG/woff2 为长缓存 immutable（T-114、T-165）外，所有响应仍为no-store；HTML 模板始终 no-store。
「带哈希」只认文件名为 `非空名字-` 加恰好 8 位 `[A-Za-z0-9_-]`（T-169），多段连字符的普通名
（如 `archivo-latin-800-normal.woff2`）与非 8 位哈希一律 no-store；口径以 `auth-http.md` 为准。

本接口与UI仍在整体验收中；同步、统一健康告警、独立备份与真实身份/部署尚不能据此计为完成。
