# 人侧接入方列表（T202／T151）

`GET /api/v1/agents` 属于已有 L2 主人会话门禁；不接受任何查询串、POST 或 HEAD。与策略动作共用两个阻塞许可，未登录 401、方法不符 405、带查询串 400。未装策略平面回 404 `policy_plane_absent`，未装身份视图回 404 `agents_plane_absent`；身份库／策略库／关系链不可用回固定中文 503 `agents_unavailable`，不返回宿主路径或半份列表。

可信启动层显式传 `--policy-data /绝对策略目录 --agent-data /绝对身份目录`；身份目录必须已有，须与内容、静态资源、认证、审计、策略、写者和控制目录隔离。同给 `--board-agent-data` 时两者必须相同。只增加本参数的候选发行接线；本功能不激活生产发行、不签发或轮换凭据。

## 身份能力边界

`studiolo_supply::AgentIdentityView` 与 `AgentStore` 复用唯一受控 SQLite 开库点和路径身份核验；视图使用 `READ_ONLY | NO_MUTEX | NOFOLLOW | PRIVATE_CACHE`，仅打开已有格式 0／1、核验完整 schema、quick check 和外键，拒绝非 WAL 库。只读开库不设置磁盘 journal mode、不初始化、不迁移、不补表。WAL 读取可能建立/写共享内存侧文件，故同 UID 的私有目录仍须允许 SQLite 协调。

视图公开类型、实例（含父号）、类型有效性、自身到根的链和快照；没有写入方法或凭据摘要出口。黑板 `BoardPlane.identities` 同样持视图。源码守卫禁止供给与 agent-server 之外的生产模块导入写者或调用身份写方法；SQLite 连接实际拒写的测试不能只由源码扫描替代。

今后升级身份格式时，必须先交付能读取新格式的人侧程序，再执行显式身份库升级。

## 封闭响应

```json
{
  "generated_at": "2026-11-13T12:00:00Z",
  "effective_hash": "64 位小写十六进制",
  "global_revoked": false,
  "children_enabled": true,
  "types": [{"name":"claude","issued_at":"RFC3339","identity_revoked_at":null,"capability_version":1,"policy_revocation":null}],
  "instances": [{"id":"ULID","agent_type":"claude","parent":null,"registered_at":"RFC3339","last_seen_at":"RFC3339","denial":null,"policy_revocation":null}],
  "instances_truncated": false
}
```

实例按 `(registered_at DESC, id DESC)` 取最近 500 条，多取第 501 条判定截断；格式 0 的父实例恒为空且 `children_enabled=false`。窗口外祖先仍参与权限判定，链最多四层；循环、跨类型或缺失父链使整份读取失败。类型与返回实例及其祖先取自同一身份库读事务；策略摘要、能力版本、全局／类型／实例吊销取自另一份完整策略库读事务。两个库没有跨库原子性保证，此接口是观察快照，每次真正调用仍实时复核。

`denial` 为空或 `{reason, by}`。身份类型凭据失效时先报告 `identity_type_revoked`；否则与接入方共用 `sql::authorization`，直接复核顺序为 `global_revoked` → `type_revoked` → `instance_revoked` → `no_capability_set`。直接复核通过而祖先实例被吊销时报告 `ancestor_revoked`，`by` 是从自身向根找到的第一个被吊销祖先；其他原因的 `by` 为 `null`。`policy_revocation` 为原策略吊销记录的公开字段，不靠历史列表的 1000 条窗口决定授权。列表绝不返回 `token_sha256`、`session_digest` 或读取记录。

`last_seen_at` 的含义是最近一次授权调用；实例初次登记时以登记时间初始化。没有心跳或连接存活保证。人侧读取本列表不更新它、不写身份库。

## 页面与动作

策略页第四个标签「接入方」按类型分组，默认筛选最近 24 小时有授权调用／登记的实例。时间窗以响应快照时间计算；深链接 `#view=policy&panel=agents&item=<实例号>` 保留目标行并移焦，目标超出窗口时明确说明不在当前列表。绝对时间可在时间文本悬停查看。

当前认领由 `/api/v1/board/overview` 客户端联接，只计快照时仍 active 且租约未到期的持有项；黑板未装配显示 `—`，读取失败显示“暂不可用”，截断时数量标“至少”。协作页持有者与命令面板均可导航到本列表。

行内撤销先预填级别和目标，填写理由并打开执行预览；确认才调用既有 `POST /api/v1/agents/revoke`，成功后重新读取列表并恢复目标行焦点。实例撤销影响自身及子孙实例，同类型其他独立实例不受影响；实例不可恢复。类型策略恢复只打开当前能力集草案，经校验和 L3 发布处理，不直接放行；身份凭据吊销须由身份管理员重新签发。读取审计四维查询仍由 F8 后续任务接入。

## 验证入口

所有验证交 108 testbox；不在本机编译或测试。定向覆盖：`studiolo-supply` 身份视图/子实例/格式契约，`studiolo-policy` 授权快照契约，`studiolo-local-server` 策略 HTTP、启动参数/目录隔离与黑板契约，`studiolo` 架构守卫，前端接入方/策略/协作/命令面板契约及构建、lint、格式检查。生产装配与手机实测须按部署/真机验收单独记录。
