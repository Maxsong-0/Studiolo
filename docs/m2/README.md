# M2 · 远程与可恢复

2026-09-04 用户要求夜间开发、合并与部署。现行 M2 对应旧 PRD 18.3，
不是 Demo/Runner 闭环；权威范围仍在 Vault 的 roadmap、PRD 与 ADR 中。

## 当前状态

- T-021～T-025、T-028～T-029已验收：认证前后端、canonical真实读取及单写者/未决幂等，新main实际合入与增量修正。当前macOS747/Linux758和两端生产通过；前端沿用T-025的125项与[8类浏览器记录](factor-browser-qa.md)。继续唯一owner捕获/新鲜度/CAS；完整M2未完成。
- T-019显式安全启动已双平台验收；[启动契约](../contracts/auth-startup.md)定义共享主人配置、既有双库与审计锚点、监听前复核。该片当时macOS575/Linux583全量与生产正反检查通过，未合并部署。
- T-018应用会话HTTP已双平台契约验收；[独立契约](../contracts/auth-http.md)明确新组合入口、配对Cookie与CSRF、已登记门槛及一次有界维护，历史结果macOS519/Linux527。
- T-012 持久 AuditLedger 已完成本地与 Linux 契约验收；契约见 [审计存储](../contracts/audit-ledger.md)。
- T-013真实Access身份收口与持久会话已验收；[契约与限制](../contracts/auth-session.md)记录该片当时无HTTP，后续新增入口见T-018。
- T-014 v2有界回收、显式迁移与旧凭证拒重已通过双平台契约验收；[补充契约](../contracts/auth-retention.md)定义维护与失败边界。T-018新登录前执行一轮原维护，无独立后台调度。
- T-015预配置TOTP因子基础已双平台契约验收；[v3契约](../contracts/auth-totp.md)明确可信登记句柄、近期认证和失败发布边界，无真实登记、恢复码或HTTP接线。
- T-016恢复码/实际内部因子替换已双平台契约验收；[v4契约](../contracts/auth-recovery.md)明确单次消费、预配置新因子OTP、全会话吊销和失败边界，后续HTTP见T-024、界面见T-025；真实秘密交付另验。
- T-017可信本地登记CLI与严格环境接入已双平台验收；[命令契约](../contracts/auth-admin.md)定义隐藏TTY、no-create与显式初始化/迁移，真实主人配对/秘密交付仍未验收。
- M1 已有成果保留在当前未提交分支，完整 M1 仍未完成。M2 优先级不抹去其未完项。
- 本轮尚未完成 M2 验收、提交、合并或部署；现有示例服务与 Cloudflare 入口保持不变。
- 夜间9月5日06:51 PDT收口并暂停心跳；09:45 PDT用户明确继续后交互开发T-019，未恢复定时器，也不沿用过期07:00截止。
- 不将本地夹具、旧版本测试或模拟时间算作真实公网登录、NAS 恢复、两周同步试跑通过。
- [发布缺口](release-readiness.md)列出代码、用户输入与真实运行门槛；HEAD已合新main，继续新鲜度/CAS与余下M2，不自动发布部分成果。

## 开发顺序

持久审计 → 应用会话与安全历史回收 → TOTP与恢复码基础 → 可信本地CLI/安全启动 → 前端会话与因子HTTP → canonical CAS 与新鲜度 → 双目录同步 →
源码编辑/冲突/历史 → checkpoint/备份/恢复 → 运维与发布。

每片必须有自己的契约、负向测试和新鲜验证记录。Auth 不获得 TaskEngine 的
Control DB 写权限；HTTP 不直接获得 canonical 文件系统写权限。
会话片已登记权威授权矩阵并验收；见 [会话/TOTP 分片设计](auth-design.md)，不直接登记真实身份。

## T-012 验证记录（2026-09-04）

最新检查点为本文末T-029；下列分片数字是当时的历史结果。

- macOS：workspace 246 项 Rust 测试、fmt、严格 Clippy 全通过。
- Linux aarch64：workspace 254 项、fmt、严格 Clippy 全通过；路径安全 8 项显式复跑通过，已含在254内，不重复累计。
- 前端：17文件60项测试、lint、format、类型与生产构建通过；JS gzip115.51kB、CSS gzip7.66kB。
- Linux 固定已有镜像 `rust@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97`；Rust 1.97.1，容器启动按固定工具链补齐 Clippy，Cargo 使用 locked/offline。不是服务器 x86_64 原生验证。
- 新增22项审计测试与2项架构测试。独立审查发现并修复深度/浮点回放、读取预算及固定文件白名单；真实 SQLite FULL 与忙锁均验证失败后精确重试，不冒充 commit I/O 或断电实验。
- 首次本机专项运行被沙箱拦截 Unix 套接字夹具；按授权升级执行后完整复跑通过。补齐测试文件说明以满足 strict Clippy，未跳过失败测试。
- 本轮未修改前端源码、HTTP 路由或部署；未做新浏览器交互、真实源/身份、NAS、断电或恢复演练。M0组合根仍装配内存审计，持久Adapter待后续认证/写组合根接线。
- 全部本片新/拆分文件≤300行；未提交推送、触发远端CI或切换服务。

