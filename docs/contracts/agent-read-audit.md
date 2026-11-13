# 人侧读取审计查询（T203／F8）

`POST /api/v1/agents/reads/query` 是 L2 主人只读查询。复用策略端点唯一方法表、会话逐次复核、CSRF、严格 Origin/Fetch Metadata、禁止任何查询串和两个阻塞许可；不要求 L3，不追加查询自己的审计。POST 用于将筛选条件留在请求体中，不写入地址。未登录401，方法不符405，语法/未知字段/非法过滤400，正文超限413；未装策略404 `policy_plane_absent`，未装读取视图404 `reads_plane_absent`，账本不可用或校验失败503 `reads_unavailable`。失败不返回部分记录、路径或原始异常。

## 输入与分页

```json
{"outcome":"read","agent_type":null,"instance_id":null,"since":null,"until":null,"path_prefix":null,"before":null}
```

请求最多4096字节；`outcome` 必填且只能为 `read`／`denied`，其他字段可缺省或为null。类型遵守已有类型名语法；实例26位ASCII字母数字；时间为带时区RFC3339，含上下边界且起点不晚于终点。路径前缀按字面匹配，最多1024 UTF-8字节、不可为空或包含控制字符，`%`不作通配。拒绝动作没有保存路径，`denied`带非空路径条件一律400，不推断路径。

`before` 为安全整数范围内的正数，是账本offset排他上界；null从最新处开始。每次在一个Deferred读事务内，从最高可用offset向前检查至多256条，完整核验涉及的段、段链及打开段后才交付封闭投影。返回记录新到旧；`next_before`是下一窗口上界，null表示到达账本起点。筛选后空页仍可有下一窗口，不能由空页推断全库没有匹配。后续追加不改变已有offset分页；客户端固定本次查询的截止时间及第一页 `head_offset+1` 排他上界，往返翻页不会被新追加挤动，重新查询才纳入新事件。

```json
{"generated_at":"2026-11-13T12:00:00Z","outcome":"read","head_offset":1,"scanned":1,"next_before":null,"records":[{"offset":1,"recorded_at":"2026-11-13T11:00:00Z","agent_type":"claude","instance_id":"01K00000000000000000000001","action":"read_note","path":"项目/A/a.md","revision":"版本字符串","reason":null}]}
```

外层6字段、记录8字段封闭。成功读取的`path`/`revision`必有、`reason`为空；拒绝的`path`/`revision`恒为空、`reason`必有，`action`为被拒动作。未认证主体保留null；不返回正文、凭据摘要或原始审计JSON。版本可能为内容哈希、策略`v<n>`或`-`，不误当成固定哈希。记录投影字符串长度有界，原始审计正文逐条验证，不累积整页大正文。

## 能力与装配

`AuditReadView` 不实现`AuditLedger`写者接口，复用唯一受控连接入口的SQLite READ_ONLY标志；不建库、不初始化、不迁移、不设置journal mode，拒绝非WAL、未知格式、替换路径和损坏段。WAL共享内存协调仍遵守原目录/侧文件规则。范围之外的封存段内部只在查到时验证，不能把一次分页说成全账本内容验证；段链和打开段每次检查，代价仍随段数增长。

`AgentIdentityView::read_audit()`只从已批准身份根的固定`audit`子目录打开视图。可信启动层在显式`--agent-data`时同时装配身份和审计视图；现有agent初始化就生成此布局。缺失或损坏账本拒绝启动，不自动补建。手工只装身份视图的调用方仍可用实例列表，审计端点返回404；不新增生产启动参数。

网页位于策略「接入方」的审计查询区，默认最近24小时；手动提交查询、成功/拒绝切换、四维筛选、上一页/更早页、取消、实例跳转。筛选条件变化或离开页面会取消请求，迟到结果不得重新显示。真实生产发布、手机Safari和真实账本签收仍按单独发行验收，不由开发夹具替代。

## 验证

全部交108 testbox：audit-ledger真实只读SQL拒写、跨连接/跨段分页/篡改/替换；supply四维过滤、匿名拒绝、稀疏页；local-server认证/CSRF/方法/参数/404/503及封闭字段；前端解码、稀疏分页、取消与卸载、构建、lint和格式。最终整批全量另做。
