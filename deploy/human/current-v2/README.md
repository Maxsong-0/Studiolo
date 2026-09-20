# T-092 原生替换后的唯一 v2 装配

**最新状态（19:19 UTC）：T-092人侧上线验收完成：真实v2、最终10枚恢复码、发布v2/回滚v3/撤销v4及配对审计已核验；测试项已移出现行策略，Claude v1及原effective摘要恢复。r7服务已启用；源码/部署材料仍未提交推送。**

当前（19:12 UTC）：r7已上线，真实v2已生效，新因子L3与最终10枚恢复码已核验，主人已确认保存；r7策略页并发读取修复已部署并通过公网加载/重新校验刷新；等待主人零权限测试项发布，再验收回滚。 下文r5为首次切换历史；activate.py是已执行的r3→r5操作，禁止重跑。当前launch-human.py和service指向r7；activate-web-r7.py是已执行的r5→r7限定部署，旧单元摘要门禁阻止重跑。

仅在原Auth已通过原生replace_factor前滚到studiolo-human-totp-v2，且totp_pending/recovery_pending为空时使用。不得通过此材料跳过真实网页替换；不要清库或重新登记。

正式包已冻结于169 `/home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r5`，manifest SHA-256 `715421e436d1b892807ed30a360209cca8582fbf3e3dfdc45169f0878cf82aad`。产品与网页沿用main50b33cf。新增材料仅更换release路径，并将唯一systemd凭据ID改为totp-next、来源改为v2密文；恢复正常已有PolicyStore预检/装配和专用写路径。不包含替换候选env或replacement-seed-ref，也不加载隔离的旧凭据。

`totp-next`是已加密凭据的认证名称，必须保留；launcher把它送入当前因子的STUDIOLO_TOTP_SEED_HEX槽。这个名称不表示运行时仍是候选。不要直接改为totp-seed，否则systemd名称认证会失败；无需读取或重新加密秘密。

已通过169原生虚构秘密exec装配验证和systemd-analyze verify。2026-09-20 18:47 UTC已完成正式安装/启用，实际进程只加载v2。新因子真实L3及最终恢复码、策略发布/回滚验收仍独立等待；不能把服务健康当成业务验收。

继续顺序：主人网页看到恢复成功并退出→只读核对Auth真实v2绑定和pending空→停止r3并等产品清理自身socket→备份非秘密owner配置与当前单元→仅把owner.json.seed_ref前滚为v2（原owner/Audience/Auth/Audit不变）→安装r5单元并启动→验证仅有totp-next credential、无旧替换slot、Unix权限/拒绝/进程健康→enable正式单元→主人重新登录，用更新30秒时间步的v2 OTP提权，并生成保存最终恢复码→策略发布/回滚真实验收。

若启动失败，保持停止并诊断；不得清socket、恢复Auth快照、重建库或回到r2/v1因子。必要时只能在核实绑定后使用仍能按持久状态选v2的r3恢复装配。旧加密凭据仍在root私有隔离区，停止加载不等于删除历史证据。


## 2026-09-20 正式切换回执

18:43 UTC只读确认真实v2 enrolled、替换证明存在、双pending0，旧恢复码及旧L3全清。Auth代次已前滚，旧代次会话0；当前唯一会话是替换后新登录。18:47:22.836534Z停r3，18:47:24.033600Z正式验收通过并enable，约1.197秒。39文件manifest一致、原socket由产品正常清理；仅owner.seed_ref前滚，原Auth/Audit/Policy保留。

操作记录 `/var/lib/studiolo-deployments/t092-current-v2-jeqer7ky`，包含原非秘密配置/单元及验收回执；部署脚本为本目录activate.py，已执行一次，前置门禁禁止在v2配置下重跑。主进程PID29084为r5产品，active/running/enabled、NRestarts0；运行时凭据目录仅totp-next，旧槽不再加载；父0700/socket0600，六个真实端点未认证均401。

