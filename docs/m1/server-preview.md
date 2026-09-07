# M1 · 服务器示例运行

T-008：用户于 2026-09-04 指定 `maxsong@192.168.100.169`，提前迁移当前 M1 示例版。
这不代表 M2 的远程鉴权、备份或写入能力已经完成。此服务器先承担运行与验收，
本机源码与未提交工作保留，不自动迁走开发工位，也不复制真实 Vault。

后续 [T-009 域名部署](cloudflare-preview.md) 已配置独立 4319 鉴权入口及域名门禁，待真实登录验收；
不改变本页 native02 / 4318 的既有部署和历史验收记录。

## 已核实环境与访问边界

- Ubuntu 26.04 LTS、Linux 7.0.0-31 x86_64，4 vCPU、7423 MiB 内存，核实时 86 GiB 磁盘可用。
- 用户级 systemd 259，支持 ProtectSystem=strict / ProtectHome=read-only 隔离。
- 服务器不需要 Rust、Node 或 Docker；运行包在本机现有 Docker 的 Linux amd64 环境构建。
- 应用仍只监听服务器 `127.0.0.1:4318`，不开放 LAN/公网 HTTP，不放宽 Host、Origin 或 CORS。
- SSH 两端端口必须相同；浏览器须使用 `127.0.0.1`，不能改用 `localhost` 或服务器 IP。
  这里的 SSH 承担远程身份验证；服务器本机进程仍视为可信，不是多租户鉴权方案。

## 构建匹配源码的运行包

在本机仓库根目录运行，输出目录必须不存在；构建需要本机 Node 与 Docker：

```bash
mkdir -p target/preview-packages
bash scripts/package-preview.sh "$PWD/target/preview-packages/<唯一发布名>"
```

脚本将当前工作区白名单源码存档，在独立源码快照构建前端并跑测试；随后在 Linux amd64
构建 release 服务与全部 Rust 单元/集成测试程序，后者必须在服务器原生执行。
Mac 的 amd64 仿真已实测不支持 openat2，不能作为路径安全通过证据。
`payload/` 含二进制、网页、虚构示例库、原生测试程序与执行清单、
`source.tar.gz`、基线提交信息、构建镜像/运行库信息与 `SHA256SUMS`。
基线提交不是运行包版本：当前 M1 未提交，来源应以源码归档和完整校验清单为准。
不包含 `.git`、依赖缓存、真实笔记或凭据。成功包保留作为验收证据，不自动清理用户文件。
服务器将归档解压到该发布目录的 `source/`，与构建时记录的绝对源码路径一致，
保证架构测试验证随包提供的真实源码与 CI 约束，而不是缺文件后绕过测试。
每个发布名使用独立 Docker 编译卷：已有 Cargo 目标缓存可能保留旧的编译时绝对路径，
不能只根据新 Cargo JSON 的 manifest 路径判断测试二进制已经重编。发布名不可复用。

## 服务与安全转发

服务模板在 `deploy/preview/studiolo-preview.service.in`；将 `@RELEASE@` 替换为已经校验的
独立发布目录名，安装到用户级 `studiolo-preview.service`。运行路径必须是实体目录，
示例文件不能是硬链接；使用 Linux openat2，失败关闭，不降级为不安全路径读取。

- 发布根：`/home/maxsong/.local/share/studiolo-preview/releases/`；每次部署使用新名字，不覆盖既有发布包。
- 服务单元：`/home/maxsong/.config/systemd/user/studiolo-preview.service`。
- 服务以 maxsong 运行，文件系统只读隔离、无新增权限、私有临时目录；内存上限 512 MiB，任务上限 64。
- 为退出 SSH 后继续运行，需要 maxsong 的 linger；该用户级设置和服务的启用均须记录实际结果。
- 初次安装前检查同名单元不存在；更新既有实例必须由用户授权后再切换单元并重启，不重启其他服务。
- `RestrictSUIDSGID=yes` 在该服务器会让只读 openat2 返回 ENOSYS；模板已设为 no，
  仍保留 ProtectSystem/ProtectHome 的只读隔离和 NoNewPrivileges。native02 原始单元不改写，
  使用独立 `10-openat2-compat.conf` 覆盖该项，配置文件在仓库 `deploy/preview/`。

