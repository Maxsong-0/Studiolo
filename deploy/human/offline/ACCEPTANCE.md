# T-094 人侧离线恢复撤权与异机回执（2026-09-20）

已完成获准108恢复副本的实际撤权、v2因子异机重封装、审计与保留状态核验；生产没有执行恢复撤权。
最终有效工件为 **full-offline-r2.tar.age**。恢复后真人重新登录/L3、新agent身份正向供给仍未验收，
不能把本记录视为T-093完整灾备闭环。源码及部署材料尚未提交推送，新Auth程序未部署生产。

## 冻结点与最终工件

| 项目 | 证据 |
|---|---|
| snapshot_id | `t094-20260920-ql5a20r3` |
| 共同停写 | `2026-09-20T21:20:06.707983+00:00` 至 `21:20:08.153252+00:00`；约1.445269秒 |
| 包版本 | 外层schema2，tool_revision=2；r2复用同一冻结数据，未再次停写 |
| 最终密文 | `full-offline-r2.tar.age`，98,276,984字节 |
| 密文SHA-256 | `7739729f7157bfe4003fec7ca16873546eb44aafba55c60fde747e7494475a90` |
| 外层manifest SHA-256 | `4865d592b03ba3d7b4c5e0395af0f57c24d8a8b9a77c48ba9cd97b43f7ba32fc` |
| 原agent程序 | `ca14d2df43d14c13293fc8b76d7870ddbf86fe59826921c6a372bd554341dd12`；M5/M6发行 |
| 原human r7程序 | `8b5f1aa5643b7e9c8934760e307c84b365babd91cd953952ede51d96805bae0a` |
| 新恢复auth-admin | `c7f96a83d80ea1128698495ff0aa7c13f1078b8ea683a3e7343edf345ecc9699` |
| 新恢复local-server | `756f48004efa2e84f13fd5965fa5d3b0e5ecbd203dbeaa30c0b16036859f2310` |

包含当前agent身份/Audit/Policy/凭据、完整白名单replica、人侧Auth v4/Audit v2/owner配置/r7完整发行、
双层age封装的v2因子，以及匹配恢复语义的新程序、实际动态库和源码来源记录。
不含canonical全Vault、Control/blackboard全量或完整系统镜像。

恢复源码基线为`50b33cfba226bae07cc7d527f474cbd46f680d20`，源码覆盖层及其逐文件摘要在包内
`tools/restore-runtime/provenance.json`记录；不可把该基线单独当作新程序的完整源码。
原r7不认识新增`scope=restore`事件，撤权后的恢复库必须使用匹配新程序，不能直接降级r7。

## 三处密文与回执

- 169：`/var/lib/studiolo-dr-export/t094-20260920-ql5a20r3/full-offline-r2.tar.age`。
- Mac：`~/.local/share/studiolo-dr/receipts/t094-20260920-ql5a20r3/full-offline-r2.tar.age`。
- NAS：`/volume1/studiolo-dr/t094-20260920-ql5a20r3/full-offline-r2.tar.age`。

三处大小/摘要一致。Mac完整age解密认证送空设备通过，明文不落Mac；私钥仅存Mac专用私有目录。
NAS在`21:37:59.017176+00:00`完成root0700目录/0600文件的不覆盖发布和fsync，仅保存密文。
Mac同目录保存`export-receipt-r2.json`、`human-restore-receipt-r2.json`、
`agent-negative-receipt.json`、`nas-receipt-r2.json`、`cleanup-receipt.json`。
导出回执里的因子绑定/撤权字段为false，表示导出当时尚未恢复；后续实际结果以独立恢复回执为准。

旧`full-offline.tar.age`、第一次失败演练和历史T-093工件保留记录，不能替代本次r2有效工件。

## 108人侧真实副本

先验证完整age认证、独立可信清单，再复制到新root私有目录；原包不改。
内层因子经有界原生管道用本批独立host key重新封装，私钥不传108，种子不进Python或日志。
CLI在独立netns、core=0、锁页环境运行；通过`Totp::attach`确认原v2绑定，不生成或处理OTP。

