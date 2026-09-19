# 自托管 CI runner（T-080，ADR-020）

私有仓库的 GitHub Actions 免费分钟数有限（2026-09 两天烧掉约 1,400 分钟后全部停摆）。自托管 runner 不消耗分钟数。
本目录是一台**专用** CI 机器的搭建脚本；同一台机器可以给多个私有仓库各跑一个实例。

**安全边界**：只挂私有仓库（公开仓库的外部 PR 能在机器上执行任意代码）；runner 用户 `ghrunner` 无 sudo、无口令；
它在 `docker` 组里（容器作业所必需，在本机等同 root），所以这台机器不放任何生产凭据，也不当生产机用。只需出站 HTTPS。

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

## 机器上有什么

| 位置 | 作用 |
|---|---|
| `actions-runner@<实例>.service` | `User=ghrunner`，`LimitMEMLOCK=infinity`（Studiolo 的秘密驻留面加固用 `mlockall`，给足后作业不需要 `sudo prlimit`） |
| `~ghrunner/runners/<实例>/.env` | 作业开始钩子（把容器作业留下的 root 属主文件改回来）与 `CARGO_TARGET_DIR`（常驻在工作区之外，不随 checkout 清掉） |
| `/usr/local/sbin/ci-runner-service` ＋ sudoers | 运维用户只能免密启停／查看 runner 单元 |
| `ci-runner-cleanup.timer` | 每天清一次过期镜像；根分区超过 80% 才删可重建的构建目录 |
