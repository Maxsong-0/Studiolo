# 固定人侧 r7 → r8 发布

仅用于已授权的生产程序发布：发行号固定 `m4-t094-20260920-r8`，服务仅为
`studiolo-human.service`。它携带新 auth-admin，但不会调用生产撤权、登记或迁移命令。
本次发布不证明 T-093 全部完成，恢复副本的新登录/L3 与新 agent 身份正向验收仍是独立事项。

## 打包

在私有、无符号链接、祖先不可被组/其他用户写入的源码及暂存路径运行；不要使用 `/tmp`
或共享 `0775` 同步父目录。目录和输入归运行用户所有，祖先只允许该用户或 root。
`web/dist` 必须已由主线完成构建；二进制必须来自主线通过检查的 Linux production 构建。
示例正文显式指定仓库 `examples/m1-vault`，禁止换为真实 canonical vault。

```sh
python3 -I -B -S /ABS/PRIVATE/REPO/deploy/human/release/package.py \
  --repo /ABS/PRIVATE/REPO \
  --example-vault /ABS/PRIVATE/REPO/examples/m1-vault \
  --local-server /ABS/PRIVATE/STAGING/studiolo-local-server \
  --auth-admin /ABS/PRIVATE/STAGING/studiolo-auth-admin \
  --sync-launcher /ABS/PRIVATE/STAGING/studiolo-sync-launcher \
  --source-commit FINAL_40_LOWERCASE_HEX_COMMIT \
  --output /ABS/PRIVATE/STAGING/m4-t094-20260920-r8
```

输出必须不存在。输出目录 `0700`，普通文件 `0600`，三个二进制 `0755`（`studiolo-sync-launcher` 随 T-132 P1 加入清单）。
失败保留已有不完整输出，不覆盖、不自动删除。模板只替换固定 r7 RELEASE 路径。
manifest 顶层闭集为 `build/source_commit/assembly/files`，每项闭集为 `bytes/sha256`。
打印的 `manifest_sha256` 由操作者保留并独立传给激活命令。

## 安装前边界

主线将完整发行目录置于以下固定新路径，属主 UID 1000，保留上述严格权限：

`/home/maxsong/.local/share/studiolo-human-releases/m4-t094-20260920-r8`

工具自身和固定辅助文件也放到祖先不可组/其他写、无链接的目录，属主只允许 root 或
UID 1000；将经审阅的本目录脚本一起安装，不允许从 cwd/PYTHONPATH 搜索模块。
生产 root 操作者与服务 UID 1000 属于受信发布边界；这不是隔离恶意同 UID 进程的沙箱。

激活要求旧 unit SHA256 精确为
`66d3689f8d73a42edf1ba56364cc8a3212b4533ce91ca8bffe09d7455be5d847`，
旧二进制 SHA256 为
`8b5f1aa5643b7e9c8934760e307c84b365babd91cd953952ede51d96805bae0a`。
固定 owner.json 绑定 v2/Auth/Audit，Auth schema=4、Audit=2、Policy=1；两个 pending 均空，
原恢复证明存在、恢复码计数为 0 或 10，生产 restore 审计计数必须为 0。
manifest、文件集、所有节点/祖先/权限/属主/链接数和跨设备边界均先检查。
无 drop-in、enabled 旧单元和旧真实进程通过核验后，`systemd-analyze verify` 必须通过。

```sh
sudo -n /usr/bin/python3 -I -B -S /ABS/TRUSTED/TOOLS/activate.py \
  --manifest-sha256 EXACT_64_LOWERCASE_HEX_FROM_PACKAGE
```

不提供可变服务名、数据库目录、回滚目标或凭据参数。

## 一致保护与失败处理

激活先创建 `/var/lib/studiolo-deployments/human-r8-*` root `0700` 私有记录目录，
用专用 flock 防止本工具并发。只停止 human，确认无 PID 且产品已正常删除自身 socket。
不会删除异常 socket，也不会重启 agent 或 Tunnel。

备份 worker 降为 UID 1000：对 Auth/Audit/共享 Policy 全部取得 `BEGIN IMMEDIATE` 写屏障，
保持三把锁，用另外的只读连接 SQLite backup 到内存，完整性检查后 serialize 到 root
预打开的 `0600` 文件 FD。所有写事务最终只 ROLLBACK；不更改源持久 PRAGMA 或迁移 schema。
源上其他写者被 SQLite 写屏障阻塞；不能取得全部锁就失败，绝不接受部分备份。
每库最多 512 MiB、总快照最多 1 GiB，worker 内部期限 25 秒、外部 40 秒；core dump 禁用。
备份包含原 unit、owner 配置、三库独立副本及其摘要，绝不把正文、种子或 token 输出日志。
生产需 Python 支持 `sqlite3.Connection.serialize`；缺少时在停服务前拒绝。

备份和绑定复查通过后才原子替换 human unit、daemon-reload、启动。等候真实
`/proc/PID/exe` 指向新二进制及 socket，不能把尚在执行 Python launcher 的 active 状态当完成。
随后核对二进制 inode/hash、唯一 `totp-next` credential 名称、socket UID1000/0600、父目录0700、
六端点匿名401，以及配置/v2/epoch/恢复码/restore计数未变。

失败写固定类别的非秘密回执；停服务后的备份失败会尝试恢复旧服务。已改 unit 时先停候选，
核实配置和认证绑定未变，再恢复旧 unit 并启动 r7。**不回滚数据库**。
未知 unit、认证绑定变化或残留 socket 会阻止自动回退，回执标为 `blocked_requires_operator`。
成功为 `accepted.json`；失败为 `failed.json`。磁盘写满可导致回执持久化失败，此时 stdout
含 `failure_receipt_persisted:false`。SIGKILL/断电不可能由进程内异常回退保证，须人工读本批记录。
备份目录可能含敏感数据库，始终 root 私有，不复制到 CI，也不自动清理。

## 虚构测试

仅在主线安排的原生 Linux 测试机运行：

```sh
python3 -I -B -S deploy/human/release/test_release.py
```

入口包含 `test_snapshot_worker.py`。夹具在当前用户 HOME 下新建独有 `0700` 目录；
HOME 祖先必须满足同样的不可组/其他写约束。测试不会放宽产品门禁。
服务操作使用注入式替身；SQLite 使用真实虚构数据库，worker 的固定生产路径/身份及
root 输出 FD 元数据仅在测试中替换。覆盖 WAL 内容、全库先锁后读、失败释放锁、版本拒绝、
manifest/链接/权限/源变化拒绝、旧 unit 错误不停止、备份失败恢复、候选失败回退和真正 exec 等待。
这些测试不替代主线的生产安装验收。
