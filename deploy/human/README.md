# T-092 人侧 L3 服务准备材料

**最新状态（19:19 UTC）：T-092人侧上线验收完成：真实v2、最终10枚恢复码、发布v2/回滚v3/撤销v4及配对审计已核验；测试项已移出现行策略，Claude v1及原effective摘要恢复。r7服务已启用；源码/部署材料仍未提交推送。**

状态：2026-09-20 19:12 UTC，真实v2已生效，新因子L3与最终10枚恢复码已核验，主人已确认保存；r7策略页并发读取修复已部署并通过公网加载/重新校验刷新；等待主人零权限测试项发布，再验收回滚。当前操作以 `current-v2/README.md` 与 `ACCEPTANCE.md` 最新回执为准；下文r2装配和首次配对保留为历史说明，不得按旧步骤重新登记或启动r2。

`launch-human.py` 是唯一启动器，使用 Python 3 标准库和 Linux libc；没有 `.sh` 版本，不用 Bash 执行。unit 显式使用 `/usr/bin/python3 -I -B -S`，隔离 Python 路径/环境、禁写字节码并禁载 site。启动器只允许固定产品与参数，不接受 CLI 覆盖。

## 固定装配

| 项目 | 位置 |
|---|---|
| release / WorkingDirectory | `/home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r2` |
| 启动器 / 产品 | release 下 `launch-human.py` / `bin/studiolo-local-server` |
| 只读示例 / web | release 下 `example-vault` / `web` |
| owner 非秘密配置 | `/home/maxsong/.config/studiolo-human/owner.json` |
| Auth | `/home/maxsong/.local/share/studiolo-human-auth/auth.sqlite3` |
| 人侧 Audit | `/home/maxsong/.local/share/studiolo-human-audit/audit.sqlite3` |
| 共用正式 Policy | `/home/maxsong/.local/share/studiolo-agent-policy/policy.sqlite3` |
| socket | `/run/studiolo-human/http.sock` |
| 加密凭据源 / ID | `/etc/credstore.encrypted/studiolo-human-totp` / `totp-seed` |
| 公开 Origin | `https://studiolo.maxsong.me` |

这是系统服务，固定 `User=maxsong` / `Group=maxsong`；不套用用户单元的 `%h`。owner.json 的 auth_directory/audit_directory 必须分别等于表内目录；owner 的 issuer/audience/subject/email、TTL 和 seed_ref 由已批准配置固定。不得凭邮箱猜 subject。

示例库是新 release 的固定根，**不把 replica 的代次目录当作稳定根**。启动参数不含 TCP port、旧 `--access-*` 或 canonical writer；4400/4318/4319 可继续运行，切 Tunnel 是后续独立动作。

## 官方 credential 语义与交接接口

systemd 在激活服务时解密并认证 `LoadCredentialEncrypted` 的内容，再以只读运行时文件交付；服务通过 `CREDENTIALS_DIRECTORY` 找到它，程序无需直接读取 `/etc/credstore.encrypted`。凭据在服务激活期间固定，停止后释放；`ProtectSystem=strict` 已启用挂载命名空间。来源：[systemd Credentials](https://systemd.io/CREDENTIALS/)。

密文必须为**系统服务**创建，不加 `--user`；ID 必须显式绑定 `totp-seed`，因为默认 ID 来自输出文件名，和这里的密文文件名不同。主线配对工具使用以下命令形状，将恰好40字节的小写 hex 通过私有管道送入 stdin（无换行），不得在终端、argv、聊天或普通文件中拼接种子：

```text
systemd-creds encrypt --name=totp-seed - /etc/credstore.encrypted/studiolo-human-totp
```

上面只是主线私有流程的调用接口，本材料不生成秘密或执行加密。官方依据：[systemd.exec 上游文档](https://raw.githubusercontent.com/systemd/systemd/main/man/systemd.exec.xml)、[systemd-creds 上游文档](https://raw.githubusercontent.com/systemd/systemd/main/man/systemd-creds.xml)。密码学绑定可能依赖本机 TPM/host key，因此复制密文不等于异机可恢复；异机恢复须另验。

wrapper 从 `CREDENTIALS_DIRECTORY/totp-seed` 逐级使用目录 FD/no-follow 打开，要求无符号链接、可信 owner、不可组写/他人写；运行时目录为0550、文件为0440且root:root，严格核验Linux ACL v2：只允许root和本服务UID读取（目录可遍历），group/other无权限、无额外ACL项；文件单链接且恰好40字节。只读取最多41字节并严格检查小写 hex，不剥除换行/NUL/空白，不读取 `.env`，缺失/畸形即固定错误退出。所有临时 FD 都在 exec 前关闭。

读取前确认固定 UID/GID、关闭 core 并设置/回读不可转储。然后只向固定产品的 `STUDIOLO_TOTP_SEED_HEX` 环境槽交付；其余环境只含固定 PATH/LANG。不输出种子、内容摘要或底层异常，不启动子进程。

边界：Python 的 bytes/str/env 副本不声称可零化，也未由 wrapper 锁页；exec 更换映像时的 dumpable/锁页状态仍须产品立即重新 harden。现有产品仅支持种子环境槽，因此这份桥接不宣称消除所有环境驻留窗口；同 UID 管理员/特权管理员也不在隔离承诺内。

## 安装前与上线门禁

1. 发布物先统一验包、核生产特征、记录哈希。产品必须归 maxsong 且0755；启动器及 release 不允许组写/他人写。本材料不重复107上的构建/测试。
2. owner.json 为0600、单链接、maxsong所有；配置父目录及 Auth/Audit/Policy 为0700，祖先无符号链接/不可信写权限。Auth v4 必须完成真实配对与双审计就绪；三个库必须已存在。wrapper只查元数据，产品再检查实际格式和权威状态；不会初始化或迁移。
3. 单元仅为 Auth/Audit/Policy/socket 四根开写；公开示例、web、配置、release 保持只读。PrivateTmp 提供进程私有临时目录，不共享业务数据。
4. `RuntimeDirectory=studiolo-human` 创建0700父目录；`RuntimeDirectoryPreserve=yes` 避免停止/重启时由 systemd 清掉异常旧 socket。wrapper 发现任何已有 http.sock 就拒绝；没有 rm/清理钩子。正常退出由产品核身份后清理自身 socket，异常残留由主线先诊断再决定处置。
5. 生产机应先用 `systemd-analyze verify /候选目录/studiolo-human.service` 核对其实际 systemd 版本；本地 macOS 没有该工具，静态解析不等于服务器验证。确认密码加载成功后先验证候选 Unix 入口，再切 Tunnel。
6. Tunnel 需同 maxsong UID，service=`unix:/run/studiolo-human/http.sock`，httpHostHeader=`studiolo.maxsong.me`；保留真实唯一 `CF-Connecting-IP`。不要重启/替换其他服务来让候选“通过”。
7. 真人 Access 登录 → L2 Cookie/CSRF → 手机真实 TOTP step-up → 人侧发布/回滚/全局恢复与审计，仍是主线验收；`active`、socket存在或 l3_available=true 都不能代替。

## 本地静态核对

```sh
python3 -I -B -S -c 'import ast,pathlib; ast.parse(pathlib.Path("launch-human.py").read_text()); print("syntax OK")'
```

部署回执见 `ACCEPTANCE.md`。测试使用虚构种子；真实配对由主人在控制终端执行 `enroll-human.py`。
