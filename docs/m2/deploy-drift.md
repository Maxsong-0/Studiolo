# 部署漂移对账

> T-256（2026-11-28）。脚本：`scripts/deploy-drift.py`；回归：`python3 -I scripts/test-deploy-drift.py`（在测试箱跑，不在 ci 套里）。

回答两件事：

1. 哪些提交合了 main、线上的发行里没有；
2. 哪些 BUG 档还写着「修复中」，其实修复提交已经在线上，该验收了。

脚本只读：只起 git（本机仓库、只读子命令），不 ssh、不 fetch、不写任何文件。169 上那一步由用户敲，不带 sudo。

## 用法

在仓库根（要 Python 3.11 以上）：

```sh
git fetch origin
python3 scripts/deploy-drift.py command
```

它印两条命令。第一条在 169 上只读取发行记录，第二条对账：

```sh
python3 scripts/deploy-drift.py probe | ssh studiolo-169 'python3 -I -B -S -' > /tmp/deploy-drift-169.json
python3 scripts/deploy-drift.py report --record /tmp/deploy-drift-169.json
```

`report` 的参数：

| 参数 | 缺省 | 说明 |
|---|---|---|
| `--main` | `origin/main` | 拿哪个引用当 main；脚本不 fetch，报告头写着它是哪个提交 |
| `--bugs` | vault 的 `项目/Studiolo-Docs/06-BUG` | BUG 档目录；vault 位置认 `OBSIDIAN_VAULT_PATH` |
| `--mac` | 读本机 LaunchAgent | Mac 副本包的提交号；不给就从 `me.studiolo.agent-sync.plist` 指到的目录名取，并逐字节核内容 |
| `--worker` | 无 | Worker 线上那一版的提交号。Worker 没有机器可读的发行记录，不给就报「未知」 |

退出码：0 没有漂移也没有要看的；1 有漂移、看不清，或有该验收、要人看的 BUG；2 输入读不懂，不出结论。

## 线上那一份怎么认

| 发行 | 来源提交从哪来 |
|---|---|
| 人侧程序、网页、随包工具 | 人侧单元的 `ExecStart` 指到的发行目录，`manifest.json` 的 `human_program_source_commit`、`web_source_commit`、`sync_program_source_commit`（这一项可以没有） |
| agent | agent 单元的 `ExecStart` 指到的发布目录，`base-commit.txt`；`provenance.txt` 写着出自有未提交改动的工作区就不认 |
| 告警包 | 169 上 `/opt/studiolo-alert/current` 链接的目录名；108 那一份不在记录里 |
| Mac 副本包 | 同步包目录名里的提交号（或 `--mac`） |
| Worker | 只认 `--worker` |

`bootstrap_source_commit`（一次性首建工具）故意冻结，不算漂移。
任何一项读不出来就是「未知」，不当成没有漂移；记录本身读不懂（不是 JSON、重复键、键对不上、提交号形状不对、提交不在仓库里）整个退 2。

## 一个提交算不算进某个发行

按路径归类，常量都在脚本开头：

- Rust 程序：`crates/*/Cargo.toml` 的正式依赖和构建依赖闭包（线上那份和 main 那份取并集，开发依赖不算），加 `Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`，加编进程序的包外文件（`EMBEDDED`）。闭包读出来是空的就退 2。
- 网页 `web/`；agent 另加单元模板；Mac 副本包是 `scripts/sync-agent-vault.sh` 和 `scripts/replica_*`；告警包是 `deploy/alerts/`（不含 Worker）、两个验收脚本、`studiolo-alert` 和它用的两个库；Worker 是 `deploy/alerts/worker/`。
- 测试、`.md`、格式与检查的设置不算。
- 只动锁文件的提交，每个 Rust 程序都算。

`main` 上的合并提交不单列，列的是它带进来的普通提交；改动只夹在合并提交里（解冲突时带的）另列文件名。
线上的提交不在 main 上：单列，不对账。改到了哪儿都归不到的路径：单列并退 1，给脚本补归类。

单独部署、这里看不出上没上线的（备份 worker、门铃、Bridge 的单元、Runner、预览站、认证管理程序、Bottega）列在报告最后，不对账。

## BUG 档怎么认

只看状态行写「修复中」的档：有 frontmatter 认里面的 `状态:`，老档没有 frontmatter 认正文第一处 `状态：`；没有状态行的归「要人看」，目录里一份 BUG 档都没有就退 2。修复提交先认状态行反引号里的提交号（`a..b` 当区间），没有才认正文里说「合 main」的句子（「没合 main」「待合 main」不算）。

- **该验收了**：修复提交都在 main 上，碰到的每个发行线上都已经有。
- **还没上线**：写明哪个发行还没有哪个提交。
- **要人看**：没有状态行、看不出修复提交、提交不在 main 或不在仓库里、只碰了五个发行之外的东西、碰了单独部署的东西、碰到的发行线上是哪个提交未知。

两处放宽，都在那一行里写明：随包工具和人侧程序共用代码，人侧程序有了、工具还没有时只提醒；改了人侧或 agent 的装配工具时只提醒（装没装进去要看那次发行）。

## 局限

- 只比源提交，不核线上的二进制是不是真由那个提交构建的（那是发行装配的事）。
- `crates/core` 这类共用代码动一次，依赖它的每个发行都算有漂移：宁可多列。
- BUG 档的写法不统一，认不准的都归「要人看」，不猜。
