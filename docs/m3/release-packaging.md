# T-063 运行包与部署脚本

2026-09-17 建档。本页说明 local-server 运行包在 T-046～T-048 之后包含什么、
怎么在本机出包与自检、怎么在服务器安装与原生验收、以及怎么回滚。

发布线有两条，互不混用：

| 发布线 | 打包脚本 | 原生验收脚本 | 发布内容 |
|---|---|---|---|
| 只读供给 agent | `scripts/package-agent.sh` | `scripts/run-agent-tests.sh` | `studiolo-agent` 与客户端投影脚本 |
| local-server 运行包 | `scripts/package-preview.sh` | `scripts/run-preview-tests.sh` | `studiolo-local-server`、`studiolo-maintain`、`studiolo-auth-admin` 与示例网页 |

本页只覆盖第二条。第一条的现状见 [T-040 发布记录](read-supply-release.md)。

## 运行包内容

发布名沿用 `<里程碑>-<任务>-<日期>-<序号>`（例 `m3-t049-20260917-runtime01`），
既是输出目录名，也写进 `expected-source-root`，服务器端据此拒绝挪位置运行。

`payload/` 内容：

- `bin/studiolo-local-server`、`bin/studiolo-maintain`、`bin/studiolo-auth-admin`：
  三个生产程序，均以生产身份特征构建（`studiolo-auth/production` 与 `auth-admin/production`）。
- `product-features.txt`：生产构建的特征与链接回执，见下节。
- `runtime-libraries.txt`、`auth-admin-runtime-libraries.txt`、`maintain-runtime-libraries.txt`：
  三个程序打包时的 `ldd` 清单；打包阶段出现 `not found` 即拒绝出包。
- `tests/` 与 `test-list`：按 crate 与测试目标收集的独立 Rust 测试程序，仅编译不执行。
- `source.tar.gz`、`expected-source-root`、`base-commit.txt`、`provenance.txt`、`build-image.txt`、
  `rust-version.txt`：源码快照、记录的运行时路径、来源提交与构建镜像 digest。
- `web/`、`example-vault/`、`check-preview.py`、三个 `.service` 单元、
  `run-preview-tests.sh`、`install-runtime-package.sh`。
- `SHA256SUMS`：覆盖上述全部文件，最后一步生成。

## T-046～T-048 新增程序的覆盖方式

`studiolo-backup`、`studiolo-observability`、`studiolo-sync-transport` 是库 crate，
它们不产出独立可执行文件，而是链接进 `studiolo-local-server`。
「纳入运行包」因此有两层含义，两层都有门禁：

1. **测试随包**：`package-preview.sh` 的 `NODE_PRODUCTS` 之前那段 Cargo JSON 收集，
   要求按 crate 与测试目标点名收齐 `backup/contract`、六个 `observability/*_contract`、
   `host-config/sync_transport_contract`、`host-config/alerting_contract`，
   不接受其他 crate 的同名目标充数。`sync-transport` 的一次一运行契约写在 lib 目标内
   （`#[path] tests/owned_run_private`），因此按 lib 测试程序单独验收。
2. **真的被链接**：`NODE_PRODUCTS` 解析三次生产构建的 Cargo JSON，要求
   `backup`、`observability`、`sync-transport`、`host-config`、`core`、`vault-broker`、`auth`
   都作为工作区产物出现。只在 `Cargo.toml` 里声明而没有进入链接结果，等同于没发布。

同一段门禁还负责拒绝测试特征混入：任一工作区包启用了 `test-support`、
`studiolo-auth` 没有 `production`、生产构建里出现 `profile.test` 产物、
或三次构建对同一个包启用了不同特征，都直接拒绝出包，不写回执。
通过时写出 `product-features.txt`，服务器端与安装器都会再核一遍这份回执。

## 本机出包

```bash
bash scripts/package-preview.sh /绝对路径/m3-t049-20260917-runtime01
```

脚本会先在源码快照里跑四组 Python 契约（`test-check-preview.py`、`test-preview-package.py`、
`test-release-package.py`、`test-install-package.py`）与前端全套检查，再进容器交叉构建。
不出包只想自检时，单独跑：

