# canonical新鲜度的后续设计输入

2026-09-05，T-028之后另行认领；只读核查，尚未实现。复用现有完整扫描、
WorkbenchReadModel与FileTree构建，不新造索引系统。持久搜索与增量更新不并入首片。

## 当前缺口

- readonly/scan.rs明确只保证局部安全稳定读取，不保证整库事务快照。
- local-server现有HTTP刷新future持Mutex，取消等待会释放它，而spawn_blocking仍在运行。
  该锁不能直接成为canonical单任务刷新保证。
- generation是进程内视图代次，真实revision绑定UUID/路径/正文；两者与持久提交水位
  indexed_through/committed_through必须分开表达。HTTP的u64水位以字符串表达。

## 最小完整捕获与发布

1. watcher注册完成并记录失效代次；失败或溢出保持stale。
2. 在唯一Broker owner进程内互斥范围内复核根/marker/data与未决状态，捕获完整批次
   和提交水位；根FD租约负责跨进程，同一owner锁负责所有应用写入与捕获不交错。
3. 释放owner锁，复用原解析/索引/文件树构建不可变候选。
4. 发布前重新取得owner锁，复核绑定、水位、未决及失效代次；变化即丢弃候选。
5. 在一个短状态锁内原子替换完整Generation及indexed_through，不能把正文、导航、
   文件树和新鲜度分开发布。
6. 实际后台任务持刷新许可到结束；取消HTTP等待不放槽。重复触发只合并一个待重扫标记。

新提交/写入开始、watcher故障、扫描失败或超预算、候选复核失败、未决、根或绑定变化、
水位倒退都保持stale。失败保留旧完整只读视图，不冒称新视图完整；重启强制完整扫描。
扫描安全diagnostics意味着不能宣称完整；读取侧已允许的frontmatter诊断保持其原语义。

所有canonical安装必须进入同一水位或由owner同步失效，包括artifact及manifest、
后续bridge和显式恢复。不能只比较两个水位相等就开放编辑。

## 仍须核实的证明边界

owner锁能串行化可信Broker写者，不能冻结管理员带外改文件。watcher事件也可能延迟，
注册完成不等于已经观察到所有并发变化。后续正式计划须明确唯一写者运行约束、捕获时
文件/目录稳定复核与watcher代次的证据边界；不得把前后水位相等宣称任意外部写入下的
整库原子快照。实时目标CAS仍须在保存时独立执行。

## 编辑与bridge的门禁

编辑在owner锁内最终重验freshness、水位、绑定、未决与目标CAS。bridge摄入走另一
受信内部能力，仍验身份/白名单/审计/未决/删除熔断与base revision，但不要求索引先追平；
否则stale会阻断追赶，永远不能恢复。请求里不能接受ignore_stale之类开关。

验收覆盖捕获与写入互斥、构建后新提交、发布/失效竞争、取消仍占槽、失败保留旧视图、
溢出恢复、根/marker替换、未决/重启/水位倒退，以及stale挡编辑但合法bridge仍可追赶。

## T-028冻结后核对的最小接口候选

local-server的私有CanonicalCoordinator拥有唯一FsVaultBroker及owner锁，workbench仍
纯构建。Broker可新增capture(limits)和revalidate_capture(stamp)：捕获复用现扫描，
输出ReadEntry、真实CanonicalRead及不可构造/反序列化的CaptureStamp；stamp私有绑定
实例、永久配对和提交水位，不是授权票据，不返回FD/锁guard/通用回调。

固定owner→view锁顺序：owner内捕获，owner外构建，重新owner内核stamp/水位/未决，
再在短view锁内核watcher代次并替换完整视图。以后保存也在相同owner锁内最终重验，
同步标stale再mutate；artifact和bridge必须走同一owner的类型化入口。
刷新任务持有强Arc与实际执行期permit，请求取消不能提前释放租约或并发额度。

现--canonical只装CanonicalReadVault，host-config v1也没有Broker data字段。
必须显式新增写者装配及data与assets/config/auth/audit的隔离检查，使用open_existing；
不能自动升级旧只读开关、猜用Auth/Audit目录或允许另一组件再open一个Broker。
