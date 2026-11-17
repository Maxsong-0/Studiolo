# 固定 restic 命令计划

Rust 命令计划固定 restic **0.19.1**，与既有部署包文档一致。旧 0.18.2 常量没有对应官方标签，旧 `check --snapshot` 参数也不存在；不能用仅检查字符串的测试宣称真实 CLI 已接好。

- 全部计划带 `--json --no-cache`，不依赖 App 的 HOME、全局缓存或继承的 restic 配置。
- `CheckSubset { bucket }` 使用 `check --no-lock --read-data-subset <n>/30`（n 只能是 1～30），每次读仓库里的一组数据包，由 worker 每日轮流抽查（[仓库抽查契约](backup-repository-check.md)）；它没有“仅校验某快照”的含义，也不替代原作业/封存/最终清单配对证明。带 `--no-lock` 是因为 restic 拿锁时不跳过陈旧锁，被强制结束的检查留下的独占锁会挡住之后所有备份；维护前先停 worker。原来没有调用方的整库 `CheckRepository`（`check --read-data`）2026-11-17 删掉。
- 作业 tag 包含单调序号与 checkpoint ID 的 128 位 SHA-256 前缀；恢复后重用序号不能把新 checkpoint 指向旧作业。同一键稳定产生同一 tag。
- RepositoryRef、JobTag、SnapshotRef 的持久化/IPC 读取也必须经 parse；反序列化不能绕过封闭参数校验。
- 允许注入的秘密名称为 RESTIC_PASSWORD_FILE、RESTIC_REST_USERNAME、RESTIC_REST_PASSWORD。仓库位置由固定 `--repo` 提供，不再同时传互相冲突的 RESTIC_REPOSITORY_FILE。这里只登记名字，不读取值；App 不因此获得凭据。

真实 CLI 契约：`crates/backup/tests/restic_cli.rs`，默认明确 ignored，测试机必须提供 `STUDIOLO_RESTIC_TEST_BINARY` 并显式 `--ignored`。它用一次性虚构本地仓库执行 init、生成的 backup/check/restore 参数、完整快照 ID/原 tag 查询、恢复字节核对；30 组抽查全部干净，破坏本测试产生的最大数据包后只有它所在那一组查出问题、`broken_packs` 为 1，别的组照样干净。它不证明 NAS、追加后端权限或生产可恢复性。

官方 Linux amd64 bzip2 发行摘要：`f415415624dcc452f2a02b8c33641791a8c6d6d3b65bbb3543fcf9a25151585c`。108 验证先下载发行包并核对该摘要，再解压执行；不能只凭 `version` 的自报字符串信任任意二进制。正式 worker 仍需固定已核验的执行文件身份、清空环境、施加超时/输出预算和原封存绑定。

一手依据：[官方 0.19.1 发行](https://github.com/restic/restic/releases/tag/v0.19.1)、[固定版本 check 参数](https://github.com/restic/restic/blob/v0.19.1/cmd/restic/cmd_check.go)、[固定版本恢复语义](https://restic.readthedocs.io/en/v0.19.1/050_restore.html)。本子单元不等于独立生产 worker 或完整 Rust 恢复链已完成。
