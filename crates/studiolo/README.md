# studiolo

Studiolo 的 M0 组合根，装配文件系统 VaultBroker、SQLite TaskEngine 和隔离 Stub Runner。

`M0Workbench::open` 打开显式指定的 Vault 与数据目录；调用方通过 TaskEngine 执行命令、投影和重启对账。它不启动网页、常驻服务或真实外部工具，审计使用内存 Adapter。

本 crate 承载最小垂直链、重启对账与写入口架构测试。crates.io 的已发布版本仍是占位包，本地 M0 实现尚未发布。工作区结构与验证命令见仓库根 README。
