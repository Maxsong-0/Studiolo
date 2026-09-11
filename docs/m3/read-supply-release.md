# T-040 HTTP、客户端投影与索引发布

2026-09-10，用户明确要求先合并部署。候选包含 T-037 HTTP 文本入口、T-038 客户端投影 CLI、
T-039 持久派生索引。本文件记录发布流程，实际合并、原生验收和系统切换状态分别登记。

## 发布边界

现有 T-036 系统服务以 maxsong（UID/GID 1000）运行，回环端口 4400；本次只升级该服务。
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

`deploy/agent/upgrade-read-supply.sh.in` 是专用 T-036→T-040 系统升级模板，
填入发布目录、新二进制、单元及源码清单 SHA-256 后生成一次性安装器；执行探针前重新核对源文件集合及摘要。
它固定核对已验收 T-036 旧二进制与旧系统单元摘要，拒绝其他版本、drop-in 或第三方替换。
旧 `install-t036-system.sh` 仅用于历史用户服务迁移，不能用于这次系统升级。

1. 检查旧服务 enabled/active、实际 PID/二进制/UID 映射，保存 root 私有旧单元副本。
2. 使用系统管理器在 4401 运行新程序，保持正式隔离和缓存目录；MCP 与 HTTP 分别验收。
3. 重启阶段单元，要求启动日志 `source: Disk, parsed: 0, persistence: Ready`；再次检查完整供给。
4. 停止阶段单元，再次核对旧系统单元绑定，原子替换正式单元、daemon-reload、restart 4400。
5. 验证新进程、四工具、四 HTTP 入口、14 个 HTTP 拒绝场景、ready 回执和缓存命中。
   中断或失败时，仅在正式单元仍是本次写入对象的情况下恢复备份的 T-036 系统单元并重启旧程序。

安装器保留 root 私有备份目录并输出其路径，不触碰已禁用的用户级 MCP 单元。
若机器掉电或进程被 SIGKILL，退出守卫不能执行；须据备份和实际 systemd 状态人工恢复，
不以残留锁作为已经成功或回滚的证据。

`check-agent-http.py` 关闭环境代理和重定向，只输出回执摘要；错误正文和秘密不输出。
HTTP/MCP 新鲜度探针只有“有效回执在连续读取间切换”返回 2，安装器对此最多重试三次，
其余身份、完整性、陈旧、协议和服务失败直接终止，不能靠重试掩盖。

## 当前状态

准备时已读查线上 T-036 active/running、PID 40172、NRestarts 0，网页与 Tunnel PID 未变。
本地运行包门槛 13 项、HTTP 探针 9 项、副本探针 8 项通过；新 HTTP 探针已在真实隔离服务上通过。
T-039 相关两端各 172 项、严格 Clippy、release 与投影管线验收见 [持久派生索引](persistent-index.md)。
合并、服务器原生验收和正式系统切换尚未完成；服务器 sudo 需要用户在终端完成认证。
