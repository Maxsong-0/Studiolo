# M6 每次 Gateway 调用的 Broker 封存

`GatewayPublisher::seal_gateway_call(&mut self, GatewayCallSeal)` 为每次上游调用发布独立的
私有元数据 manifest。Fs 和 Memory 都实现该能力；没有增加原 `VaultBroker` supertrait，
也没有改变最终 `ArtifactCommit`/attempt manifest 的语义。

固定 commit point：

```text
.workbench/gateway/<task-id>/<attempt-id>/<sha256(call_id)>/manifest.json
```

调用者不能提供目标路径。`call_id` 即使包含 `../`、斜线或 `$` 也只参与 SHA-256，不能越界。
目录、目标和逐级父链始终从已经固定并持有独占租约的 canonical root capability 打开，
拒绝 symlink、hardlink、大小写碰撞、危险节点和观察到的父链身份替换。

## 输入与回执

`GatewayCallSeal` 精确携带：

- `task_id`、`attempt_id`、`fence`、`call_id`、`audit_intent`；
- `raw_sha256`、`raw_bytes`、`source_manifest_sha256`；
- `model_provider`、`model`、`retain_until_unix_seconds`；
- `snapshot_id`、`effective_policy_hash`。

DTO 没有 raw 字节、prompt、模型回复正文、文件描述符或环境路径。三个摘要及 snapshot ID
为 64 位小写十六进制；字符串有固定长度上限且拒绝控制字符；保留期限必须为正绝对 Unix 秒。
这些是 Broker 结构校验，不能替代可信 Gateway 对 grant、源保留期限、fence、模型和实际 raw
摘要的核验。Broker 不凭元数据宣称私有 raw 已保存或将被按时删除。

`GatewayCallReceipt { manifest_path, sha256 }` 返回逻辑私有路径和完整 manifest 的摘要。
manifest 内容包含格式版本、Vault UUID 和完整 `GatewayCallSeal`，因此同调用的全部字段
都参与幂等证明。同 task/attempt/call 且完全相同的元数据返回同一回执；任一元数据变化返回
`IdempotencyConflict`，永不替换已有 manifest。

## 落盘与隔离

磁盘 adapter 在开始和完成前后重验既有 writer 身份与未决门禁。批量写入在途或其他 writer
操作未决时，Gateway 封存也拒绝，不创建新目录。

先创建并 fsync 私有父目录，再写完整元数据临时文件并 fsync；通过同目录 create-only 原子
安装发布 manifest，最后 fsync 目录并重新读取核对完整字节。manifest 是该调用唯一可见
commit point，所有临时文件都不构成成功。安装或 fsync 中断后，以相同请求重放检查原字节并
再次 fsync，可恢复原回执；不同内容不能借重试覆盖。公开 API 不提供修改或删除该 manifest。

这里不复制 raw，不向 L0 history 写 raw 或元数据，也不占用
`sources/attempts/<task>/<attempt>/manifest.json`。同 attempt 的多次 Gateway 调用和最终产物
提交可以各自拥有独立 commit point。私有元数据写入不会冒充正文提交或推进正文提交水位。

## 验收

已写测试覆盖 Fs/Memory 两次调用加最终 artifact 提交互不冲突、同 call 不同 digest 拒绝、
重开后幂等、恶意 call ID 路径隔离、pending batch 门禁、非法元数据拒绝和私有目录 symlink。
没有真实上游调用，没有实际 raw 正文搬运测试；raw 保存和保留策略由 Gateway 自己验收。

根整合后统一运行 `cargo test -p studiolo-vault-broker` 与单包 Clippy；Linux 路径语义必须走
实际 githubci 原生回路。编写测试本身不等于已通过运行。
