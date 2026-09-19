# T-040 HTTP、客户端投影与索引发布

2026-09-10，用户明确要求先合并部署。T-037 HTTP 文本入口、T-038 客户端投影 CLI、
T-039 持久派生索引已于 2026-09-11 随 T-040 发布，系统切换与公网复验通过，证据见末节。

## 发布边界

本次将 T-036 系统服务升级为 T-040，继续以 maxsong（UID/GID 1000）运行、使用回环端口 4400。
既有身份库、读取审计、托管副本和 Mac 每 60 秒的两个白名单自动同步沿用。
客户端投影 CLI 与完整参考文档随源码包发布，不自动写入真实 Claude/Codex 工作目录。
网页 4318/4319、Tunnel 和凭据不变；图 2 与完整 M3 验收仍属后续。

新系统单元增加 `--index-cache /home/maxsong/.local/share/studiolo-agent-index`，
该目录由 maxsong 预建为 0700，与 data/replica 平级；ReadWritePaths 仅增加这一个缓存根。
资料和发布目录保持只读，其余 systemd 身份、隔离、资源限制沿用。

## 运行包与原生验收

候选名为 `m3-t040-20260910-read01`。从干净、已提交源码调用 `scripts/package-agent.sh`，
固定 amd64 镜像 digest，以独立构建卷生成二进制与 Rust 测试；`base-commit.txt` 和 `provenance.txt`
记录来源。候选可在 CI 期间构建；只有同一提交已纳入通过检查的 main 后才能切换正式服务。

打包仅复制显式白名单中的 Git 已跟踪普通文件，拒绝链接和 Python 缓存。
`source-files` 与 `SOURCE_SHA256SUMS` 绑定精确源码文件集合及逐文件字节；
外层 `SHA256SUMS` 覆盖二进制、测试程序、源码压缩包、系统单元及内部清单。
除原有脚本外，白名单包含所有客户端投影模块、测试、两个真实管线和新 HTTP 探针。

随包 Rust 构建须具备按 crate/target 指定的 17 个关键目标，不能用其他 crate 的同名目标充数。
服务器以普通用户从记录的 `source` 目录运行 `bash ../run-agent-tests.sh`，先验证解压源码集合和摘要，
再运行所有独立 Rust 程序、10 组 Python 契约，以及两条虚构资料真实 MCP/HTTP 管线。
此步骤不操作线上资料或凭据，不需要服务器 Rust、Node 或 Docker。

## 系统服务升级与回滚

`deploy/agent/upgrade-read-supply.sh.in` 是系统升级模板。它最初是 T-036→T-040 专用、把旧基线写死在模板里；
2026-09-18（T-076）起旧基线改为占位符：`scripts/render-agent-upgrade.py <payload> --old-release … --old-binary-sha … --old-unit-sha … --output …`
从运行包算出新发布的四个摘要、填入线上**实际在跑的**那个发布的名字与两个摘要，生成一次性安装器；模板必须与运行包内的同名文件摘要一致，否则拒绝渲染。
旧基线的两个摘要在服务器上只读取得：`sha256sum <旧发布>/bin/studiolo-agent /etc/systemd/system/studiolo-agent.service`。
安装器执行探针前重新核对源文件集合及摘要；它固定核对所绑定的旧二进制与旧系统单元摘要，拒绝其他版本、drop-in 或第三方替换。
**它一次跑完阶段验收与生产切换**（4401 阶段通过后原子替换单元并重启 4400，失败自动回滚），不是只做阶段验收。
旧 `install-t036-system.sh` 仅用于历史用户服务迁移，不能用于这次系统升级。

0. **审计账本 v1 → v2**（T-051 起磁盘格式为 v2，旧发布写的是 v1，新二进制打不开 v1）：先校验账本恰好是 v1；
   阶段验收跑在生产数据的**副本**上（副本里先用新二进制 `admin audit-upgrade` 升一次，等于带真实数据的彩排，线上零风险）；
   切换时进停机窗口：停 4400 → 备份账本 → 就地升级 → 换单元 → 启动 → 验收。回滚顺序固定为
   **账本还原 → 单元还原 → 起回旧服务**（旧二进制打不开 v2），期间写入的 v2 账本改名留档不丢弃。