## T-013 验证记录（2026-09-05）

- macOS workspace **302项**（299运行时＋3身份compile-fail）；Linux aarch64 workspace **310项**（307＋3）全部通过。Linux路径安全8项另行复跑通过，已包含310内，不重复累计。
- 两端fmt和严格Clippy通过；最终macOS release生产构建、Linux dev生产构建通过；故意混用production/test-support被指定编译错误拒绝。脚本语法与diff空白检查通过，Auth文件最长242行。
- Auth58项运行时（14Access、15私有存储、29会话），Audit32项、架构12项；相较T-012全workspace新增56项。真实RS256签名＋双SQLite重开/故障/精确重放由3项专门契约覆盖，不是公网登录。
- 覆盖双期限/时间地板、秘密摘要/零化、跨库pending发布、单个/全部吊销、并发open/晚到激活、拒绝聚合与命名空间、行/回执/终态关联、真实SQLite忙锁/FULL、query_only写失败与激活前权限失效。
- 最后修复畸形/错配Intent回执及256条补记不足的失败漏计数；拒绝计数写失败不发凭证。补测跨idle审计等待与双补记者旧ACK迟到，无sleep推测交错。独立只读复审未留阻断性发现。
- Linux沿用上节固定arm64镜像；容器内固定Rust1.97.1补齐Clippy，Cargo locked/offline。没有升级业务依赖、触发远端CI，未做服务器x86_64/浏览器/真实身份/Vault/NAS/断电/commit I/O或恢复验收；前端60项仅为T-012历史结果，本片未复跑。
- codebase-design把身份、持久状态与跨库发布规则集中在Auth的小Interface；审计回执自洽沿用AuditLedger编码，不复制哈希公式。Auth不拿ControlDb或Vault能力，M0组合根仍用内存审计。
- 1024是累计未回收会话上限；下一片须设计安全保留/回收和迁移，实际验证累计超过1024仍能继续签发，防旧凭证复活并保留未交付证据。不能直接扩大常量或删除必要证据；TOTP/会话HTTP随后单独验收。
- T-013完成不代表完整M2；未提交、推送、合并、打包发布或切换任何既有服务。

## T-014 验证记录（2026-09-05）

- macOS workspace338（335运行时＋3身份compile-fail）、Linux aarch64 workspace346（343＋3）全部通过。Linux路径8项额外复跑已包含346内，不重复累计。两端fmt/严格Clippy、macOS release及Linux dev生产特征构建通过；混用production/test-support再次以指定编译错误拒绝。脚本语法/diff检查通过。
- Auth94运行时：14Access、15私有存储、65会话；本片新增36项（凭证/序号7、迁移12、回收/并发/长期17）。单个长期测试真实执行1026轮签发→全吊销→维护，epoch1027后重开仍能签发；不是SQL批造数据，也不把1026轮算成1026个测试。另有真实双SQLite回收和故障重开。
- v2使用显式迁移、持久烧号、带版本/序号的HKDF输出；固定legacy拒重集最多1024对，不为新v2积累无限墓碑。直接复用锁定hkdf0.12.4和sha2 0.10.9兼容别名，其余sha2 0.11不变。
- 稳定单计划/替换检查点，每次16终态与16epoch前缀；核对原本地证据精确审计回执、确认清理许可后，同事务复核并清理。active/pending/未交付证据保留；覆盖迟到issuer/ACK、多实例、原回执改写、计划/检查点单字段损坏、许可前后与最终Auth写失败。维护只清已终态，不主动扫描到期。
- 三案比较后收敛小Interface，不增加公开票据或新Archive Port。独立复核发现序号耗尽可回滚时间地板/到期，已改持久拒绝并验证时钟回退不可复活；旧权限故障夹具改为真实Outcome阶段触发，首次失败已完整重跑解决。Auth所有文件≤244行。
- 迁移SQL事务内失败回滚；提交后时钟等检查失败可返回Err但库已合法v2，明确记录并验证幂等重开保活。管理员一致重写/整库回滚、真实commit I/O/断电、服务器x86_64、浏览器/前端、真实身份/Vault/NAS、两周运行或恢复演练均不冒称本片验证。旧服务、原dirty分支和M1成果保留；未提交推送/fetch/合并部署。