原生测试必须在安装并启动服务前完成；运行包内 `run-preview-tests.sh` 检查源码位置并
逐项执行 `tests/` 中列明的测试程序，任何失败立即停止。不为部署跳过 M0 路径安全或架构回归。

Mac 上单独终端建立转发（保持此命令运行）：

```bash
ssh -N -o BatchMode=yes -o StrictHostKeyChecking=yes -o ExitOnForwardFailure=yes \
  -L 127.0.0.1:4318:127.0.0.1:4318 maxsong@192.168.100.169
```

打开 [服务器示例工作台（经 SSH）](http://127.0.0.1:4318/)。本机旧 4317 不受影响。
转发停止或 Mac 断网后仅本机访问中断，服务器服务不因此停止。重新连接后刷新整个页面；
generation 仅在单进程内递增，不能跨服务重启沿用旧网页状态。

## 验证与运维

在 Mac 或服务器上对示例端点运行：

```bash
python3 scripts/check-preview.py http://127.0.0.1:4318 --example
```

脚本只接受回环地址，核对示例标识后才显式刷新；不把它用于真实 Vault 验收。
检查总览/中文搜索/来源导航、静态资源、过期代次和跨源/写入拒绝。

服务器中查看该服务：

```bash
systemctl --user status studiolo-preview.service --no-pager
journalctl --user -u studiolo-preview.service -n 30 --no-pager
```

停止此部署：`systemctl --user disable --now studiolo-preview.service`。发布文件仍保留，
不删除源码或笔记。linger 影响整个 maxsong 用户的服务，停用前必须确认没有其他服务依赖它。

## 实际部署记录

2026-09-04（服务器日志使用 UTC，为 2026-09-05）：部署已完成。

- 发布目录：`/home/maxsong/.local/share/studiolo-preview/releases/m1-t007-20260904-native02/`。
- 本机完整构建包与原生测试日志：`target/preview-packages/m1-t007-20260904-native02/`。
- 用户服务 `studiolo-preview.service`：active / enabled，Linger=yes；退出原 SSH 再登录仍在运行。
  本轮未重启服务器，不将启用配置冒充实际重启演练。
- 服务仅监听 `127.0.0.1:4318`，内存限制 512 MiB、TasksMax=64、NoNewPrivileges=yes，
  ProtectSystem=strict、ProtectHome=read-only；新配置启动后 NRestarts=0。
- 服务器原生 33 个测试程序共 **166 项 Rust 单元/集成测试全通过**，包括 8 项 Linux 路径测试；
  前端快照 **28 项测试及生产构建通过**。服务器直连与 Mac 经 SSH 的 **41 项 HTTP 检查分别全通过**。
- 浏览器内置自动化：URL/标题正确、页面非空、无框架覆盖层、控制台 warn/error 为空；
  搜索→纸页范围→第 6 行→大纲第 8 行→Esc 焦点恢复通过。最终有效截图取默认 CSS 724×1170 窗口，
  无整页横向溢出。虚拟大尺寸截图出现捕获拼接异常，已恢复默认尺寸，不将异常截图作为桌面验收证据；
  本轮没有改动前端，独立重建的 JS/CSS 内容与 T-007 已验收产物相同。
- 完整文件校验通过；应用读取/刷新前后示例目录清单指纹相同，无源文件变动。仅部署虚构示例，
  没有真实 Vault、远程写入、公网鉴权或性能基准；M1 仍未全部完成，未提交推送。

可复核校验值（SHA-256）：

| 对象 | 值 |
|---|---|
| 运行包 SHA256SUMS | `b4de04aeac1148d48de7a46a5adffbc3d29900e09c96020b8f633ec7cd9a29a1` |
| 服务二进制 | `7bb64e2c3db34d723d8443590fa3a8a616b6a2d99228e8a186d8bd5da2f5dc08` |
| 独立 openat2 兼容配置 | `684c4ea5115b58f63974ec246b2631eec5e6fda7b198216b9066d40aa6c4f762` |

当前兼容配置保存在 `/home/maxsong/.local/share/studiolo-preview/10-openat2-compat.conf`，
用户服务的 `studiolo-preview.service.d/10-openat2-compat.conf` 链接至它。
native02 源码归档含 macOS provenance 扩展头，GNU tar 忽略时会给提示；文件字节校验和原生测试均通过。
