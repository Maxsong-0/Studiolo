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

本接口与UI仍在整体验收中；同步、统一健康告警、独立备份与真实身份/部署尚不能据此计为完成。
