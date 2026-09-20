# 当前 M4 的恢复点与接入方策略启用（T-086 / T-087）

适用于已经运行 v2 审计账本、尚未给 `serve --policy` 的 M4。它不重新发布二进制，不执行 v1→v2 迁移，
不启用人侧 L3/因子模式。恢复包是同机可验证恢复点，不等于异机/NAS 灾难恢复，也不能补回丢失的历史 v1 账本。

## 操作包

先只读取得当前 `/etc/systemd/system/studiolo-agent.service` 原文和运行二进制 SHA-256；不要取 token 正文。

```bash
python3 -B scripts/prepare-m4-policy.py \
  --unit /private/tmp/observed-agent.service \
  --binary-sha <刚核验的SHA-256> \
  --output /private/tmp/studiolo-m4-policy-ops
```

准备器只接受当前 maxsong 的两个目录白名单、4400、私有数据与派生索引等已知单元形状。
生成的 `policy.service` 只比原单元多 `--policy /home/maxsong/.local/share/studiolo-agent-policy`
和对应 `ReadWritePaths`；身份、隔离、memlock、程序路径均保持。清单绑定每份操作文件。

当前已核验的有效类型只有 `claude`。首版能力集保留 `项目/Studiolo`、`_规则`；默认拒绝其他读取；
禁止直接写正文、写黑板与发布策略；允许提交永不自动生效的 desired；目前没有已发布的 skill，硬绑定为空。
操作前再次查询有效类型，出现其他类型就拒绝，先重做能力集材料。不会签发或轮换真实 token。

上传完整目录后先校验 `apply.sh` 摘要及 `FILES.sha256`，可以无特权运行 `systemd-analyze verify policy.service`。
最后由主人在**真实前台终端**执行 `sudo bash <操作目录>/apply.sh`，三个标准流不能重定向、管道或后台。
脚本显示能力集正文，并直接继承该终端执行 `policy-bootstrap`；不创建伪 TTY，不绕过主人门禁。

## 实际执行顺序

1. 校验 UID、TTY、当前单元/程序摘要、无 drop-in、4401 空闲、有效类型、线上现有完整供给。
2. 把操作文件固定到 `/var/lib/studiolo-deployments/t087-*`，root 持有，摘要不符即停止。
   此处只有非敏感操作材料，目录为 root:maxsong 0750；含凭据的恢复根另设 root:root 0700。
   停机前用服务用户验证实际读取权限，防止祖先目录权限阻断探针。
3. 短暂停止 4400 的 agent，在 `/var/lib/studiolo-recovery/<UTC>-t087-*-before-policy` 创建一致恢复包，然后及时启动原服务。
   包含身份库、v2 审计、既有 token、程序和原单元；SQLite backup 合入 WAL，逐库检查版本、quick_check、外键，并校验全文件清单。
4. 只恢复到新的私有目录，使用恢复出的程序和数据在系统管理器 4401 下启动，跑 MCP/HTTP/401/新鲜度验收。
   派生索引重新生成，内容来自现有持续更新的只读副本；恢复包不包含 canonical Vault 或整个内容副本。
5. 初始化独立策略目录，主人前台 bootstrap 首版 `claude` 能力集，审计进入当前生产 v2 账本。
6. 在复制出的策略库和身份库上验证 4401 策略分支、版本一致和白名单拒绝；再注入仅隔离副本中的吊销测试行：
   实例吊销只拒绝该实例、新实例仍允许；类型/全局吊销拒绝既有及新实例。测试清理只移除自己插入的行，再复验恢复。
   这一步验证运行时裁决，不声称验证了尚未部署的人侧 L2 管理 API。
7. 最终停机窗口重新保存含策略库及 bootstrap 审计的一致恢复包，原子换单元、启动并验收正式 4400。
   恢复包内保存计划启用的新单元；其完整性会验证，初始无策略恢复包已通过真实恢复副本验收。
8. 释放 4401，保留操作记录和两份恢复包。成功后保存不含凭据的 `RESULT.txt`，记录摘要、恢复包路径和已通过的验收项。
   失败不删除恢复包；预检副本可再生成，结束后删除。

## 失败与恢复边界

- bootstrap 前失败：原服务保持原单元；若正处于停机窗口，退出守卫起回原服务。
- bootstrap 后失败：新策略和审计保留。正式单元若已切换，按绑定摘要回退旧配置，不回退身份/审计历史。
- 发现第三方修改了单元时，拒绝覆盖它并报告人工检查。
- 策略目录已存在时，本脚本拒绝重复执行。先检查持久记录和 `admin policy-status`，不能删除策略库来绕过首版限制。
- 真正恢复数据必须先停所有写者、保全失败现场并决定需要回到哪个时间点；`restore-copy` 只向不存在的新目录复制，
  不覆盖生产路径。完整性验证是摘要及结构检查，不是抵抗备份所有者篡改的签名。
- 原 T-040 二进制不能读取 v2 审计库；禁止把本包称作恢复了旧 v1 回滚路径。
- 未来发布必须保留现行 `--policy` 及可写目录配置；默认模板仍是未初始化策略的兼容模式，不能直接覆盖已启用单元。

## 证据与命令

```bash
# 纯虚构夹具，按项目约定在 githubci 原生执行
scripts/test-linux.sh bash -euo pipefail -c \
  'python3 -I -B scripts/test-m4-recovery.py; python3 -I -B scripts/test-prepare-m4-policy.py; python3 -I -B scripts/test-check-agent-policy.py; python3 -I -B scripts/test-check-policy-revocation.py'

# 生产只读探针：token 只留在服务器文件中
python3 -I -B <操作目录>/check-agent-policy.py http://127.0.0.1:4400/mcp ~/.config/studiolo-agent/claude.token

# root 验证恢复包，不改变原包
sudo python3 -I -B <持久操作目录>/m4_recovery.py verify <恢复包>
```

策略探针对 initialize、handshake、skills 索引的可观测 effective 哈希前 12 位及能力集版本做一致性核对。
协议没有暴露全部 64 位，不能把它报告为全哈希验证。恢复包内部 SHA-256 则是完整摘要。