公网实际页面刷新后工作台与账户正常，安全面板无恢复候选按钮。Tunnel四连接/错误0。已打开新因子二次认证表单并交主人完成更新OTP及最终10枚恢复码保存。旧批已整批失效。没有读取真实OTP、恢复码、Cookie或进程秘密环境。


## r7网页修复

r7新包41文件全验，manifest `6b30030657a0807ed50abac5a106b38fa3b03602b6ff2e0de04071636aa0266d`；Rust二进制仍为r5同一SHA `8b5f1aa5643b7e9c8934760e307c84b365babd91cd953952ede51d96805bae0a`。来源main50b33cf加尚未提交的4文件web修复；包内source-evidence.json/source.patch记录差异，不冒称已经合并。安装unit SHA `66d3689f8d73a42edf1ba56364cc8a3212b4533ce91ca8bffe09d7455be5d847`，launcher SHA `ad31e1e60fbcda0680ce526ea455d692e5b9cf2e4e66d6159f63663b965f3665`。

部署记录 `/var/lib/studiolo-deployments/t092-web-r7-981qatht`。19:09:47.500991Z停r5，19:09:48.484843Z r7验收完成约0.984秒；PID31114、active/running/enabled、NRestarts0、唯一凭据totp-next。部署者未改owner配置/Auth/Audit/Policy；socket由产品正常清理，Unix权限和六个匿名端点401通过。r5不可变包保留。


真实页面已加载新asset并完成草稿重新校验后的刷新。最终发布和回滚仍需要主人执行，详见../ACCEPTANCE.md。


## 人侧发布、回滚与测试项撤销已验收（2026-09-20 19:15–19:19 UTC）

主人19:15:39.320Z完成新因子二次认证，grant deadline为19:20:39.294Z；该5分钟有效期内发布/回滚无需重复输入OTP。源码发布事务始终在PolicyStore变更前执行require_l3，即时重验Access、会话、CSRF、活动因子和近期认证；本轮未改此门禁。

主人真实操作后只读对账：19:16:16.523004003Z发布测试subject t092-policy-check 为v2（desired 01M302NMSVZ683PCFPN3MECDRQ），19:16:24.214263885Z回滚到v2生成v3（rollback_of=2）。v2/v3正文SHA均为4114108784a6df22bee6699feb6cbc78d7a0df12c710ca8b62149794aa307a75，均零权限；回滚产生新版本，未覆盖历史。本次实际回滚目标就是当前正文，证明原生回滚路径/新版本/审计，不宣称验过不同权限正文的往返。

主线准备撤销desired 01M3043536Z8P8R13S5TFH1ZT7并校验。主人点击发布，19:19:06.601348013Z生成v4 retired=true，随后明确回复“测试项已撤销”。当前仅Claude能力集v1，吊销表仍0；effective恢复基线d755edd21773df5265258f5c55f639e37e4777453e0b244b406fd992e0a45b81。测试身份/token从未创建，版本历史保留。

三次policy.activated都已持久Outcome且同event_id各有一条Intent；Outcome版本、rollback_of、retired与前后effective哈希链逐项匹配PolicyStore。事件ID分别policy.v1:policy.activated:01M303YTG2Y1TC7PX364DG0GWG（v2）、policy.v1:policy.activated:01M303Z20E4GPPY4EDBD03N5DP（v3）、policy.v1:policy.activated:01M30440K0NP4H1K9PWGER6J72（v4）。未截获真实发布HTTP回包的audit_confirmed字段，以权威持久成对记录为审计落盘证据；不冒称做过全账本哈希重放验证。

T-092人侧上线验收完成；r7产品代码/部署材料仍在独立工作树，未提交推送。T-093仍独立未完成，现有agent-only加密包不包含最终人侧Auth/新因子，也不是canonical全库备份。NAS首次管理员配置与获准现代Linux恢复目标仍需主人配合。

