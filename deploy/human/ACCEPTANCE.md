# T-092 部署回执（2026-09-20，人侧上线验收完成）

- 产品来源：main `50b33cfba226bae07cc7d527f474cbd46f680d20`；生产 release 构建含 auth/production、不含 test-support。CLI 合成前台/拒绝测试通过。
- 107 原生：factor_contract 14、policy_http_contract 3、unix_ingress_contract 15，共32项通过；web npm ci、tsc、vite 构建通过。
- 169：systemd 259.5；r2 包 manifest SHA-256 `b0a73612a95f38478b4c190df2bbd0c43af166d3d775ecff4b75fa565a99860a`。运行依赖齐全，systemd-analyze verify通过（只有系统已有xfs CPUAccounting警告）。
- 虚构systemd凭据：目录0550、文件0440、root:root，ACL只授予UID1000读取/遍历；实装启动器读取和ACL检查通过。真实凭据未由agent生成或读取。
- 配对工具r2：非TTY拒绝通过且无Auth/密文状态变更；三个FD同控制终端会话，密文先写完/fsync再不覆盖原子发布。
- 真实主人首次配对已提交：Auth v4、enrolled_factor_count=1、pending_factor_operations=0。正式systemd单元已安装并启用，active/running、MainPID12502、NRestarts0；socket0700父目录/0600文件，未认证请求401。L2/L3、策略发布/回滚、恢复码仍未验收；Cloudflare新路由表单已准备，尚未保存，等待明确切换确认。

## 当前装配与回退

配置已准备在 `/home/maxsong/.config/studiolo-human/owner.json`；独立Auth/Audit目录已准备。共享Policy仍为既有agent-policy，Auth必须始终与human-audit配对。

Cloudflare Tunnel `studiolo-preview` 的现状经界面核验：人侧 `studiolo.maxsong.me` → `http://127.0.0.1:4319`，httpHostHeader=`127.0.0.1:4319`；agent域名 → `http://127.0.0.1:4400`。人侧单元健康且配对完成后才切人侧至 `unix:/run/studiolo-human/http.sock` 与Host=`studiolo.maxsong.me`；保留现有Access单邮箱策略。当前尚未做该更改。

旧只读人侧服务仍在，恢复原人侧route和Host即可回退入口；不得回滚/删除新增Auth、因子、审计或共用Policy。r1材料保留且未激活；正式候选为r2。


## 后续优先状态：首次因子暴露已隔离

用户将首次配对材料贴入聊天后，原登记不能继续作为正式上线因子。人侧单元已stop和disable，旧活动密文移到 `/var/lib/studiolo-human-quarantine/t092-exposed-enrollment-yaghjdqh/`，未复制秘密到本回执。Auth/Audit未重建或清空。只读计数：sessions0、totp_grants0、totp_pending0、enrolled1。

r3原生替换材料已冻结验包，5项合成终端/密文发布测试通过，生产systemd语法及非TTY拒绝通过。r3去掉policy-data和Policy写路径，原Access入口尚未切换。准备步骤只给主人私有TTY生成新候选；下一步仍需真实网页登录、恢复码、新手机OTP完成原生替换，之后重新登录并生成正式恢复码。详见replacement/README.md。

## 主人已准备候选、恢复入口启用（2026-09-20）

主人已确认“新因子准备好了”。r3完整41文件验包、两槽密文元数据、原Auth v4/old_enrolled1/会话及pending0核验后，安装并启动恢复单元；active/running、NRestarts0，仍disabled以待正式收口。操作记录 `/var/lib/studiolo-deployments/t092-replacement-80iydz0h`；原单元另存，不回滚或重建Auth/Audit。Unix父0700/socket0600、六个真实端点无Access均401，策略写路径未开放。首次探针误用不存在的/api/v1/auth/login得到405，按契约修正为/api/v1/sessions后六项通过；不把该405算产品故障。