## T-015 验证记录（2026-09-05）

- macOS workspace377（374运行时＋3身份compile-fail）、Linux aarch64 workspace385（382＋3）全部通过。Linux路径8项额外复跑已包含385内，不重复累计；两端fmt/严格Clippy、macOS release/Linux dev生产构建通过，production/test-support混用以指定编译错误拒绝。脚本语法/diff检查通过，Auth最长289行。
- Auth133运行时，本片新增39：8算法、24持久流程、4坏回执/迟到ACK、3真实双SQLite。单个长期测试通过公开接口执行128次同会话step-up，只保留1份最新grant和固定消费锚点；不是128项测试或实际多日运行。T-014的1026轮长期会话回收也在全量中再次通过。
- RFC4226/6238向量、边界/前导零/2038后、三个候选与严格防重放；账号/来源先持久预扣，成功不退款，跨Cookie/地址/重启不重置。64网络桶容量由有界数据库夹具验证，不冒称发起64个网络请求。
- 确定性交错验证并发同码、吊销后的迟到激活、旧ACK不覆盖新grant；真实双SQLite全部旧句柄销毁后重开，审计文件权限失败后恢复补记不自动授权。query_only分别覆盖预算/最终激活写失败；坏payload、错误offset与另一审计历史在写前/提交前拒绝，未实施真实commit I/O或断电实验。
- codebase-design三案收敛为小运行Interface与独立可信登记handle；显式v3只增5表/7对象，原v1/v2 DDL不改。消费锚点留存防简单last_step单列回退，pending阻止其引用的会话被GC，完整审计历史不删除。
- 先核官方接口后只新增锁定sha1 0.10.6，复用hmac 0.12.1；初次下载因沙箱DNS失败，授权升级后成功。首轮Clippy的重复fixture模块加载/手动range模式已修，未加忽略；统一复跑通过。独立只读契约复核无实质不一致。
- Linux沿用上节固定arm64镜像与Rust1.97.1，补齐rustfmt/Clippy；先locked fetch补全缓存（含既有其他目标依赖），再locked/offline检查，不升级锁定版本。不是服务器x86_64原生验收。
- 未更改前端或HTTP，未复跑前端/浏览器；无服务器x86_64、真实Access/TOTP登记、密钥部署、Vault/NAS、两周同步或恢复验收。恢复码/轮换/可信TTY/秘密装配与HTTP仍是后续门槛；未提交推送/fetch/合并部署，旧服务不变。

## T-016 验证记录（2026-09-05）

- macOS workspace421（418运行时＋3身份compile-fail）、Linux aarch64 workspace429（426＋3）全部通过。Linux路径8项额外复跑已包含429内，不重复累计；两端fmt/严格Clippy、macOS release/Linux dev生产构建通过，production/test-support混用均以指定编译错误拒绝。脚本语法/diff检查通过，Auth最长289行。
- Auth177运行时，本片新增44：6秘密/派生、2旧格式兼容、27独立恢复契约、3真实双SQLite、3坏当前回执/迟到换发、3时钟地板回归。公开接口真实64轮换发得到640唯一原码，只保留当前10码/固定控制；不是640项测试或真实长期运行。T-014的1026轮会话回收和T-015的128次step-up全量再次通过。
- codebase-design三案收敛两个完整动作：真实L3换发；L2/CSRF/新验Access加恢复码及新因子OTP直接切换，不签发恢复bearer/通用L3。先烧码/候选步，再精确原回执与双ACK，最后同Auth事务换因子、清旧L3/整批旧码、epoch推进吊销全部会话；返回句柄仍要新登录和新OTP。
- v4显式迁移只追加4表/6对象，完整31对象，旧DDL不改；当前10码加暂存10码、一笔pending，持久恢复预算与TOTP独立，候选高水位跨失败/换批保留。新秘密复用已锁HKDF/SHA256/随机/零化依赖，本片无新增业务依赖。
- 故障与确定性交错覆盖双实例同码、旧签发/ACK/换发迟到、坏payload/eventID/offset、双向pending互斥、合法吊销/GC保护、跨Cookie/来源/重启预算。真实Auth与Audit全部旧句柄关闭再重开；真实Audit双ACK落盘后在最终Auth事务内收紧临时文件权限，提交前检查失败，因子/epoch不变且码已消费。补记/重开不能激活，不声称真实commit I/O或断电实验。
- 独立审查发现沿用旧TOTP的外部坏历史回执分类会回滚新观察的clock_floor/expiry；两个管道均改AuditUnavailable，真实本地损坏仍CorruptStore。新增三项各覆盖L3与L2到期情形，拒绝计数/地板/到期持久，回退不复活；修复后完整双平台重跑通过。初稿行数超限已拆支持文件，不重复加载fixture。
- 另一契约复核收窄两项承诺：2076是恢复证明交付阶段上限，成功后另有最多256会话补记；原子回滚仅限提交前，提交后检查报错或响应丢失可能已切换/迁移，必须核对权威状态。逻辑行数有界不代表固定延迟、WAL/审计历史不增长。
- Linux沿用固定arm64镜像和Rust1.97.1，补齐rustfmt/Clippy，Cargo locked/offline；不是服务器x86_64原生验收。无前端/浏览器复跑，未新增HTTP/真实TTY/秘密部署/真实身份/Vault/Sync/NAS或恢复实操；未提交推送/fetch/合并部署，既有服务不变。
- T-016子计划同文件归档，写域释放；下一片可信本地登记/密钥装配先查重认领与设计，继续认证HTTP/维护、canonical CAS/同步/编辑/备份恢复/运维。完整M2与真实两周/干净机恢复门槛未通过，总M2计划不归档。

