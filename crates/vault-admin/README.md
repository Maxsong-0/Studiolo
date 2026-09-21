# canonical 首建管理命令

`studiolo-vault-admin` 仅从**管理员已经冻结的私有快照**，把明确允许范围内的 Markdown 通过 VaultBroker 导入两个既有空目录。普通服务继续使用 `FsVaultBroker::open_existing`。本命令不更改 Auth 或 Policy，不签发人类身份或 L3 权限，不监听 HTTP，不提供持续 Obsidian 同步。

```sh
studiolo-vault-admin plan \
  --config /srv/studiolo/private/owner.json \
  --vault /srv/studiolo/canonical \
  --writer-data /srv/studiolo/writer-data \
  --assets /srv/studiolo/release/web \
  --source-snapshot /srv/studiolo/frozen-source \
  --allow 项目/Studiolo --allow _规则

studiolo-vault-admin initialize \
  --config /srv/studiolo/private/owner.json \
  --vault /srv/studiolo/canonical \
  --writer-data /srv/studiolo/writer-data \
  --assets /srv/studiolo/release/web \
  --source-snapshot /srv/studiolo/frozen-source \
  --allow 项目/Studiolo --allow _规则 \
  --expected-plan '<刚审核的 plan_digest：64 位十六进制>'
```

示例目录只是参数说明，不代表已批准或已经部署的真实目录。所有绝对路径必须事先由操作者选定；CLI 不创建根、不调整权限。`--allow` 可以是目录前缀或精确 Markdown 路径，必须存在，不接受重复或相互包含的范围。一次最多 64 个范围。

准备要求：

- 主人配置沿用 host-config v1，0600；父目录、Auth、Audit、canonical、writer-data 和 source 都为当前用户所有的 0700 真实目录。配置和全部批准目录不可为符号链接；各服务根、来源与配置文件必须满足分离约束。不得把只读滚动副本原地改成 canonical。
- Audit 必须已经存在且格式可用。`plan` 不打开审计写连接；`initialize` 只用 `SqliteAuditLedger::open_existing`，没有审计库则拒绝，不自建或迁移。
- canonical 与 writer-data 必须完全为空；已有 `.workbench`、Broker 绑定、隐藏文件或任意其他条目都不能重跑。一次导入部分失败后保留现场，不自动删除、覆盖、修复或继续导入。
- 来源全树按 `ReadOnlyVault::open_private` 扫描：目录 0700、文件 0600；拒绝链接、隐藏项、诊断、不完整扫描和所选范围内附件。仅允许有效 UTF-8 Markdown。范围外附件不导入；本工具不是附件迁移器。
- 扫描硬上限为单文件 4 MiB、全树 Markdown 64 MiB、全树条目 20,000、深度 32。任何超限拒绝，不产生截断“成功”。

`plan` 连续两次有界读取，输出 `version / plan_digest / files / bytes / allow`，不输出正文、逐文件列表或宿主路径。摘要绑定源及目标、Auth/Audit/assets 根的路径与设备/inode、配置文件身份及内容修订、排序范围、排序文件路径/字节数/内容 hash。`initialize` 重新执行同一检查，并用常量时间比较 32 字节摘要；不匹配时先于审计写入和 Broker 初始化失败。

两次扫描一致**不能证明跨文件原子快照**。操作者必须在整个审核与初始化窗口冻结来源，并保持可信父目录稳定。每次导入前后复验原批准根、持有句柄身份与配置；这不是对同权限管理员或 root 的隔离，也不消除任意恶意路径替换的全部竞态。

正式 CLI 在解析参数/配置前固定进程 `umask=0077`，避免调用 shell 的默认 0022 使新目录变为 0755；随后调用 `studiolo_process_safety::harden`，向 stderr 写固定逐项分类。Linux 运行账号必须有足够 `RLIMIT_MEMLOCK`；不满足则拒绝，不降级运行。规划读取可能更新 OS atime，除此之外不创建或改写持久文件。

初始化先记录 `vault.bootstrap` intent（actor_kind `local_os_bootstrap`、本地 UID、`explicit_offline_plan` authority），再在 Broker 单写者租约内首建。每份文档经 `VaultWriter::mutate(Mutation::create(...))`，稳定幂等键为 `bootstrap-{plan_digest}-{排序序号}`。成功或失败写 outcome；失败审计写明已经导入的计数，失败现场原样保留。成功 stdout 仅含 `vault_uuid / files / bytes / plan_digest`，任何输出失败都必须先核对实际状态，不能自动重试。

这只建立快照当时的 canonical 正文。后续原 Obsidian 文件变化不会自动进入该库；持续同步必须另行配置和验收。黑板的消费者写权限仍由真实 effective policy、身份和 L3 发布裁决，本 CLI 不批准这些权限。

定向验收：

```sh
CARGO_TARGET_DIR=/private/tmp/studiolo-vault-admin-target cargo test --locked --offline -p studiolo-vault-admin
cargo fmt -p studiolo-vault-admin -- --check
```

真实 CLI 权限测试只在子进程设置继承 umask=0022，并逐项核对导入后 canonical/writer-data 的所有新目录 0700、新文件 0600。macOS 和 Linux 都默认执行，没有 ignored 分支。Linux 须沿用标准测试机/CI 的 unlimited MEMLOCK；加固失败使测试失败，不降级跳过：

```sh
cargo test --locked --offline -p studiolo-vault-admin cli_overrides_inherited_umask
```