用户当次明确确认后，Cloudflare人侧route已保存为unix:/run/studiolo-human/http.sock，Host=studiolo.maxsong.me；UI路由列表确认，agent仍http://127.0.0.1:4400。匿名实际浏览器仍先进入Studiolo Preview Access邮箱登录。已将浏览器交给主人按真实旧OTP→生成恢复码→新OTP+恢复码执行恢复；尚未收到恢复完成，未观察秘密画面或读取Cookie/验证码。

r5唯一v2正式候选已冻结，manifest SHA-256 `715421e436d1b892807ed30a360209cca8582fbf3e3dfdc45169f0878cf82aad`，原生虚构装配与systemd语法通过。未激活；完成真实因子替换后才按current-v2/README.md推进。真正L3、恢复后重新登录/最终恢复码、策略发布回滚仍未验收。

## 已修复登录后502（2026-09-20）

前一轮仅验证匿名Access页面，遗漏了Access之后的真实源站链路。用户实际遇到502。内核证实旧用户Tunnel进程继承unprivileged_userns AppArmor，Unix connect因disconnected path被拒；并非human服务或凭据故障。

经相同hardening无凭据预检后，Tunnel迁为系统级同UID单元，保留原凭据/所有hardening/socket0700/0600/Access，原用户单元停止并disabled。切换记录 `/var/lib/studiolo-deployments/t092-tunnel-system-smn57eoj`，中断约0.886秒。真实新进程与human同initial userns；四连接就绪、请求错误0、无新LSM拒绝，人侧/agent健康且NRestarts0。用户原502页面刷新已显示工作台及账户按钮；agent公网无凭据仍401。已把网页交给主人继续因子替换，未观察或读取秘密。维护方式改为sudo systemctl，不再用systemctl --user。详见tunnel/README.md。


## 旧手机条目已删除，r6 再交付工具已就绪（2026-09-20）

主人进一步说明旧条目早已删除，不能继续让其尝试旧手机码。生产只读状态仍为 v1 enrolled、pending0、恢复码0；原隔离密文按 totp-seed 认证解密到空设备通过，未读取明文。保留当前原生恢复路线，新增 r6 主人私有 TTY 只读再交付工具；不使用聊天中的旧密钥，不改 Auth/凭据/尝试预算、不计算 OTP。

冻结目录 `/home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r6-restore-old`，helper SHA-256 `455180e945eafcd081c5bd9b874ccf18e60d1ea619b2333f0c83bb61ef5d2d33`。初始、回车后解密前、解密后显示前三次确认仍为 v1 且 pending 空。三次检查不构成与其他认证操作的原子互斥；并发前滚后旧屏幕不恢复旧权限。helper 不持 Auth 写锁、不停服务，root 前台 TTY/内存保护、严格文件门禁与临时清屏均保留。

7 项完整真实 SSH PTY 虚构测试全部通过（成功、隐藏重提示、Ctrl+C、坏密文、未登记、已前滚、pending）；各例文件状态不变。真实非 TTY 在访问认证材料前拒绝，生产密文/Auth 元数据不变，旧活动槽仍为空。human/Tunnel/agent active/running、NRestarts0。源文件/测试/说明落独立工作树 deploy/human/restore-old，未提交推送。

给主人执行：`ssh -t maxsong@192.168.100.169 'sudo python3 -I -B -S /home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r6-restore-old/restore-old-factor.py'`。添加“Studiolo 旧因子（仅用于替换）”，保留新条目；旧码二次认证→生成并保存恢复码→一枚恢复码+新条目码恢复验证器。看到成功并退出全部会话后回复“恢复完成”。仍待主人实际完成；r5 未激活，L3/最终恢复码/策略验收未完成。


## 主人报告恢复无反馈，尚未完成因子切换（2026-09-20 18:35–18:37 UTC）