## T-017 验证记录（2026-09-05）

- macOS workspace475（472运行时＋3身份compile-fail）、Linux aarch64 workspace483（480＋3）全通过；Linux路径8项额外复跑已包含483内。两端fmt/严格Clippy、macOS release/Linux dev的local-server与auth-admin生产构建通过；CLI转发production并故意混入Auth test-support，均以指定错误拒绝。两端生产CLI另跑实际PTY/双库黑盒，不止默认测试构建。
- 新增54项：Auth no-create/状态13，Audit no-create8，CLI29单元+1实际产品进程，架构3。Auth190、Audit40、架构15；一项TTY契约内部驱动15情形，不重复加到总数。原1026轮会话/128次step-up/64次恢复码换发全量再过。
- codebase-design三案采用独立binary-only Module；配置16KiB、当前UID/600/700、无链接/目录身份不重叠；固定环境40位小写hex、自有副本零化，当前短命进程core软硬限额0。真正控制TTY有界隐藏输入、信号取消/恢复/默认行为复原；没有产品子进程、SQL复制、秘密文件或通用KMS Port。
- 实际进程覆盖无TTY拒绝、明确初始化、环境严格校验、首次登记与重复拒绝、stdout坏管道的“已提交但非零退出”。真实Audit权限故障在输入回调后触发：OTP可消费、Pending持久、恢复权限后补记不激活，再次登记不索取OTP；限流跨每次重开不重置。
- 首轮PTY定位macOS raw→canonical额外PENDIN，官方XNU与实测唯一位差一致；固定一次再flush/原样set/完整readback，最终不忽略回显位。HUP允许已恢复后的Cancelled或无法恢复的Restore，均不能输出OTP；无控制TTY夹具使用真实fork/setsid，避免TIOCNOTTY平台差异。全部修复后完整双平台再过。
- 新锁signal-hook0.4.4/registry1.4.8，复用rustix1.1.4 fs/termios/process与已有serde/zeroize等；固定Linux镜像/Rust1.97.1，先locked fetch再offline，Python3.11.2为容器既有测试依赖。没有服务器x86_64、真实UID变更/setuid、真实秘密/手机配对/HTTP/前端/浏览器/Vault/NAS/两周/干净机或commit I/O/断电验收。
- 所有相关文件≤300行（CLI最长201，Auth既有最长289）；脚本语法/diff检查通过。T-017计划同文件归档，原dirty分支和旧服务保留，未提交推送/fetch/合并部署；下一片认证HTTP/生产维护先重新查重、裁决矩阵与设计，不重做已完成T-012～T-017。

## T-018 验证记录（2026-09-05）