1. 检查旧服务 enabled/active、实际 PID/二进制/UID 映射，保存 root 私有旧单元副本。
2. 使用系统管理器在 4401 运行新程序，保持正式隔离和缓存目录；MCP 与 HTTP 分别验收。
3. 重启阶段单元，要求启动日志 `source: Disk, parsed: 0, persistence: Ready`；再次检查完整供给。
4. 停止阶段单元，再次核对旧系统单元绑定，原子替换正式单元、daemon-reload、restart 4400。
5. 验证新进程、四工具、四 HTTP 入口、14 个 HTTP 拒绝场景、ready 回执和缓存命中。
   中断或失败时，仅在正式单元仍是本次写入对象的情况下恢复备份的 T-036 系统单元并重启旧程序。

安装器保留 root 私有备份目录并输出其路径，不触碰已禁用的用户级 MCP 单元。
若机器掉电或进程被 SIGKILL，退出守卫不能执行；须据备份和实际 systemd 状态人工恢复，
不以残留锁作为已经成功或回滚的证据。

本次成功后的备份目录为 `/etc/systemd/system/.studiolo-t040.nFeRsmmQ`。
后续回滚应核对并恢复该备份中的 T-036 **系统单元**及旧二进制，不恢复历史用户单元。
一次性升级脚本只接受 T-036 旧基线，成功切换后不可重复运行。

`check-agent-http.py` 关闭环境代理和重定向，只输出回执摘要；错误正文和秘密不输出。
HTTP/MCP 新鲜度探针只有“有效回执在连续读取间切换”返回 2，安装器对此最多重试三次，
其余身份、完整性、陈旧、协议和服务失败直接终止，不能靠重试掩盖。

## 当前状态（2026-09-11）

[PR #3](https://github.com/Maxsong-0/studiolo/pull/3) 已合 main `cbfb5df8e7dd6357b5e9b1a79785e4a608460644`。
分支、PR 和 [main CI 34609104890](https://github.com/Maxsong-0/studiolo/actions/runs/34609104890) 均通过。
运行包 `m3-t040-20260910-read01` 来自候选 `cec434cd8fcb7c0af36eabc894ff866b4cad6743` 的干净源码，
75 项发布文件摘要、565 项源码清单/字节/归档与候选提交完全一致；合并后文档更新不改写包的来源提交。

服务器 Linux x86_64 原生执行 59 个 Rust 测试程序共 362 项、10 组 Python 契约共 117 项，
以及两条使用虚构资料的真实 MCP/HTTP/客户端投影管线，全部通过。
本地运行包门槛 13 项、HTTP 探针 9 项、副本探针 8 项通过；
T-039 开发阶段两端各 172 项与严格检查见 [持久派生索引](persistent-index.md)。

切换前直接进程读取真实副本的预验为冷启动解析 99、重启解析 0/复用 99；
此后用户在终端完成 sudo 安装，系统管理器 4401、阶段重启缓存及正式 4400 的 MCP/HTTP、
拒绝与新鲜度验收均通过。直接进程预验和本次 systemd 隔离验收分别记录。

切换后独立直连复核：正式 PID 53994，enabled/active/running、NRestarts=0，
UID/GID 四列均为 1000、UID 映射恒等；`/proc/<pid>/exe` 指向本发布二进制并匹配下列摘要。
正式启动日志为 `source: Disk, parsed: 0, reused: 100, persistence: Ready`；
缓存目录 0700、`index.sqlite3` 0600，4401 已空闲，既有网页和 Tunnel PID 与基线一致。

| 验证对象 | SHA-256 |
|---|---|
| 正式运行二进制 | `86e8110f8d4440000f5845d9774adcdcf96911c1da93e4ee00729c66323bd58e` |
| 正式系统单元 | `a2df827337f6862f491fa65b0316d77a1b7ed455cee818f3aa4b9c150a946ffc` |

2026-09-11 14:35 UTC，从 Mac 经实际 HTTPS 公网入口复验：MCP 初始化与四工具完整链、
匿名/错误凭据 401、五处副本回执，以及 HTTP 四入口成功和 14 个拒绝场景，五组均首轮通过。
MCP 与 HTTP 回执同批、ready、stale=false，观测年龄 17–20 秒、阈值 300 秒；年龄仅表示验收时点。
本次记录位于 `/private/tmp/studiolo-t040-release/evidence.json` 和 `public-checks.json`。

HTTP 文本供给与持久派生索引已上线。客户端投影 CLI/参考文档已随源码包发布，
真实 Claude/Codex 资料工作区尚未安装，真实客户端启动验收未完成。
完整 M3、图 2 项目运作图与目录结构漂移仍属后续。
