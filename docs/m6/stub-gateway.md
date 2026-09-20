# M6 Demo Stub 基线

`GatewayStubRunner` 在组合 crate 内实现 `RunnerAdapter`，保留原 M0 `StubRunner` 的兼容合同。
可信装配传入 `Arc<dyn Gateway>` 与已验证的 `JobCapability`；适配器不自行发 capability，
启动时严格核对 task/attempt/fence，重放不得改变 RunSpec。

每次运行固定调用 `web.fetch` 与 `db.query`。call ID 包含 task/attempt/fence/工具，重放复用
相同 ID。Web 请求固定为 `target_id=demo-web`、`path=/demo.json`、空 query；DB 请求固定为
`source_id=demo-db`、`view=demo_items`、limit=32。真实返回体作为 RunInput 进入原 Stub 的
`result.md`，同时原样写入 `gateway-demo.json` 候选。未生成固定推断或验收成功结论。

`MemoryGateway` 可替换响应的测试只证明 Gateway seam；`DemoGateway` 的契约进一步使用
真实回环 HTTP 服务和临时 SQLite 文件，验证两种源的数据变化会进入新 attempt 的候选。

`DemoGateway::open` 必须显式 `mode: demo`。由可信配置固定数字回环地址与非零端口；只
执行 `GET /demo.json`，不接 Runner URL/header，不解析 DNS，不跟重定向，不接受压缩或
分块响应。连接/写超时各限 2 秒，整个连接与响应读取另有 2 秒截止期限，header ≤8 KiB，JSON body ≤64 KiB。它不开放生产 Web 源。

数据库必须是可信配置中的绝对路径、文件名 `demo.sqlite3`，打开方式是 SQLite READ_ONLY，
并设置 query_only、trusted_schema=OFF。只查询普通表 demo_items 的 id/label/value 三列，
固定 id 顺序，上限 32 行、label 1024 字节；多余行、类型不合、任意 SQL/视图/源 ID 都拒绝。
测试初始化 schema 为：

```sql
CREATE TABLE demo_items(id INTEGER PRIMARY KEY,label TEXT NOT NULL,value INTEGER NOT NULL);
```

Demo 使用原 MemoryGateway 复核四层权限、实时 attempt/fence、取消、到期、准入审计和
call ID 冲突。准入结果显式标识 demo_admission，只表示准入。真实 I/O 另外由
`gateway.demo.effect` Intent/Outcome 包围，带 task/attempt/tool，审计仅含 mode/decision，
不含 raw 返回值。任意审计失败不释放候选；I/O 后再次复核权限，撤销不能释放刚获得的数据。

实际结果的幂等槽在进程内最多 128 个；同 call ID 精确重放返回原结果，不重新 HTTP/查询。
真实 I/O 或 Outcome 故障也占用结果槽，失败不自动重试。进程重启不承诺 Demo 调用持久幂等；
该合成基线不能冒充真实 Runner 的耐久 Gateway、隔离验收或生产外发准入。

Runner 的 start 和 observe 都重新通过 Gateway 检查已完成调用；取消/撤销/到期/权限拒绝
返回失败或 Cancelled，不产生新的成功候选。候选本身不等于任务成功，仍需宿主引擎验收。
`probe=Ready` 仅表示内置合成适配器可用，不宣称 Claude/Codex 或外部源已就绪。

## Demo 初始化向导（CLI）

```sh
cargo run --locked -p studiolo --example demo -- /absolute/new-demo-directory --keep-web
```

目录必须为全新路径；任何既有目录（含空目录）均拒绝，不采用、覆盖或清空已有 Vault。
向导创建私有 Demo Vault、SQLite Demo DB、真实回环 Demo Web、独立 Control DB 与 SQLite
权威审计，再通过真实 Gateway → Stub → TaskEngine → Broker 保存一次候选与 manifest，
任务停在 NeedsReview。没有自动人工验收，也不读取模型凭据。主程序创建的内容全部经 Broker。

`--keep-web` 保持 Demo Web 至按 Enter；省略时完成自检后关闭 HTTP，数据与候选保留。
输出包含实际 task/manifest/审计数量及人侧只读示例命令，标签固定为「当前为演示源」。
界面中的执行与回放入口保持可见；真实 Runner 未配置时显示缺少前置条件并保持冻结。
这个命令不注册系统服务，不更改真实备份计划或生产配置。生产安装入口的 GUI 仍不在此命令内。