- 最终macOS workspace519（516运行时＋3身份compile-fail）、Linux aarch64 workspace527（524＋3）全部通过；额外Linux路径8项已含527内。两端fmt/严格Clippy、macOS release/Linux dev的local-server与auth-admin生产构建通过；两端local-server混用production/test-support均以指定编译错误拒绝，生产CLI各另跑实际进程黑盒通过。
- 新增44项：Auth已登记策略/配对/维护15、HTTP27、架构2；Auth205、Audit40、CLI30、架构17。原1026轮会话、128次step-up、64次恢复码换发再过。HTTP真实loopback测试覆盖实际流式超长与慢正文，不是HTTPS浏览器Cookie验收。
- codebase-design三案收敛封闭路由与单个HttpOnly Cookie，登记门槛集中在Auth每个签发事务及最终激活，不在HTTP先查状态再授权。新issue_enrolled先观察到期、执行一次原有界维护再签发；原issue/旧路由不隐式改变，无独立后台维护、新schema或第14个crate。
- GET通过Cookie内双秘密与持久摘要配对才回送CSRF；吊销另须显式恒时比较X-CSRF-Token。严格Origin/Fetch Metadata/固定自定义头/JSON/实际正文和头部预算、总2秒/1KiB/2048帧边界；重复Cookie/安全头、跨配对、无因子、过期/吊销、Auth与Audit双库故障均有拒绝契约。
- 两个共享blocking槽的许可持有到实际工作结束；真实取消两个HTTP任务后第三请求仍503，解除审计阻塞后恢复，已提交孤儿会话可存在。不承诺取消回滚或慢审计不占读取槽；503不清可能有效Cookie，提示先核对状态。HTTP测试仅公开组合接口/临时双SQLite；部分Auth内部破坏证明夹具用明确SQL故障注入。
- 初次编译修正测试误用登记句柄、缺少共享种子helper及Request跨await借用非Sync；handler改拥有Request，Clippy分支修正。完整Linux首轮暴露旧PTY hangup返回Err(Restore)后被迟到SIGHUP终止，测试误只收退出0；仅在已观测取消/恢复错误、无秘密输出后接受明确信号1，未放宽产品恢复检查。新增读后SIGHUP默认行为，单TTY契约现16情形；Linux专项五次复跑及修复后完整双平台通过，额外复跑不累计测试数。
- 仅为local-server增加既有锁定版本的subtle/zeroize正常依赖边，测试复用reqwest/Audit/hmac/sha1并显式启用tokio io-util；没有新包或业务版本升级。Linux沿既有精确arm64镜像与Rust1.97.1，rustup补Clippy后Cargo locked/offline，不是服务器x86_64验证。
- 所有相关源码≤300行：新HTTP源码最长154、HTTP夹具203、Auth既有最长289；脚本语法/diff检查通过。T-018计划同文件归档，写域释放；未改主binary配置/前端/部署，无真实身份/手机/HTTPS浏览器/Vault/NAS/两周/干净机或commit I/O/断电验收。原dirty分支与旧服务保留，未提交推送/fetch/合并部署。

## T-019 验证记录（2026-09-05）

- 09:45 PDT用户明确继续，交互开发T-019；夜间心跳保持PAUSED，不恢复旧截止或新建定时任务。
- macOS workspace575（571运行时＋4compile-fail）、Linux aarch64 workspace583（579＋4）全部通过；Linux路径8项另跑已含583内。两端fmt/严格Clippy、macOS release/Linux dev的服务器与CLI生产构建通过；两端production混入test-support均以指定错误拒绝。
- 新增56项：Auth就绪13、共享配置11＋1compile-fail、参数3、组合7、实际启动进程20、Broker根身份1。原CLI12配置测试迁移而非复制，现CLI18、host-config23运行时＋1compile-fail、Auth218、Audit40、架构17。原1026轮会话/128次step-up/64次恢复码换发再次通过。
- 两平台另用实际生产服务器重跑同20项启动黑盒，且生产CLI重跑真实进程契约；不重复计入workspace总数。新模式成功路径经私有取钥替身与真实签名JWT/配置/双库/路由验证，未冒称真实Cloudflare或HTTPS浏览器成功。旧本机模式有实际loopback GET/HEAD/405/404回归。
- 三案比较选择操作者共用owner.json，新增第14个workspace crate host-config，安全文件知识与领域解析只有一处；管理员与服务器均用封闭Interface。不引入新业务依赖版本或schema。HTTP不持配置/SQL/seed/登记句柄；启动层只no-create、校验当前登记/恢复双ACK与固定根，真实取钥和初扫完成后才监听，不降级。
- 独立审查发现非UTF8 argv可能触发默认panic回显；main改args_os有界解析并固定脱敏，真实进程负测已通过。根复核补充缺库/主人TTL/错Audit先于取钥的内部断言；Auth与配置交叉复审均无遗留阻断。
- macOS首轮沙箱阻止既有Unix socket夹具；获准正常升级后全量重跑，没有跳过测试。相关文件≤300行（本片新增/迁移最长216，Auth既有最长289）；no-create不冒称零写，启动快照不穷举历史ACK、不抗管理员一致克隆。
- 本片契约与权威矩阵/框架/INDEX/TASKS/日志已同步，计划同文件归档、写域释放。仍在原dirty分支/HEAD0bd066d，未提交推送/fetch/合并部署；本轮无前端/浏览器、服务器x86_64原生、真实身份/秘密/Vault/NAS/恢复演练或两周试跑验收。
- 下一片先认领前端会话引导、登录/退出与会话管理，保留Apple设计/图标/可中断动效，再因子HTTP与canonical revision。前端未接线前不要把新模式替换线上只读入口；完整M2剩余门槛见发布缺口。
- 完整M2剩余代码与外部门槛见[发布缺口](release-readiness.md)。本片完成不能代替全部认证产品接线、canonical CAS/新鲜度/同步/编辑/备份恢复/运维，更不等于M2已发布。