只读核对：权威v1 enrolled，v2未生效；totp_grants1、recovery_codes10、totp_pending/recovery_pending0，1个活动L2。进一步聚合确认10行consumption/intent/outcome均空，used_mask0，recovery高水位尚未消费，replacement proof无，recovery_rates空。因此旧因子二次认证及恢复码生成已完成；恢复尚未进入恢复码/候选OTP匹配和消费。不能凭预算空断言浏览器绝未发POST，也不能将累计expired拒绝归于这一次。现有会话时限有效，三个服务active/running、NRestarts0；无认证状态修改或重试。

浏览器仅读取标题/按钮/公开状态/恢复端点错误状态，未读输入、Cookie、恢复码列表或截图。观察当时停在工作台、账户弹窗关闭、无恢复端点控制台错误。已重新打开账户与恢复表单，确认“确认恢复验证器”可见/可用，无alert；交给主人自己填写已保存的一枚完整恢复码与新手机条目当前OTP并提交。用户所指“无反馈”的具体现象已询问，尚待回答；无根因定论。

源码只读排查发现候选分支：格式校验拒绝在前端显示错误且不POST；并发/寿命stopped可能只重置表单未新增panel提示；成功204必须切到登录页并显示恢复成功消息。不能把这些可能性当作本次复现。未修改产品源码或部署新服务；正式v2与策略验收仍等待真正恢复提交完成。


## 新因子真实替换与唯一v2正式服务已完成（2026-09-20 18:43–18:49 UTC）

主人回复“好了”后只读确认v2 enrolled、替换证明存在、双pending0、旧L3/恢复码0。Auth代次已前滚，旧代次会话0，当前1个活动会话建立于替换之后；因此旧会话失效且主人已重新登录。不把上一轮无反馈推断为服务成功，本次以真实持久绑定及证明为准。

r5 39文件manifest全过；切换记录 `/var/lib/studiolo-deployments/t092-current-v2-jeqer7ky`。18:47:22.836534Z停r3到18:47:24.033600Z新服务通过并enable约1.197秒。原socket由产品清理，部署者未改Auth/Audit/Policy；备份原非秘密配置与unit，仅owner.seed_ref前滚v2并安装已验r5单元。PID29084、active/running/enabled、NRestarts0；运行时凭据仅totp-next，旧槽不再加载；六个真实端点无认证均401，Unix父0700/socket0600。稍后复核仍稳定，Tunnel四连接、请求错误0。

实际公网页面刷新、已登录工作台和账户正常；恢复候选按钮消失。已将新因子“二次认证”表单交主人，要求新手机OTP→生成最终10枚恢复码→密码管理器保存→收起，只回复“新恢复码已保存”。读取仅按钮/提示，未读取输入、码列表、Cookie或环境秘密。还未收到保存确认；新L3及最终恢复码/策略发布回滚不提前报完成。源码/部署材料仍未提交推送。


## 新因子最终恢复码与 r7 策略页修复（2026-09-20 18:52–19:12 UTC）

主人回复“新恢复码已保存”。只读聚合已确认v2 enrolled、恢复码10枚/1批/消费0、双pending0，并曾存在真实有效新L3；保存证据来自主人确认。此后L3自然过期，不能把该快照当作当前持续有效。

独立测试subject `t092-policy-check` 的零权限desired `01M302NMSVZ683PCFPN3MECDRQ` 已提交/校验，无权限扩大。没有签发身份或token；目前尚未发布。基线effective `d755edd21773df5265258f5c55f639e37e4777453e0b244b406fd992e0a45b81`，仅Claude能力集v1，正文SHA `b3afa48dd486ac7df442c525750309adc2c8679a9c89e1196708c1a9da129fa4`。

提交后刷新曾出现“认证结果暂不能确认”。定向复现发现两个三路Promise.all都撞上auth_http的两个try_acquire_owned并发许可；新回归测试在旧实现3/3失败。策略初读与reload改用共同顺序读取函数，不改服务端许可、认证、CAS或写操作重试。107原生6/6策略测试、eslint、tsc及vite build全过，只有既有构建chunk大小提示；4个源码文件SHA与已测试远端一致。原先“恢复验证器无反馈”仍未建立根因，不混称本次问题。

