# 正式 canonical 与黑板首次接线

这是绑定既有 `m4-t094-20260920-r8` 的一次部署，不是通用安装器。只为已有生产 Auth、Audit、Policy 和 agent 身份库增加 canonical/Broker/BoardPlane；不签发身份、不发布权限、不启用 Runner 或模型 API。任何已绑定文件、目录、单元或摘要变化都先停在预检，不覆盖其他发布。

## 构建与候选包

在 GithubCI 原生 Linux 完成 workspace fmt、严格 Clippy、全量测试、路径安全及生产特征构建：

```sh
cargo build --locked --release -p studiolo-vault-admin
cargo build --locked --release -p studiolo-local-server --features studiolo-auth/production
python3 deploy/blackboard/test_assembly.py
python3 deploy/blackboard/test_freeze.py
python3 deploy/blackboard/test_activation.py
python3 deploy/blackboard/test_sync_tools.py
python3 deploy/blackboard/test_sync_release.py
```

初始化工具不依赖 Auth，不具备测试因子验证器或 Auth 的 `production` feature；不要给它附加 `studiolo-auth/production`。人侧使用包含纯黑板能力集修复的新 production 构建；Auth 管理程序与网页仍复用已验证的 r8 文件。

生产工具目录保持仓库相对布局：本目录（含 `candidate.py`、`binding.py`、`snapshot.py`、`sync_tools.py`）、`deploy/human/release/{release_common,runtime_checks,activation_ops,snapshot_worker}.py` 和 `scripts/{replica_common,replica_store,replica_cleanup}.py`。目录及祖先必须真实、root 或 UID1000 所有、不可组/其他写；文件单链接。部署到专用路径，不从 `/tmp` 或另一个项目的工具目录运行。

（历史：r2 首次接线时）UID1000 运行当时版本的 `prepare.py --source-commit <初始化程序来源提交> --vault-admin <初始化程序绝对路径> --vault-admin-sha256 <摘要> --human-source-commit <人侧程序来源提交> --local-server <人侧程序绝对路径> --local-server-sha256 <摘要>`，生成固定 `assembly.RELEASE` 的全新私有 r2 候选包。**这组参数已废弃**，现行用法见文末「现行发行（rN→rN+1）」。它替换人侧程序、派生启动参数和两个服务单元，并加入初始化程序；Auth 管理程序与网页必须仍逐字节匹配 r8。manifest 分别记录两份程序的来源，不能将复用程序冒称为新提交构建。未完成候选保留 `INCOMPLETE`，不自动清理或重试覆盖。

r1 候选从未激活；它已完成一次 canonical 首建。r2 沿用同一已初始化的 canonical/Broker 绑定，不再次执行冻结或初始化。旧 r1 候选、冻结快照和首建审计原样保留。旧 agent 是 root:maxsong 的 0550 冻结程序；预检核验这一既有模式，不修改它的权限。

## 冻结与首建

1. 保留现有 Vault→agent 只读副本同步。UID1000 运行 `python3 -I -B -S …/freeze_replica.py`；它在原同步锁内核验 ready、300 秒新鲜度、三个批准范围和所有字节，写入全新私有快照。共享锁竞争、陈旧副本或不完整内容均拒绝。
2. 事先建立 `assembly.VAULT`、`assembly.WRITER`、`assembly.BOARD` 三个 UID1000/0700 空目录。禁止将轮换副本原地升级成写者。
3. 用新 `studiolo-vault-admin plan` 指向现有 owner 配置、candidate web、冻结快照中的 **`vault/` 子目录**和两个首建目录；快照外层含同步清单，不是 Vault 根。两个项目使用 `--allow 项目/Studiolo --allow 项目/Tillerwright`；`_规则` 含 ZIP/TXT 附件，必须逐一指定其 10 份 Markdown 的精确路径，不得批准整个 `_规则` 前缀。这次共 12 个范围、207 份 Markdown；四份附件仍保存在原冻结快照和原只读副本中，不导入 canonical。审阅文件数、字节数及 plan 摘要后，以完全相同参数加 `--expected-plan` 执行 `initialize`。两个命令都要求原生 Linux 进程加固成功，包括 unlimited MEMLOCK。
4. 首建通过 Broker 写入 Markdown，并记录 `local_os_bootstrap` 审计；不冒充人类 L3 或 Sync 证据。部分首建失败原样保留，禁止盲目再次初始化。

来源只被冻结一次。原 Obsidian 的后续变化仍进入旧只读副本，不会自动进入新的 canonical。网页标签明确显示“初始快照（持续同步未接通）”；新库不提供正文编辑所需的同步新鲜度证明。

## 双服务切换与验收

使用 root 执行固定工具：