## T-021～T-023 验证记录（2026-09-05）

- macOS workspace **613**（609运行时＋4compile-fail）、Linux aarch64 **623**（619＋4）全部通过；Linux路径8项另跑已包含623，不重复累计。两端fmt/严格Clippy、macOS release/Linux dev服务器及CLI生产构建通过；两端生产/test-support混用均以指定错误拒绝，生产服务器另跑20项启动黑盒，CLI另跑真实PTY产品进程。
- 前端20文件**84项**全通过，lint/Prettier/TypeScript/生产构建通过；JS gzip119.09kB、CSS gzip8.12kB。真实Chromium生产bundle使用虚构HTTP会话夹具验证8类操作，含真实双标签BroadcastChannel、503不重放POST、焦点返回及390px窄屏；人工查看桌面/移动截图，无JS异常。不是HTTPS Cookie或真实Access登录验收。
- 新增38项macOS/40项Linux Rust：Auth运行因子6、Broker17＋Linux2、bootstrap5、canonical HTTP5、启动3、参数1、架构1。前端新增24项；既有长期1026轮会话、128次step-up、64次恢复码换发均随全量再次通过，不重复算作测试项或实际多日运行。
- [前端会话契约](../contracts/frontend-session.md)：严格固定bootstrap、显式登录和当前/指定/全部退出；CSRF仅内存、迟到响应拒绝、401卸载内容、POST不确定只显式核对。前端storage回退测试的环境分支已修正；真实浏览器另验BroadcastChannel分支。
- [canonical读取契约](../contracts/canonical-read.md)：已有身份no-create、同根窄能力、1MiB稳定有界读取和真实revision；每读及监听前复核根/marker，旧M1 generation仍为原快照。独立审查发现大写.MD兼容和prepare后marker变化两处缺口，均修复并回归。共享安全读取内核不授写能力。
- [固定槽因子契约](../contracts/auth-factor-runtime.md)：逐次权威选择、恢复后重开/跨实例跟随、旧槽只收紧不回退；继续原TOTP/Recovery状态机，无新schema或秘密复制。独立交叉审查无阻断发现，不声称抵御管理员一致整库回滚。
- 首轮跨Unix FIFO测试误用Linux专用API，已沿既有系统mkfifo夹具修正；macOS沙箱阻止既有Unix socket测试后获准正常权限全量复跑。旧架构断言按新组合参数更新，并新增窄读能力约束；无跳过失败用例。所有新增源码≤300行，未顺带重构既有M0大型Broker文件。
- 沿用固定Rust1.97.1及既有精确Linux arm64镜像，Cargo locked/offline；无业务依赖版本升级。没有服务器x86_64、真实身份/手机/Sync Vault/NAS/恢复/两周运行、commit I/O或断电验收；原dirty分支与HEAD0bd066d保留，未提交推送、合并或部署。
- 三个子计划同文件归档，写域释放；用户“先把M2完结了”的总目标持续。下一项T-024已认领三条因子HTTP、真实Unix来源与固定秘密启动装配，随后因子界面、canonical单写者/新鲜度/CAS、同步/编辑/备份恢复/运维；总M2计划未归档，旧心跳仍PAUSED。

## T-024 验证记录（2026-09-05）

