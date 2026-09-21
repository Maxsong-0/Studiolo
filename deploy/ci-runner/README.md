# 自托管 CI runner（T-080，ADR-020）

私有仓库的 GitHub Actions 免费分钟数有限（2026-09 两天烧掉约 1,400 分钟后全部停摆）。自托管 runner 不消耗分钟数。
本目录是**专用** CI 机器的搭建脚本。一台机器可以给多个私有仓库各跑实例，也可以给**同一个仓库**多开实例并行跑作业；
机器本身也可以有多台，同标签即同池。当前在役：`githubci`（192.168.100.107，8 核 12G）、`github-2`（192.168.100.109，24 核 76G，T-099 于 2026-09-20 接入）。

**安全边界**：只挂私有仓库（公开仓库的外部 PR 能在机器上执行任意代码）；runner 用户 `ghrunner` 无 sudo、无口令、
**不在 `docker` 组里**（T-081 起工作流没有容器作业；docker 组在本机等同 root）。这台机器不放任何生产凭据，也不当生产机用。只需出站 HTTPS。

## 首次搭建（两条命令由人执行，其余可由 agent 完成）

```bash
# ① 在 CI 机器上，一次 sudo：系统包、ghrunner、systemd 模板单元、窄 sudo 助手、清理定时器，并以 ghrunner 身份装好实例
scp deploy/ci-runner/*.sh <运维用户>@<CI机器>:ci-runner/
ssh -t <运维用户>@<CI机器> 'sudo bash ~/ci-runner/bootstrap-root.sh studiolo <runner版本> <linux-x64包sha256>'
# ② 在自己电脑上：注册令牌经 stdin 送入，不回显、不落盘（令牌 1 小时有效）
gh api -X POST repos/<owner>/<repo>/actions/runners/registration-token --jq .token \
  | ssh ghrunner@<CI机器> '~/runners/studiolo/register.sh <owner>/<repo>'
```

版本与 sha256 取自 <https://github.com/actions/runner/releases>；`install-runner.sh` 不验签不解包。
之后（运维用户免密，仅限这一个助手）：`sudo /usr/local/sbin/ci-runner-service enable-now studiolo`，
再设仓库变量 `CI_RUNNER_LABELS=["self-hosted","linux","x64","studiolo"]`。**删掉这个变量即回退 GitHub 托管。**

## 给另一个仓库加实例

```bash
ssh ghrunner@<CI机器> '~/install-runner.sh <实例名> <版本> <sha256>'
gh api -X POST repos/<owner>/<repo>/actions/runners/registration-token --jq .token | ssh ghrunner@<CI机器> '~/runners/<实例名>/register.sh <owner>/<repo>'
ssh <运维用户>@<CI机器> 'sudo /usr/local/sbin/ci-runner-service enable-now <实例名>'
```

个人账号下 runner 只能按仓库注册；一个实例一次只跑一个作业，实例之间互不影响。

## 给**同一个仓库**多开实例（并行跑多个作业，T-100）

一个 runner 进程同一时刻只跑一个作业，想并行就多开实例。实例名必须唯一（它决定 runner 名 `<hostname>-<实例名>`），
但**标签必须和第一个实例相同**，作业才会派进同一个池——所以 `install-runner.sh` 的第 4 个参数给标签实例名：

```bash
# 实例叫 studiolo2，标签仍是 studiolo（省略第 4 个参数时标签沿用实例名，与旧行为一致）
ssh ghrunner@<CI机器> '~/install-runner.sh studiolo2 <版本> <sha256> studiolo'
gh api -X POST repos/<owner>/<repo>/actions/runners/registration-token --jq .token | ssh ghrunner@<CI机器> '~/runners/studiolo2/register.sh <owner>/<repo>'
ssh <运维用户>@<CI机器> 'sudo /usr/local/sbin/ci-runner-service enable-now studiolo2'
```

标签在**装实例时**写进该实例的 `register.sh`（`STUDIOLO_RUNNER_LABEL`），注册时不再接受参数：
标签拼错就等于装了一台永远派不到活的 runner（2026-09-20 实际踩过一次，见 ADR-020 沿革）。

开几个看机器：每个实例各有独立的 `CARGO_TARGET_DIR` 与 checkout，互不踩；Rust 作业吃 CPU 与盘，
24 核 / 76G / 80G 可用盘上建议 2 个、最多 3 个。仓库的测试不抢固定端口（真监听用 `127.0.0.1:0`），
同仓库两个作业并行是安全的；发布打包脚本写固定的 `/tmp/studiolo-checksums`，那条链路别并发跑。

## 机器上有什么

| 位置 | 作用 |
|---|---|
| `actions-runner@<实例>.service` | `User=ghrunner`，`LimitMEMLOCK=infinity`（Studiolo 的秘密驻留面加固用 `mlockall`，给足后作业不需要 `sudo prlimit`） |
| `~ghrunner/runners/<实例>/register.sh` | 装实例时生成，开头写死 `STUDIOLO_RUNNER_LABEL`（该实例注册时用的标签）；令牌只从 stdin 读、不回显不落盘 |
| `~ghrunner/runners/<实例>/.env` | runner 启动时自己读：作业开始钩子（把容器作业留下的 root 属主文件改回来）、`CARGO_TARGET_DIR`（常驻在工作区之外，不随 checkout 清掉） |
| `~ghrunner/runners/<实例>/.path` | 作业的 `PATH`（含 `~/.cargo/bin`）。只有官方服务入口 `runsvc.sh` 会读它，所以单元的 `ExecStart` 是 `runsvc.sh`；注册会覆盖它，故在 `register.sh` 末尾写。实测 `.env` 里写 `PATH` 对作业步骤不生效；用 rustup 的工作流另用 `GITHUB_PATH` 兜底 |
| `/usr/local/sbin/ci-runner-service` ＋ sudoers | 运维用户只能免密启停／查看 runner 单元 |
| `/etc/security/limits.d/90-studiolo-memlock.conf` | SSH 会话的 `memlock` 上限。runner 作业由单元里的 `LimitMEMLOCK` 覆盖，而 `scripts/test-linux.sh` 的原生回路走 PAM，需要这份 limits（秘密驻留面加固上限不足时按设计失败关闭） |
| `ci-runner-cleanup.timer` | 每天一次；根分区超过 80% 才删可重建的构建目录，并清 7 天没动过的原生回路同步树 |
