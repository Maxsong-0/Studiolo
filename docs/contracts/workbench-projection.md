# Workbench 刷新闸门与计数投影契约

> Module：`studiolo-workbench`。本文件只描述两件事：并发刷新请求如何被**有界合流**，
> 以及首屏如何只取**计数**而不把整份文档清单读进内存。派生规则本身见
> [vault-index.md](vault-index.md) 与 `docs/m1/`。两者都是纯值语义：不创建线程、
> 不加锁、不等待、不读时钟、不做 I/O。

## 1. 刷新闸门 `RefreshGate`

### 1.1 类型化 Interface

```rust
impl RefreshGate {
    pub fn new() -> Self;
    pub fn resuming_from(ticket: u64) -> Self;
    pub fn request(&mut self, reason: &'static str) -> Result<RefreshAdmission, RefreshClosed>;
    pub fn claim(&mut self) -> Option<RefreshRun>;
    pub fn settle(&mut self, run: RefreshRun, outcome: Result<(), &'static str>);
    pub fn fail_abandoned_run(&mut self, reason: &'static str) -> bool;
    pub fn observe(&self, ticket: RefreshTicket) -> RefreshWait;
    pub fn close(&mut self);
    pub fn is_closed(&self) -> bool;
    pub fn is_busy(&self) -> bool;
    pub fn stats(&self) -> RefreshGateStats;
}
```

闸门不持有互斥量、条件变量或运行时。调用方把它放进自己的 `Mutex`（或异步等价物），
并自备唤醒手段；闸门只回答「谁去跑、跑到哪张票、结果如何」。

### 1.2 不变量

- **至多一次在途、至多一次排队**：任意数量的并发请求最多换来两次派生。
- **排他性由 `claim` 单点强制**，不由「谁拿到 `Start`」强制。`claim` 是写入在途位的
  唯一入口，并发调用中只有一个返回 `Some`。
- **票号单调递增、不回绕、不复用**。触顶时闸门主动 `close`，绝不发重号。
- **合流不交付更旧的结果**：排队位在请求到达时才占用，所以排队的那次派生一定开始于
  这些请求之后。
- **完成水位只前进不后退**。

### 1.3 正在刷新时的新请求

| 到达时的闸门状态 | 受理结果 | 调用方应做的事 |
|---|---|---|
| 空闲 | `Start(新票)` | 循环 `claim` / `settle`，直到 `claim` 返回 `None` |
| 有派生在途 | `Join(排队票)` | 等待自己的票；被唤醒后先 `observe`，仍是 `Running` 就再试 `claim` |
| 已关闭或票号触顶 | `Err(RefreshClosed)` | 直接答复不可用，不重试 |

`Start` 是「去认领」的**义务**，不是「我已经是运行者」的断言：并发请求可能同时拿到
`Start`，但只有一个能认领到派生，其余立刻读到 `None` 并转为等待。等待者被唤醒后再试
`claim` 是恢复路径的关键一环（见 1.4）。

### 1.4 失败与恢复语义

- **派生失败**：`settle(run, Err(reason))` 记录理由，被这次派生覆盖的票读到
  `Failed(reason)`。闸门**不**被锁死，也没有需要人工清理的中间态；恢复方式就是下一次
  `request` 重新跑一遍完整派生，成功后失败状态被清除。
- **运行者忘记继续认领**：只推迟排队中的那次派生，不会永久卡死。在途位在 `settle`
  时立即释放，下一个请求会被要求接手同一张排队票。
- **运行者死亡**（线程 panic、任务被取消）：观测到这一事实的调用方调用
  `fail_abandoned_run(reason)`。在途位立即释放，被覆盖的票读到 `Failed(reason)`，
  排队位保留；等待者按 1.3 的循环再试 `claim` 即可接手，不必等新请求到来。
  闸门自己不读时钟，分不清「跑得久」与「已经死了」，**因此这个入口绝不能当超时用**。
