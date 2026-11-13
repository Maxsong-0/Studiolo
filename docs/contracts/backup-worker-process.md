# 独立备份进程执行边界

`studiolo-backup-worker` 是独立的 Linux 包，不能进入 App/owner 的依赖闭包。当前子单元实现固定包执行与追加 runner；服务入口、受保护配置/凭据文件读取、作业主循环、审计 IPC、远端最终清单和完整恢复仍待后续接线，不能据此宣称生产 Rust 备份链完成。

## 固定包与身份

生产只打开 `/usr/lib/studiolo-backup-worker/restic-0.19.1`。逐级 nofollow，目录必须 root 所有且没有组/其他写权；二进制必须 root 所有、单链接普通 ELF 文件、受限执行权限、最多128 MiB。SHA-256 必须匹配受保护部署清单提供的摘要，不能信二进制旁边自报的摘要。

持有原文件 FD，经 `/proc/self/fd/<原FD>` 执行；没有 PATH 搜索或 shell。每次调用前后检查原 inode、所有者、权限、大小及修改时间。先运行固定 `version` 核对0.19.1，单次版本探测最多10秒。测试以当前测试UID持有的发行副本验证执行机制，不代表已经验收生产 root 安装目录。

`AppendRunner` 只接受配置绑定的 HTTPS REST 仓库和 Append 身份；SFTP/S3/本地路径、URL凭据、查询、fragment及非规范URL拒绝。它不接受额外argv或可变环境键。恢复与保留清理身份不得借此 runner 运行。内核对真实仓库的只追加能力及远端不可变配置仍须单独验证。

## 环境、输出与截止时间

子进程使用 `env_clear`，仅设置固定 TMPDIR 和 `RESTIC_PASSWORD_FILE`、`RESTIC_REST_USERNAME`、`RESTIC_REST_PASSWORD` 三槽。上层必须先通过共同的 process-safety 加固并记录结果，再从 worker 受保护凭据源构造 `ResticCredentials`；该类型不实现 Debug/Clone，自有字符串零化。启动后立即释放 Command 的环境副本，不把正文/凭据/原始stderr放入错误或日志。

生产临时根固定 `/var/lib/studiolo-backup-worker/tmp`，必须事先创建为当前非root worker的0700目录，逐层拒绝链接和非可信所有者/写权；调用前后重核原根身份。初始化/回收由后续服务装配完成，本模块不擅自创建宿主机目录。

stdout最多8 MiB、stderr最多64 KiB，预分配有界缓冲并在释放时零化，两根管道以非阻塞方式交替读取，不能堵住一侧管道。一个作业共用原单调时钟截止时间（最多2小时），不为每次上传/校验重置预算。输出或截止越界立即进入回收，退出0和自报文件数只作中间证据，不能替代远端校验。

## 原进程组回收

每次执行创建独占进程组。`waitid(WEXITED|WNOHANG|WNOWAIT)` 保留组长身份；先对本次原进程组发SIGKILL，再reap组长，避免PID复用后误杀别的进程。组长正常退出时同样清理继承管道的同组子孙。无法证明原child身份时不再向那个整数PID发信号。

终止后的等待最多2秒。无法证实回收则返回 `CleanupFailed`，runner永久封闭，外层必须停止服务并保留作业未知状态；不能换一个runner悄悄继续。服务最终部署仍需受控cgroup和KillMode验收。进程组测试只证明同组子孙回收，不冒充跨组/cgroup隔离。Rust父进程的内存/可转储加固不能假定跨第三方程序exec继续生效；独立UID、子进程核心转储与swap约束须在最终服务部署中实际验收。

## 开发验证

108 `20261113-155333-4b6c`：真实进程定向4项通过，6个ignored子入口由父测试实际执行；官方发行CLI单项另外用ignored入口实际执行。覆盖继承环境清除、stdout/stderr越界、超时、组长退出遗留子孙、原包摘要/权限变化拒绝、仓库URL拒绝。核验官方amd64发行压缩包SHA-256后，真实restic从原FD完成临时仓库init、重复init拒绝、backup和check --read-data，实际JSON解析通过。严格Clippy及66项架构检查通过。

所有仓库、凭据和正文均为虚构测试数据；未使用生产仓库、未修改系统账号或部署服务。生产只读封存引用核验、实际私有凭据源、审计代理、远端最终提交和完整新根恢复仍须继续。
