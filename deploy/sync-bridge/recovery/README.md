# Bridge 启用恢复点

P3 必须先于真实库配对。`canonical_snapshot.py` 在 human/agent 已停止时复制 canonical、writer、board，
拒绝 WAL/SHM、链接、硬链接、可写祖先、复制期间变化和重叠输出；输出三根闭集清单及其 SHA256。
`verify` 要求调用方提供此前独立保存的清单摘要，`restore` 只写全新私有目录，不覆盖线上根。

```sh
python3 -I -B canonical_snapshot.py freeze --output /ABS/new-snapshot --snapshot-id CHECKPOINT
python3 -I -B canonical_snapshot.py restore --snapshot /ABS/new-snapshot --manifest-sha256 SHA256 --output /ABS/new-copy
```

复制后不能手改物理身份或调用 initialize。使用同批源码构建的
`cargo build --release -p studiolo-vault-broker --example t132-restore-probe`，以恢复目录属主运行：

```sh
t132-restore-probe --vault /ABS/new-copy/canonical --writer-data /ABS/new-copy/writer \
  --vault-uuid ORIGINAL_UUID --pair-id ORIGINAL_PAIR --checkpoint CHECKPOINT
```

探针在调用 `rebind_new_roots` 前核对两份封存绑定，拒绝任一根仍是原物理根；重绑定成功后检查覆盖范围、
`open_existing` 与无 pending 写者状态。重绑定会改变 canonical 的 `.workbench/broker-binding.json`；
其余 canonical 内容必须逐文件摘要一致。board 另以只读 SQLite quick_check、外键、vault UUID、迁移版本和行数验收。
恢复内容留在 169，不能送进共享 CI 主机。没有 NAS 删除/覆盖负测及 Mac 归档时，不能签发 P3 accepted。

## NAS 追加备份

现场 NAS 尚无 Container Manager，采用同版本 `rest-server` 的原生 chroot 装配，保留既定 append-only 路线。
`studiolo-rest-backup.service` 适配 NAS systemd 219：只在 LAN 地址 192.168.100.111:18443 监听 TLS1.3，
HTTP bcrypt 认证、`--private-repos`、`--append-only`；运行 UID/GID 61032 事前须未占用，chroot 内只有程序、
认证/TLS 文件与数据。程序 root 0555，认证与私钥 root:61032 0640，数据 61032:61032 0700。
NAS 管理员的既有 SSH/sudo 身份保留清理权；追加备份密码没有 SSH 或文件系统权限，不提供网络清理服务。

客户端 restic 0.19.1，服务端 rest-server 0.14.0；二进制摘要和安装回执保存在当次 Vault 执行记录。
`configure-backup.command` 只由用户自己的 Mac 前台终端运行，要求两端已置备固定程序路径。
它让用户输入 NAS 追加密码，再在 169 输入独立仓库加密口令；公开 TLS 证书经已验证 SSH 路径传入客户端，
不关闭证书验证。NAS TLS 私钥只在 NAS 生成；口令、bcrypt 文件、原始错误正文不进 agent 输出。
任一步失败立即停止；保留已完成步骤，由操作者按元数据判断后续，不覆盖既有凭据。

`test_credentials.py` 的 activate 契约使用真实 OpenSSL 生成临时夹具证书，核对 IP SAN、
自签名验证与文件权限；systemctl 只使用替身，不启动测试主机上的服务。它同时验证已有 TLS
材料和非私有终端会被拒绝。测试证书不用于生产，也不替代 NAS 实际备份与恢复验收。

完成后仍需：制作 P4 前的新快照、restic backup、NAS 实际恢复到新目录、完整性与 Broker 验收、
用追加账号对独立测试对象做删除/覆盖拒绝检查、Mac 只读归档。全部成立后主进程才能生成
root 0600 `/var/lib/studiolo-bridge-p3/accepted.json`；配对入口只读该回执，不代替验收。

## 验证

所有检查在 107 经 `scripts/test-linux.sh` 执行：三个 Python 测试文件、恢复探针 example 测试、fmt、
example 严格 Clippy 与 release 构建。真实挂载还须在 169 检查 owner 显示身份、真实根 payload 身份、
owner 写入和 mirror→plane rename；旧手册曾把 `X-mount.idmap` 两列写反，上游勘误：
https://github.com/util-linux/util-linux/commit/f2bfef30ded60f0a9b15d428f13f5fa5d19e4116 。