```sh
sudo /usr/bin/python3 -I -B -S /可信工具目录/deploy/blackboard/activate.py \
  --manifest-sha256 <候选 manifest 的实际摘要>
```

预检要求两个原单元摘要、enabled/无 drop-in、实际旧进程、v2 认证绑定、完整候选文件集合及 Broker 配对均准确。`/var/lib/studiolo-deployments` 必须为 root 所有、无组/其他写权限的真实目录；保留既有 0750 总目录和历史记录的访问设置，本次子记录仍为 root/0700、锁及快照文件为 root/0600。顺序为停止 agent→停止 human→验证 PID 和 socket 均已退出→三库一致快照→替换单元→启动 human 并验证两个私有 socket→启动 agent。绝不删除存量 socket。

认证、恢复码计数、会话 epoch、owner 配置以及既有 agent 程序和三个读取范围不变。回执明确区分服务切换成功和尚未验收的权限/本机客户端。停止后故障仅在状态仍属于本次已知配置时恢复旧程序，不回滚数据库；未知认证/单元变化保留现场，标为 `blocked_requires_operator`。断电、SIGKILL 或恢复过程失败须依据私有发布记录人工处理。

切换后还须验证真实 MCP 黑板读取、未授权写入拒绝、原有读取新鲜度、人侧权限草稿与实际 executable。主人亲自完成 Claude/Codex 的 L3 发布后，才能验证授权写入、项目越权拒绝、持久化、撤权，以及两个本机 Consumer 的 claim/heartbeat/return 完整回执。工具测试、候选包、服务启动都不能代替这些生产验收。

## 现行发行（rN→rN+1）

r2 之后每次发行只做「上一发行 → 本发行」：换网页，可选换人侧程序，其余逐字节沿用（`assembly.py` 文件头有沿革，r5→r6 见 T-159）。

1. **改常量**（`assembly.py`）：`OLD_RELEASE`＝上一发行目录、`BUILD`＝新发行号；`HUMAN_SHA`／`AGENT_SHA` 取 169 现场 `/etc/systemd/system/studiolo-{human,agent}.service` 的 sha256；`R8_MANIFEST_SHA` 取上一发行 `manifest.json` 的 sha256。`test_assembly.py` 的发行号断言同步。三个变换函数不改。
2. **服务器上验工具**：`scripts/test-linux.sh bash -c 'python3 deploy/blackboard/test_assembly.py && python3 deploy/blackboard/test_freeze.py && python3 deploy/blackboard/test_activation.py'`（本机不跑测试）。
3. **构建**：人侧程序在 githubci 原生 `cargo --locked build --release -p studiolo-local-server --features studiolo-auth/production`；网页在 108 用同一提交 `npm ci && npm run build`。来源提交须已过 CI。
4. **暂存**（169，UID1000，0700/0600，程序 0755，不用 `/tmp`）：`bin/studiolo-local-server`、`web-dist/`、本目录 `{activate,activation_ops,assembly,prepare,candidate,binding,snapshot,sync_tools}.py` 与 `deploy/human/release/{activation_ops,release_common,runtime_checks,snapshot_worker}.py`，保持仓库相对布局。
5. **出候选**（UID1000）：

```sh
/usr/bin/python3 -I -B -S <暂存>/deploy/blackboard/prepare.py \
  --web <暂存>/web-dist --web-source-commit <40 位提交> \
  --local-server <暂存>/bin/studiolo-local-server --local-server-sha256 <64 位摘要> --human-source-commit <40 位提交>
```

   只换网页时省略后三个参数（三件套要么全给要么全不给），程序逐字节沿用上一发行、manifest 照抄其来源。T-132 r10 同时传入 `--maintain <程序> --maintain-sha256 <摘要> --sync-launcher <程序> --sync-launcher-sha256 <摘要>`；四项必须全给，并与上述人侧程序来自同一提交。manifest 新增 `sync_program_source_commit`，两个程序均为 ELF/0755；之后省略四项会原样保留，禁止静默移除、缺一程序或保留来源却换字节。工具随包交付不自动置备、配对或启用 Bridge。输出 `manifest_sha256`、`human_binary_reused`、`services_changed: false`。
6. **激活**（用户本人 root）：`sudo /usr/bin/python3 -I -B -S <暂存>/deploy/blackboard/activate.py --manifest-sha256 <上一步的摘要>`。已激活后重复执行会在预检被拒（现场已不是上一发行），无副作用。


本版固定 r9→r10（2026-09-29）：人侧程序与 Bridge 双程序来自 PR #54 合并提交 `bcd47f4`；网页沿用 r9 已验证字节和来源 `4a1ba63`，agent 仍为 t160。现场摘要在 assembly.py，激活继续保留认证与数据库；Bridge 的 P3/P4/P6 仍按独立方案执行。