- macOS workspace **667**（663运行时＋4compile-fail）、Linux aarch64 **677**（673＋4）全通过，无失败或忽略；额外Linux路径8项已含677。两端fmt/严格Clippy、macOS release/Linux dev服务器和CLI生产构建通过；两端生产服务器另跑23项启动黑盒、CLI真实PTY流程及production/test-support指定编译错误负测。
- 新增54项：Auth共用hex解码3、Unix来源/路径/生命周期21、因子HTTP14、启动装配6、环境/进程保护3、参数2、实际启动进程3、架构2。Auth227运行时；前端84和浏览器8类是T-021历史结果，本片无前端代码变化，不冒称重新验证。
- [因子HTTP与Unix契约](../contracts/auth-factor-http.md)：整个显式因子模式使用Unix，真实同UID连接证明、逐请求严格IP；三条动作复用固定槽Auth、严格浏览器门禁与两槽预算。自有秘密立即零化封装并移动到工作任务，恢复码Body保留零化owner；503或响应丢失不重放，恢复成功撤销全部旧会话。
- 真实Unix/双SQLite验证逐连接多IP、TCP伪头拒绝、完整换码/恢复、未读恢复响应后重开、持久限流、Audit权限故障/pending及执行中撤权。路径测试覆盖已有文件/link/FIFO/socket拒绝、根/父目录替换、权限及仅清本实例inode；不同UID仅私有决策负测，不是跨系统UID实操。
- 首轮测试误调用私有seed getter、旧架构依赖归属断言及Clippy两处写法已修；未放开公共秘密读取或通用文件写权限。两路独立只读交叉复审无阻断发现。Linux初次默认并发链接被signal9终止；自有容器当时约6.4/7.7GiB，多链接并发，退出后以CARGO_BUILD_JOBS=2完整重跑通过，无删测试或忽略。
- 仅增加已有rustix正常依赖边及Tokio signal到已有registry的依赖边，没有新包、业务版本或schema变更。Unix适配器精确限制父FD操作；进程core限额0、固定环境双槽与候选配对仅显式模式启用。全部相关源码≤300行，fmt/diff检查通过。
- 两端实际生产成功组合仍受私有测试取钥Seam限制；不声称真实Cloudflare/HTTPS Cookie/手机/秘密部署、服务器x86_64、Sync/NAS/恢复/两周或断电通过。保持原dirty分支和HEAD0bd066d，未提交推送、合并或部署，旧服务不变。
- T-024计划同文件归档并释放写域。T-025已先登记状态GET矩阵与计划，继续非授权状态投影/因子前端；随后单写者需先复现并修正旧Broker未决同内容误判，见[设计输入](canonical-write-design.md)。完整M2与外部门槛继续。

## T-025 因子状态与账户界面（2026-09-05）

- macOS workspace **683**（679运行时＋4compile-fail）、Linux aarch64 **693**（689＋4）全部通过，无失败或忽略；Linux路径8项额外复跑已包含693。两端fmt/严格Clippy、macOS release/Linux dev服务器和CLI生产构建、生产服务器23项启动黑盒、CLI真实PTY与production/test-support指定错误负测通过。
- Rust新增16项：Auth投影7、状态HTTP9。单事务观察、期限共享、会话隔离、未知绑定、跨实例恢复、pending不消费/补记、无IP GET、旧TCP明确null与故障不降级均通过；原长期会话/step-up/恢复码契约继续回归。
- 前端26文件**125项**全通过，新增41；lint/Prettier/TypeScript/生产构建通过，JS gzip122.56kB、CSS gzip8.35kB。实际生产bundle浏览器验证8类、41次API/7次明确POST，无未捕获异常；桌面/390px触控/减少动效、一次性交付、迟到码拒绝、503不重放、204与真实双标签广播通过，截图已人工查看，详见[记录](factor-browser-qa.md)。
- SessionClient仍唯一拥有CSRF/代次/POST互斥，内部按状态、发现与协议拆分；状态不是授权或请求完成证明。GET不消耗OTP/预算或清pending，动作执行时重验；JS只清自有引用和DOM，不承诺浏览器/剪贴板零化。
- 首轮HTTP夹具错误地假设open_existing不观察时钟，已按真实初始化/预检/执行三阶段修正，保留401清Cookie和403保留Cookie断言；9项及全量重跑通过，未修改生产逻辑来迎合夹具。Auth、HTTP和前端交叉复审无阻断。全部相关源码≤300行，无依赖或schema变更。
- 仅虚构身份/双SQLite/本机HTTP浏览器；不冒称真实Access、HTTPS Cookie、手机、秘密、服务器x86_64、Sync/NAS、干净机恢复、两周或断电验收。分支仍HEAD0bd066d/dirty，未提交推送、合并部署或改旧服务。
- T-025计划同文件归档，继续canonical写域；T-027复审分支尚未整合，其Gateway等待后撤权、审计词法兼容及schema零写入表述须先核实，见[整合前核查](m0-review-integration.md)。

## T-028 canonical单写者与未决幂等（2026-09-05）