- **死而复生的迟到结算**：被判定为遗弃之后才到达的 `settle`，其票号不再等于在途票号，
  整体忽略——既不前移完成水位，也不覆盖等待者已经读到的失败，只计入 `settles_ignored`。
- **关闭**：`close` 丢弃排队位，之后的请求一律被拒，未完成的票读到 `Closed`。已认领的
  在途派生仍须 `settle` 或判定为遗弃，两者都不会让关闭后的票重新变成完成。
- **`RefreshWait` 是状态读数，不是历史判决**：一张被失败派生覆盖的票，在更晚的成功派生
  清除失败后会读到 `Completed`。调用方应在被唤醒时读取一次并据此答复。

### 1.5 可核对的证据

`RefreshGateStats` 全部是单调计数：`admitted`、`runs_started`、`runs_failed`、
`runs_abandoned`、`settles_ignored`，`coalesced() = admitted - runs_started`。
契约测试用真实线程和真实派生调用计数核对，不用计时：
`refresh_gate_contract.rs`（33 次并发请求只换来 2 次派生）、
`refresh_failure_contract.rs`（失败、遗弃、迟到结算、关闭、票号触顶）、
`refresh_recovery_contract.rs`（8 个等待者在运行者死亡后接手，排队派生只跑一遍）。

## 2. 计数投影

### 2.1 为什么要有

`WorkbenchSnapshot` 带一份逐篇 `documents` 清单，其规模随本代文档数线性增长；首屏其实
只需要「有多少篇」。`WorkbenchReadModel::overview()` 返回 `WorkbenchOverview`：项目、
收件箱、诊断加 `WorkbenchCounts`，**不含逐篇清单**。只要数量时还可以用
`counts()` 或 `document_count()`。

### 2.2 计数不可得是一等结果

`CountRollup { value: Option<u64>, unavailable: Vec<String> }`。`value == None` 表示计数
本身不可得，**绝不退化成 0**；此时 `unavailable` 必定非空，按来源标识的码位序排序去重，
逐条说明是哪个项目使计数不可得，界面可直接展示而不必编造数字。

- `active_tasks` / `open_bugs` 是逐项目汇总：任一项目的 `Sourced` 值不可得，整个汇总
  不可得，**不做部分求和**。
- 有证据的零仍然是零：全部 BUG 都能分类且都已关闭时是 `Some(0)`，不是不可得。
- 累加溢出 64 位同样转为不可得，并给出 `累加结果超出 64 位计数范围`。

### 2.3 有界性证据

- `overview()` 构造过程不遍历文档表、不读取任何文档正文。
- `projection_bounds_contract.rs` 逐字段走遍返回值累加**实际持有的字符串字节数**：
  库从 103 篇涨到 2003 篇，首屏投影字节数**完全相等**；同一代次的逐篇清单随文档数线性
  增长，并比首屏投影大出一个数量级以上。
- 单项目派生按逻辑路径前缀取有序区间（`scan::under`），访问次数等于命中条目数加一次
  越界判定，与库内其余文档数无关；`scan` 的单元测试用真实访问计数核对。
- **未度量项**：工作区禁止 `unsafe`，本 crate 也禁止进程与文件系统访问，因此进程级
  峰值 RSS 没有在测试里度量，本文件不对它作任何声称。

### 2.4 仍然与库规模同阶的地方（已评估，非缺陷）

| 位置 | 规模 | 结论 |
|---|---|---|
| `WorkbenchReadModel::build` | 每代一次 O(N) | 建代次本身必须读完整批，不是每请求一次 |
| `snapshot()` | 每次调用 O(N) | 保留给确实要逐篇清单的调用方；首屏改用 `overview()` |
| `search()` | 每次查询 O(N) 扫描 | 字面搜索没有倒排索引时必须全扫；输出有 100 条上限且提前中断，内存有界 |
| `FileTree::build` | 每代一次 O(N) | `page()` 每次至多 200 条，已有界 |
