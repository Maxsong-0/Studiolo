# T-174 算法与设计审查及修复（2026-09-29）

基线：origin/main `c7c2fc34a06f6da36a44cfd7c98d343c91bc218a`。
用户要求整项目深度审查并当晚落实。先读取 Vault INDEX、TASKS、当日日志、仓库约定，
独立工作树实施；研究阶段 1 个代理，实施阶段索引与 Gateway 两域并行，根负责策略/搜索与最终验收。

## 已确认并实施的问题

| 问题 | 根因与修复 | 验收入口 |
|---|---|---|
| 密集链接长行放大内存和CPU | 每条链接复制整行、从头计列；改成320字符上下文窗口及单调Unicode列计数 | vault-index bounded_markdown_contract |
| 未闭合反引号重复扫描后缀 | 每个开串从头找同长闭串；改成单次分词、反向等长索引、正向跳过已配对区间 | vault-index markdown_scan_tests |
| 标题锚点碰撞 | A/A/A-1会重复a-1，纯标点锚点为空；全局已用集合与单调后缀，空锚点用section | vault-index bounded_markdown_contract |
| 规则删约束漏报 | 权限扩大提示只遍历前2000条展示diff；完整删除事件流独立检查，扩大项过量明确汇总 | policy rules_diff_contract，含真实activate |
| 大规则的小改被整替换 | LCS先按全文件乘积熔断，未变3000行也全删全加；裁共同首尾、连续表与完整边界预算 | policy rules_diff_contract |
| Gateway请求正文未释放及历史扫描 | Evicted仍保存原arguments/prompt，完成时全表计数/找最老；固定指纹与有界完成队列 | Gateway新增及既有幂等契约 |
| Gateway异常遗留占位 | 占位后授权回调panic发生于RAII建立前；占位即持有守卫并贯穿交付 | Gateway授权异常回归 |
| 指定项目搜索遍历全库 | 有序映射仍逐条过滤；用Borrow<str>做前缀range并在目录边界停止 | workbench search_scope_tests及既有search契约 |

解析规则变化使 PARSE_CACHE_VERSION 1→2；旧缓存失效并从当前Markdown重新派生。
没有改持久权威库schema、租约/fence、L3发布门禁或生产配置。

## 核验过但本轮未发现确定缺陷的路径

- Control DB / TaskEngine：claim、租约、fence、终态保留和回执幂等；Immediate事务中复核。
- Policy：发布CAS、授权读事务与撤销；此次扩大项漏报属于审阅提醒，不是L3绕过。
- Sync Bridge：完整观察、三方基线、删除熔断、静止门禁、未知结果不重放、冲突保全与恢复停点。
- Broker / Audit：单写者绑定、水位回退、未决阻读，以及分段审计的现有设计边界。
- 前端：检索取消/代次、规则动作生命周期、时间线归并与服务端排序；本轮未改前端。

这是一轮覆盖关键路径的证据审查，不声称逐行穷尽全仓或证明不存在其它缺陷。
MemoryGateway修复不等同于DurableRelay生产外发路径改写；compact墓碑仍随唯一调用ID增长，
用于保证已逐出的调用不被当作新调用再执行。正文和可重放结果的留存有明确上限。

## 算法收益与代价

- 长行链接：列号累计从每链接重复扫描前缀改为每行单调扫描；上下文复制从每链接一整行降到最多320个Unicode标量。反引号扫描以一次分词和等长索引代替重复搜索余文，使用O(反引号串数)辅助空间。
- 规则diff：共同首尾在线性扫描后排除，LCS仅作用于变化区间，连续表最多约16MB；超预算仍保守报告变化区间整体替换。约束检查覆盖所有删除事件，展示容量和判定容量分离。
- Gateway：请求身份计算与请求大小相关，但历史槽不再保存请求正文；每次完成用队列取最早结果加有序表查询，消除随全历史大小增长的逐出扫描。历史ID墓碑是防重放语义的一部分，本轮未删。
- 项目检索：定位开头O(log N)，仅枚举指定项目的K篇文档；原有100条展示上限、101条截断判定、Unicode语义和结果顺序不变。

以上是数据结构与操作范围的变化，不是实测吞吐倍率；本轮使用确定性大输入和状态断言，未用易波动的耗时阈值作为正确性门槛。

## 新增回归范围（25项）

| 文件 | 项数 | 行为证据 |
|---|---:|---|
| `vault-index/src/markdown_scan_tests.rs` | 3 | 转义/未闭合语义、7776个短组合与旧实现一致、约2MiB异长反引号输入 |
| `vault-index/tests/bounded_markdown_contract.rs` | 5 | 320/321字符边界、Unicode列号、1024密集链接、锚点唯一性、缓存版本拒绝与往返 |
| `policy/tests/rules_diff_contract.rs` | 6 | 展示上限外约束、LCS熔断、3000行未变、局部修改原行号、扩大项溢出摘要、20000空行 |
| `gateway/src/retention_tests.rs` | 5 | 固定身份、4097调用逐出、完成顺序、所有请求字段、JSON类型/对象键序/数值等价 |
| `gateway/src/admission_guard_tests.rs` | 5 | invoke/relay二次guard与拒绝审计panic，重放授权panic保留原结果 |
| `workbench/src/search_scope_tests.rs` | 1 | 2000相邻项目、Unicode路径、空范围及指定子树 |

Gateway异常回归在捕获panic后先确认没有InFlight槽，再同步重试，避免测试本身永久等待；
Policy回归走真实提交和激活，确认发布重校验与审阅的扩大项一致。既有契约继续保留。

## 验证与交付

2026-09-29 在 githubci 原生执行，整轮退出码0；Mac未运行测试、fmt、clippy或build。

- `cargo --locked fmt --all -- --check`：通过（先在同一Linux树格式化，并按本次改动清单同步回本机）。
- `cargo --locked clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo --locked test --workspace`：2818 passed，0 failed，8 ignored。忽略入口含5个由父测试受控调用的子进程用例，另3个管理员/root/systemd平台验收未在本轮执行；不把它们冒称通过。
- `cargo --locked test -p studiolo-vault-broker --test linux_path_security`：8 passed。
- `cargo --locked test -p studiolo-vault-broker --example t132-restore-probe`：4 passed。
- `studiolo-local-server` 的 `studiolo-auth/production` 特征构建、`studiolo-auth-admin --features production` 构建及真实CLI进程契约：通过。
- `studiolo-agent-server` 构建及 `check-replica-pipeline.py` / `check-client-projection-pipeline.py`：通过，含真实进程、批次切换、漂移拒绝、策略导出、安装与吊销路径。
- 本地静态 `git diff --check`：通过。源码修复过程中没有依靠反复CI推送调试；实现冻结后一次Linux整体验收全过。

原始本轮日志：`/private/tmp/studiolo-t174/linux.log`。实施分支 `codex/t174-algorithm-design`；
后续PR、CI与合入回执登记在Vault的T-174交付记录，提交前此文不预写合并成功。
生产服务未部署或重启；本轮是已验证的源码改进。

## 生产与整体验收边界

T-132的真实Bridge启用、NAS凭据、两周试跑，以及T-093恢复正向闭环不属于此次代码修复的
可替代证据；该运维写域仍由原任务占用。实现、Linux验收、main合入与生产部署分别记录。