- macOS workspace **724**（720运行时＋4compile-fail）、Linux aarch64 **735**（731＋4）全通过，无失败或忽略；相对T-025新增41/42项。Broker为94/103项，包含8项私有阶段、30项writer（含child入口）、3项错误CAS回归；Linux另新增带外create竞争。路径安全8项额外复跑已含在735内。
- 两端fmt/严格Clippy、release生产服务与CLI、真实PTY CLI、生产服务器23项启动及指定production/test-support负测通过；Linux固定原镜像、locked/offline、2构建任务。web无变更，沿用T-025的125项/8类浏览器证据，不冒称本片重新跑过。
- 根目录FD跨进程独占租约与永久root/data配对；显式open_existing不创建/修复布局。固定Prepared/Installing/Completed阶段保留明确拒绝和未知安装，Completed只补终态与提交水位；所有artifact/manifest安装共用水位，L0保存实际旧revision原字节。
- 旧Fs错误CAS三步在同进程/重开均真实复现，修复后回归通过。独立复核修正初读marker/绑定pin、初始化复核、大小写别名嵌套、记录多段nofollow和回放尾部绑定检查，无新阻断。首轮测试摘要编码、marker旧错误语义和架构旧函数名断言均已修正后全回归。
- 新源码模块≤207行，无依赖变化。legacy首次采用4096条/16MiB预算不成为新回执累计上限；内存mutation4MiB、孤立stage64项保持显式限制，流式附件、stage清理和Installing协调仍后续。详细[契约](../contracts/canonical-writer.md)不把水位当索引新鲜度。
- 原失败/OS探针/产品日志均在/private/tmp/studiolo-m2-t028-*；探针不加入Rust计数，故障点不等于真实断电。未操作真实Vault、Sync、NAS、手机或恢复环境；未提交推送、合并开发分支或部署。
- T-028计划同文件归档并释放写域。用户确认main已d24e56a且CI通过，当前开发HEAD仍0bd066d；下一项先认领并合新main，保留审计拆分、移植payload词法/契约，补Gateway等待后撤权与schema拒绝副作用，再继续M2。

## T-029 main整合与增量修正（2026-09-05）

- 实际fresh fetch与ff-only/autostash合入main，HEAD已d24e56a；审计冲突保留拆分、词法尾部逐字移入payload.rs，契约与.gitignore双方内容保留。591文件保全核对无丢失/意外变化，备份与autostash保留；未新提交推送/CI或部署。
- macOS **747**（743运行时＋4compile-fail）/Linux aarch64 **758**（754＋4）全工作区通过；两端fmt/严格Clippy、release生产服务器/CLI、真实PTY CLI、23项生产启动与指定production/test-support负测通过。额外Linux路径8项不重复累计。
- 相对T028新增23/23项，含main带入4项及T029审计3、ControlDb7、Gateway9项。Audit43/ControlDb20/Gateway25项；Gateway先确定等待复现7失败，ControlDb先实跑复现新版DELETE被改WAL，修正后全过。
- 等待返回的三种准入在槽锁外完整重验权限，仅Proceed拒绝释放自有占位；拒绝审计冲突不再误写槽冲突。schema检查先于持久PRAGMA/DDL且事务内再核；证明范围限干净关闭库的主文件、逻辑内容和journal配置。新禁apiKey历史T012 v1记录/回执拒读且不推进head，允许seed_ref精确重放；不自动迁移禁字段。
- 根修正SQLite测试类型、瞬时锁断言及私有夹具目录；生产文件系统权限扫描未放宽。独立复审无遗留阻断，新增源码最大251行、无新增业务依赖。Linux沿既有固定arm64镜像/locked/offline/jobs=2；web无变更，沿用T025证据，不算本片重跑。
- 证据为/private/tmp/studiolo-m2-t029-*日志；[main整合记录](m0-review-integration.md)保留原复现与兼容边界。计划同文件归档，继续新鲜度/CAS/同步/编辑/备份恢复与运维。真实身份/HTTPS/Sync/NAS/干净机/两周/断电未由夹具替代，完整M2未发布。

## 外部验收条件（仍未通过）

待用户指定允许接入的 Sync Vault/副本、DS920+ 地址及专用备份目标；认证秘密不进入
仓库、同步 Vault、聊天或诊断包。实际身份登记、NAS 追加负测、干净机恢复和两周
headless 试跑均单独验收，未完成时明确显示未就绪。第二故障域已决定不配置，
对应同址灾难和 NAS/管理员失陷风险仍不覆盖。

只有明确版本通过发布门槛后才合并 main 并切换既有 Studiolo 服务；
不可变发布目录、校验、备份与回滚证据不能由“构建成功”代替。