| 项目 | 实测 |
|---|---|
| epoch | 2→3 |
| 新增撤销会话 | 0；快照唯一会话已经过期，不能声称撤销了有效会话 |
| 清除旧L3记录 | 1；不声称其当时仍有效 |
| 废止恢复码 | 10 |
| 因子与消费证明 | 原v2、消费高水位和历史证明保留 |
| 时钟/限流 | 原clock_floor及预算逐项不变 |
| 审计 | 原Audit前缀保留；新增Intent/Outcome均获确认 |
| 同恢复ID重复执行 | 原epoch/计数/事件复用，语义状态不再变化 |
| 原备份 | 再次严格清单核验通过，未修改 |
| 真人新登录/L3 | 未执行 |
| 人侧对外监听 | 未启动 |

这些实测证明副本持久撤权与因子绑定；没有索取原Cookie或恢复码，未做真实秘密的HTTP重放。
旧会话全部失效的契约由epoch和定向Auth测试覆盖，不能据本次已过期样本宣称活跃Cookie重放验收。

## 108agent真实副本与清理

同一冻结点的agent材料在独立netns实际启动原生M5/M6程序，撤销副本唯一原类型；
原token的initialize真实返回401。未签发新身份，`positive_supply_check=false`。
该材料来自r1解包副本，最终清理前已核验其冻结数据与r2逐项相同。

`21:41:23.968548+00:00`确认任务单元inactive/MainPID0、无进程引用，核验目录属主/设备/节点后，
仅删除本批6个临时明文/工具/夹具目录和私有host key。108默认systemd host key始终不存在，未改共享密钥。
清理是逻辑删除，不承诺物理擦除；Mac/NAS密文保留。

随后生产核验：human、agent、Tunnel均active且NRestarts=0；Auth仍epoch2、唯一v2、10枚恢复码、restore事件0。
生产human继续r7，未部署新撤权程序或执行生产撤权。

## 测试与实际发现的修复

| 测试组 | 结果 |
|---|---|
| 107快照与schema2/schema1 | 29+7+22共58项通过，无ResourceWarning |
| 107Auth既有契约 | 前轮237项通过 |
| 修复后107Auth恢复契约 | 16项通过 |
| 修复后107AuthAdmin | 34项通过；root专用1项在108另跑通过 |
| 107原生C过滤器 | 9项通过，`-Wall -Wextra -Werror` |
| 108虚构管线/完整加密 | 9+5项通过，不使用生产OTP |
| 共同停写导出控制流 | 8项通过 |
| production构建/定向Clippy | auth-admin及local-server构建通过；auth/auth-admin Clippy零警告 |

初次导出因Type=simple先报告active、launcher尚未exec正式程序而被拒；服务已恢复。
修正为启动后有界等待正式程序并核对固定摘要，未降低核验要求。

首次人侧演练发现普通`open_existing`及`TotpEnrollment::state()`会推进运行时钟；该次保留状态核验失败。
新增无时钟观察的`open_existing_for_restore`，移除状态投影，并用领先一小时的虚构clock夹具抓回归。
修复后重建r2、从原冻结包复制新目录，完整真实状态核验才通过；没有修改失败副本冒充首次成功。

## 余项与适用边界

T-094新增能力及获准副本验证已完成。T-093仍需真实新agent身份正向供给、恢复人侧重新登录/L3、
旧MCP会话/活跃Cookie的端到端拒绝，以及完整恢复RTO/RPO测量。
共同停写1.445269秒是备份中断时间，不是RTO；冻结时间是恢复点，不是已证明的RPO目标。
canonical全Vault与其他未纳入组件另需明确备份范围，不能用白名单replica冒充整库灾备。
下一次演练须重新建立隔离副本并复核来源和宿主状态；本批已清理，不存在可直接登录的恢复服务。

操作器与固定批次限制见[README](README.md)，加密流见[PORTABLE](PORTABLE.md)，
恢复语义见[Auth恢复契约](../../../docs/contracts/auth-restore.md)。