```bash
python3 scripts/test-preview-package.py   # 13 项：测试收集与原生产品门禁
python3 scripts/test-release-package.py   # 16 项：生产构建门禁与整包验收门禁
python3 scripts/test-install-package.py   # 18 项：安装器三段逻辑（Linux 上含 2 项整脚本端到端）
for s in scripts/package-preview.sh scripts/run-preview-tests.sh \
         deploy/preview/install-runtime-package.sh; do bash -n "$s"; done
```

## 服务器安装

安装器只做三件事：校验整包、私有权限落盘、登记待重启。
**它不启停、不重启、不 reload 任何服务，也不写 `/etc`，更不需要 root。**

```bash
# 1. 解压运行包到临时目录后，安装到发布目录（发布目录不可变，同名重装会被拒绝）
bash install-runtime-package.sh /绝对路径/解压后的 payload m3-t049-20260917-runtime01
# 2. 解压源码快照到记录的 source 目录，再从该目录原生验收
cd ~/.local/share/studiolo-preview/releases/m3-t049-20260917-runtime01/source
bash ../run-preview-tests.sh
```

安装阶段的拒绝条件：包内出现符号链接等非普通条目、有组或其他用户可写的条目、
`SHA256SUMS` 不符、三个生产程序缺失或不可执行、缺少生产特征回执、
回执缺程序或缺新增运行时 crate、身份模块没有生产特征、回执含 `test-support`、
`expected-source-root` 与发布名对不上、发布根目录是符号链接或不属于当前用户、
同名发布目录已存在、上次安装留下未清理的暂存目录。

落盘后的权限：发布根与发布目录 `0700`，普通文件 `0600`，
`bin/` 与 `tests/` 下的程序 `0700`。

安装先写到同一文件系统上的 `.<发布名>.partial` 暂存目录，就位前复验三件事：
没有普通文件与目录之外的条目、没有条目对组或其他用户可见、
**落盘副本按随包 `SHA256SUMS` 逐字节仍与源包一致**。
最后一条挡的是复制中断与磁盘写满：发布目录一旦改名就位即不可变，
半截副本必须在就位前就被拒绝。任一条不符都不改名，发布目录不会出现；
暂存目录刻意保留供人工确认，并因此挡住下一次同名安装。

`run-preview-tests.sh` 在跑测试前重复同一组整包门禁（条目类型、权限位、`SHA256SUMS`、
生产特征回执），再核三个生产程序可加载运行库，最后逐个原生执行随包 Rust 测试程序。

秘密不进运行包：Access audience、授权邮箱、Tunnel token 全部走
`~/.config/**` 下的 `EnvironmentFile` 与 token 文件，由用户自己注入。

## 待重启与回滚

需要重启才生效的改动一律只登记、不执行。安装器把单元名追加到
`~/.local/share/studiolo-preview/pending-restart.log`（目录 `0700`、文件 `0600`），
由用户在自己选择的时间批量执行启用与重启。共享的 4318 预览、4319 Access 与
Tunnel 服务，安装器一概不碰。

回滚就是换回上一个发布目录：发布目录不可变、按发布名并存，
回滚只需把单元指回上一个发布名并由用户重启，无需反向迁移任何数据。
`deploy/preview/10-openat2-compat.conf` 等既有 drop-in 不随本流程改动。

## 已验证与未验证

**已在本机验证（2026-09-17）**：三组 Python 契约共 47 项通过
（macOS 上 `test-install-package.py` 的 18 项里，2 项 Linux 端到端跳过）；
在 `rust:1.97.1-bookworm` 容器内以非 root、含空格的发布根目录整脚本跑通安装器，
18 项全绿，包括私有权限、待重启登记、同名重装拒绝、摘要篡改拒绝与落盘复验拒绝；
`package-preview.sh`、`run-preview-tests.sh`、`install-runtime-package.sh`
等七个 bash 脚本 `bash -n` 通过。

`test-preview-package.py` 与 `test-release-package.py` 解析的是打包脚本内嵌的 Node 判定代码，
因此只能在装有 `node` 的构建机上跑；服务器端的 `run-preview-tests.sh` 不需要 Node。

**未验证**：真实服务器部署、systemd 单元的实际启用与重启、公网入口复验，
以及容器交叉构建出包的完整一次运行——后者需要在容器内完整构建整个工作区，
本任务没有执行，因此运行包的实际产出物尚未见过。
本任务范围内不做真实部署，这些条目须在正式发布时另行完成并留证。