r7新包41文件全验，manifest `6b30030657a0807ed50abac5a106b38fa3b03602b6ff2e0de04071636aa0266d`；Rust二进制仍为r5同一SHA `8b5f1aa5643b7e9c8934760e307c84b365babd91cd953952ede51d96805bae0a`。来源main50b33cf加尚未提交的4文件web修复；包内source-evidence.json/source.patch记录差异，不冒称已经合并。安装unit SHA `66d3689f8d73a42edf1ba56364cc8a3212b4533ce91ca8bffe09d7455be5d847`，launcher SHA `ad31e1e60fbcda0680ce526ea455d692e5b9cf2e4e66d6159f63663b965f3665`。

部署记录 `/var/lib/studiolo-deployments/t092-web-r7-981qatht`。19:09:47.500991Z停r5，19:09:48.484843Z r7验收完成约0.984秒；PID31114、active/running/enabled、NRestarts0、唯一凭据totp-next。部署者未改owner配置/Auth/Audit/Policy；socket由产品正常清理，Unix权限和六个匿名端点401通过。r5不可变包保留。

公网页面确认URL/标题正常，DOM script为 `/assets/index-BdDJO9zj.js`。真实首次加载读出Claude v1/测试草稿；重新校验返回成功，操作后完整刷新结束按钮重新可用，alert空；截图确认页面可用。新页面无console error，观察到一条AssignmentCache/LMS adapter warning，未归因到Studiolo。主人策略发布/回滚尚未验收；按ADR-019“只有人能写effective”，已打开新OTP表单并交给主人完成二次认证后点击测试项发布。助手不读取OTP、恢复码、Cookie或秘密环境。



## 人侧发布、回滚与测试项撤销已验收（2026-09-20 19:15–19:19 UTC）

主人19:15:39.320Z完成新因子二次认证，grant deadline为19:20:39.294Z；该5分钟有效期内发布/回滚无需重复输入OTP。源码发布事务始终在PolicyStore变更前执行require_l3，即时重验Access、会话、CSRF、活动因子和近期认证；本轮未改此门禁。

主人真实操作后只读对账：19:16:16.523004003Z发布测试subject t092-policy-check 为v2（desired 01M302NMSVZ683PCFPN3MECDRQ），19:16:24.214263885Z回滚到v2生成v3（rollback_of=2）。v2/v3正文SHA均为4114108784a6df22bee6699feb6cbc78d7a0df12c710ca8b62149794aa307a75，均零权限；回滚产生新版本，未覆盖历史。本次实际回滚目标就是当前正文，证明原生回滚路径/新版本/审计，不宣称验过不同权限正文的往返。

主线准备撤销desired 01M3043536Z8P8R13S5TFH1ZT7并校验。主人点击发布，19:19:06.601348013Z生成v4 retired=true，随后明确回复“测试项已撤销”。当前仅Claude能力集v1，吊销表仍0；effective恢复基线d755edd21773df5265258f5c55f639e37e4777453e0b244b406fd992e0a45b81。测试身份/token从未创建，版本历史保留。

三次policy.activated都已持久Outcome且同event_id各有一条Intent；Outcome版本、rollback_of、retired与前后effective哈希链逐项匹配PolicyStore。事件ID分别policy.v1:policy.activated:01M303YTG2Y1TC7PX364DG0GWG（v2）、policy.v1:policy.activated:01M303Z20E4GPPY4EDBD03N5DP（v3）、policy.v1:policy.activated:01M30440K0NP4H1K9PWGER6J72（v4）。未截获真实发布HTTP回包的audit_confirmed字段，以权威持久成对记录为审计落盘证据；不冒称做过全账本哈希重放验证。

T-092人侧上线验收完成；r7产品代码/部署材料仍在独立工作树，未提交推送。T-093仍独立未完成，现有agent-only加密包不包含最终人侧Auth/新因子，也不是canonical全库备份。NAS首次管理员配置与获准现代Linux恢复目标仍需主人配合。

