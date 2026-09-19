#!/usr/bin/env bash
# 自托管 CI 服务器的一次性 root 初始化。只在**专用** CI 机器上执行：
#   sudo bash bootstrap-root.sh [<实例名> <runner 版本> <linux-x64 包 sha256>]
# root 只做需要 root 的事（apt 官方源的系统包、用户、systemd 单元、窄 sudo、清理定时器）；
# 给了三个参数时，最后以**无特权的 ghrunner** 身份调用同目录的 install-runner.sh 装好该实例（下载官方 runner 并验签）。幂等。
set -Eeuo pipefail
umask 022
export PATH=/usr/sbin:/usr/bin:/sbin:/bin DEBIAN_FRONTEND=noninteractive
[[ "$EUID" == 0 && ( $# == 0 || $# == 3 ) ]] || { echo '请用 sudo bash 执行；参数要么不给，要么给 <实例名> <版本> <sha256>。' >&2; exit 2; }
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
OPERATOR="${SUDO_USER:-}"
[[ -n "$OPERATOR" && "$OPERATOR" != root ]] || { echo '需要经 sudo 由普通运维用户执行，以便把其 SSH 公钥授给 ghrunner。' >&2; exit 2; }

# 1. 系统包。time：大文件用例用 /usr/bin/time 量峰值 RSS。
# T-081 起不装 docker：CI 的前端作业改成原生跑，本机 Linux 回路也改成在本机原生跑。
apt-get update -qq
apt-get install -y -qq build-essential pkg-config git curl ca-certificates python3 time jq rsync

# 2. 专用用户：无 sudo、无口令登录。T-081 起没有容器作业，因此**不**加入 docker 组（docker 组在本机等同 root）。
id ghrunner >/dev/null 2>&1 || useradd --create-home --shell /bin/bash --comment 'GitHub Actions runner' ghrunner
passwd -l ghrunner >/dev/null
getent group docker >/dev/null && gpasswd -d ghrunner docker >/dev/null 2>&1 || true
install -d -m 700 -o ghrunner -g ghrunner /home/ghrunner/.ssh /home/ghrunner/runners /home/ghrunner/hooks
# 运维用户已有的 SSH 公钥同样授给 ghrunner，之后的无特权步骤直接以 ghrunner 登录完成。
OPERATOR_KEYS="$(getent passwd "$OPERATOR" | cut -d: -f6)/.ssh/authorized_keys"
[[ -s "$OPERATOR_KEYS" ]] || { echo "找不到 $OPERATOR 的 authorized_keys。" >&2; exit 1; }
install -m 600 -o ghrunner -g ghrunner "$OPERATOR_KEYS" /home/ghrunner/.ssh/authorized_keys

# 3. runner 的 systemd 模板单元：每个仓库一个实例 actions-runner@<名字>，目录 /home/ghrunner/runners/<名字>。
#    LimitMEMLOCK：Studiolo 的秘密驻留面加固用 mlockall，上限不足时按设计失败关闭；在单元里给足，作业就不需要 sudo prlimit。
cat > /etc/systemd/system/actions-runner@.service <<'UNIT'
[Unit]
Description=GitHub Actions runner (%i)
After=network-online.target docker.service
Wants=network-online.target
ConditionPathExists=/home/ghrunner/runners/%i/.runner
ConditionPathExists=/home/ghrunner/runners/%i/runsvc.sh

[Service]
User=ghrunner
Group=ghrunner
WorkingDirectory=/home/ghrunner/runners/%i
# 官方服务入口：只有 runsvc.sh 会读 .path 并导出为作业的 PATH（run.sh 不会）。install-runner.sh 把它从 bin/ 拷到实例根目录。
ExecStart=/home/ghrunner/runners/%i/runsvc.sh
Restart=always
RestartSec=10
KillMode=process
KillSignal=SIGTERM
TimeoutStopSec=5min
LimitMEMLOCK=infinity
LimitNOFILE=65536
UMask=0022

[Install]
WantedBy=multi-user.target
UNIT

# 4. 窄 sudo：运维用户只能免密启停/查看 runner 单元，别的什么都不行。
cat > /usr/local/sbin/ci-runner-service <<'HELPER'
#!/usr/bin/env bash
set -euo pipefail
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
[[ $# == 2 && "$2" =~ ^[a-z0-9][a-z0-9-]{0,39}$ ]] || { echo '用法：ci-runner-service <enable-now|restart|stop|disable|status> <实例名>' >&2; exit 2; }
unit="actions-runner@$2.service"
case "$1" in
  enable-now) systemctl daemon-reload; exec systemctl enable --now "$unit" ;;
  restart)    exec systemctl restart "$unit" ;;
  stop)       exec systemctl stop "$unit" ;;
  disable)    exec systemctl disable --now "$unit" ;;
  status)     exec systemctl status --no-pager "$unit" ;;
  *) echo '未知动作' >&2; exit 2 ;;
