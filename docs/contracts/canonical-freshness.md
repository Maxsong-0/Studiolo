# canonical 捕获与刷新新鲜度契约

T-030，2026-09-05。实施中；Broker 和配置定向测试已过，服务装配及双平台整合尚待验证。
完整 M2 仍包含持久索引、保存、同步、编辑、备份恢复及真实运行门槛。

## Broker 的完整捕获

`FsVaultBroker::capture(&mut self, ScanLimits)` 返回一次扫描的 `ReadEntry`、
真实 `CanonicalRead` 和 `CaptureStamp`。附件只含元信息，Markdown 的 revision
直接在扫描中绑定实际 Vault UUID、逻辑路径和原字节；不为每个文件重新打开读取。
原 `ReadOnlyVault::scan` 继续使用原只读命名空间，不能作为编辑 revision。

捕获前后均核实永久 root/data 配对、固定对象、未决状态和提交水位。任何安全诊断
或总预算失败都不会签发部分捕获；原解析层 frontmatter 警告不变成文件扫描遗漏。
`ScanLimits` 的单页、累计正文、条目、深度和路径预算维持原含义。

Stamp 不可公开构造或反序列化，只公开提交水位读取。它绑定内存实例、永久配对、
Vault UUID 和 `committed_through`，不携带 FD、不延长根租约，也不是授权票据。
`revalidate_capture` 必须交回原 Broker；不同实例或水位变化返回 `CaptureChanged`，
扫描不完整返回 `IncompleteCapture`，未决保持 `MutationUnresolved`。

私有 Writer 在完整验证的日志读取和成功持久提交后记录本实例最高已见提交水位。
低于该值后稳定拒绝 `CorruptWriterState`；恢复较高文件不能解除本实例的拒绝。
不把 Prepared/Installing/Completed 的操作序号当作已提交水位，不改变磁盘 schema，
不声明可以检测未观察到的回滚、跨重启回滚或管理员一致克隆。

新 `FsVaultBroker::read_file(path, max_bytes)` 从读取开始即执行实际字节预算，
在同一 owner 中提供真实单页读取；未决或绑定/已见水位倒退拒绝，不返回旧缓存。
`validate_root_identities(canonical, data)` 只用于可信装配核对批准目录与所持 FD。
它不输出路径/FD，不能将调用者提供的身份数字变成权限。

## 唯一 owner 与整体发布

`CanonicalWorkbench::prepare` 消费唯一 Broker 和封闭的 `CanonicalServingRoots`。
HTTP 只收到工作台控制句柄，不能初始化 Broker、取得裸写者或提供任意写回调。
当前 M0 与新服务是互斥装配：另一 `M0Workbench::open` 必须遇到 `WriterBusy`。
未来同时装配 TaskEngine 时必须移交并复用同一 owner，不能另开实例。

每轮实际后台作业依次完成：owner 内捕获、owner 外构建、owner 内重验、短状态锁内
整体发布。固定 owner → 调度状态 → 视图的锁序；正文、导航、树与 indexed 水位
作为一个 Generation 发布。失效代次与健康检查必须同发布锁定，不能先检查后再
无条件替换视图。失败保留上一个完整只读视图，并标明 stale。

后台作业持强引用直到真实执行结束。HTTP 取消仅停止等待；最多一个实际作业和一个
待重扫位，触发不创建无限后台任务或逐请求队列。停止时关闭新触发、唤醒等待者，
在后台关闭 native watcher 并等待执行中的捕获/构建结束，再释放 Broker 租约。
旧只读刷新同样把许可及发布放入实际 blocking 闭包。

## 监视与证明范围

精确使用 `notify 8.2.0`，监视注册后才做初次完整捕获；启动失败不提供可监听路由。
回调只作失效通知，使用弱引用，不能扫描或在自身线程关闭 watcher。
打开、明确读取及访问时间事件不算内容改变，防止扫描触发自身；实际内容修改、
创建、删除、写关闭及 Rescan 仍失效。Linux 的打开事件是 `Open(Any)`，同样须忽略。

Rescan 代表事件不足以重建当前状态，必须完整重扫；监视故障还必须重新注册成功。
周期检查、失败与事件触发均经过同一有界调度器。该约束依据
[notify 文档](https://docs.rs/notify/8.2.0/notify/)及
[Rescan 语义](https://docs.rs/notify/8.2.0/notify/event/enum.Flag.html)。

监视事件可延迟或遗漏，owner 锁也不能冻结管理员带外改写。这里的新鲜度表示已追平
本 owner 的已知提交、监视健康且发布时无已观察失效；不证明任意外部并发下的整库
原子快照。后续保存仍须在 owner 内最终重验授权、范围、新鲜度和目标实时 CAS。

新鲜度观测与 Generation 在同一短锁读取。进程 generation 与持久提交水位不同；
`indexed_through` / `committed_through` 用十进制字符串，无法确认当前值时为 null。
stale、未决、监视健康及固定原因不授予编辑权限，也不输出主机路径或配置内容。

## 显式装配

`--canonical-writer-data` 要求绝对、既有 data、明确 session 配置和公共 Origin；
与旧 `--canonical` 互斥。旧 flag 保持窄只读能力；新模式只用 `open_existing`，
不初始化缺失目录或借用 Auth/Audit 存储。重开可能依既有写者协议补确定终态，
因此新进程不承诺零写。

封闭根快照检查 canonical、assets、auth、audit、writer-data 五个目录两两分离，
data 为既有当前 UID 的 0700 普通目录，配置文件位于 data 外。共同配置父目录
维持旧语义；路径别名按真实祖先身份检查。writer-data 也进入 Unix socket 隔离。
TCP 和 Unix 的新模式均在退出时停止调度并等待实际作业完成。

本片不开放 HTTP 保存或 AI 整库供给；HTTP 会话与因子来源门禁继续独立生效。