esac
HELPER
chmod 755 /usr/local/sbin/ci-runner-service
printf '%s ALL=(root) NOPASSWD: /usr/local/sbin/ci-runner-service\n' "$OPERATOR" > /etc/sudoers.d/90-ci-runner-service
chmod 440 /etc/sudoers.d/90-ci-runner-service
visudo -cf /etc/sudoers.d/90-ci-runner-service >/dev/null

# 4b. 可锁定内存：秘密驻留面加固用 mlockall，上限不足时按设计失败关闭并终止进程。
# runner 的作业由单元里的 LimitMEMLOCK 覆盖；**SSH 会话**（scripts/test-linux.sh 的原生回路）走 PAM，需要这份 limits。
cat > /etc/security/limits.d/90-studiolo-memlock.conf <<LIMITS
ghrunner  -  memlock  unlimited
$OPERATOR  -  memlock  unlimited
LIMITS
chmod 644 /etc/security/limits.d/90-studiolo-memlock.conf
grep -q pam_limits /etc/pam.d/sshd || printf '警告：/etc/pam.d/sshd 未启用 pam_limits，memlock 限额可能不生效。\n' >&2

# 5. 磁盘清理：构建目录会把盘写满（2026-09-17 本机就出过一次）。每天一次；超过 80% 才动手。
cat > /usr/local/sbin/ci-runner-cleanup <<'CLEAN'
#!/usr/bin/env bash
set -euo pipefail
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
used=$(df --output=pcent / | tail -1 | tr -dc 0-9)
command -v docker >/dev/null && { docker image prune -af --filter 'until=168h' >/dev/null 2>&1 || true; }
if (( used >= 80 )); then
  # 只删可重建的构建产物；runner 空闲时才会有人来取这些目录。
  find /home/ghrunner/runners -maxdepth 5 -type d \( -name target -o -name node_modules \) -path '*/_work/*' -prune -exec rm -rf {} + 2>/dev/null || true
  find /home/ghrunner/cache -mindepth 2 -maxdepth 2 -type d -name target -prune -exec rm -rf {} + 2>/dev/null || true
  # scripts/test-linux.sh 的原生回路：每个工作树一份同步树与构建目录，7 天没动过就清。
  find /home/*/.cache/studiolo-target /home/*/.cache/studiolo-tests -mindepth 1 -maxdepth 1 -type d -atime +7 -exec rm -rf {} + 2>/dev/null || true
fi
CLEAN
chmod 755 /usr/local/sbin/ci-runner-cleanup
cat > /etc/systemd/system/ci-runner-cleanup.service <<'UNIT'
[Unit]
Description=CI runner disk cleanup
[Service]
Type=oneshot
ExecStart=/usr/local/sbin/ci-runner-cleanup
UNIT
cat > /etc/systemd/system/ci-runner-cleanup.timer <<'UNIT'
[Unit]
Description=Daily CI runner disk cleanup
[Timer]
OnCalendar=daily
RandomizedDelaySec=1h
Persistent=true
[Install]
WantedBy=timers.target
UNIT
systemctl daemon-reload
systemctl enable --now ci-runner-cleanup.timer >/dev/null

echo '初始化完成：ghrunner 用户、actions-runner@.service、ci-runner-service 助手、memlock 限额与清理定时器已就位（不含 Docker）。'
install -m 700 -o ghrunner -g ghrunner "$HERE/install-runner.sh" /home/ghrunner/install-runner.sh
if [[ $# == 3 ]]; then
  # 新加入 docker 组要在新会话里才生效；runuser 起的是新会话。
  runuser -l ghrunner -c "bash /home/ghrunner/install-runner.sh $(printf '%q %q %q' "$1" "$2" "$3")"
else
  echo '未给实例参数：之后以 ghrunner 登录运行 ~/install-runner.sh <实例名> <版本> <sha256>。'
fi
