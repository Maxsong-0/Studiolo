# T-093 离线封装操作器

`pack_offline.py` 与同目录 `pack_offline_helpers.py` 是一组标准库 Linux 工具，只组装新目录并验证；不执行加密、归档、传输、服务启停、身份吊销或目录清理。本目录只含程序与虚构测试，不含生产数据。

## 输入边界

- `--recovery`：已停写生成、由原 `m4_recovery.py verify` 认可的完整恢复包。打包前及复制后再次执行原验证器；保留可选策略库、token 和原有固定布局。
- `--human`（可选）：由独立 `human_snapshot.py verify` 通过的冻结人侧快照；提供时输出schema2，完整保留Auth/Audit、owner配置、release和portable-factor.age。可信tools必须同时提供human_snapshot.py及其_io/_format/_core三个模块；不得从待验证包选择验证器。
- `--replica`：调用方已冻结的完整接收端根，包含 `sync.lock`、`control/state.json`、`releases/<generation>/{manifest.json,vault/}`。保留全部现存代次；拒绝 `.stage-*`、临时 state、未知节点或 ready 回执不匹配。沿用原 `replica_common`/`replica_store` 校验每代完整文件集、白名单路径、大小及摘要。
- `--tools`：调用者拥有、模式 0700 的可信普通目录，至少含 `m4_recovery.py`、`replica_common.py`、`replica_store.py`、`replica_cleanup.py`；也可放恢复探针及其依赖。打包器会执行其中的验证器，因此必须来自已审阅、固定版本的源码，不能指向未经鉴真的解密包。
- `--runtime-libraries`：与恢复包二进制对应的依赖清单普通文件。
- `--os-release`：系统发行信息普通文件。`/etc/os-release` 常是符号链接，应使用已确认的 `/usr/lib/os-release` 或可信生成的普通副本。
- 所有输入与输出都用绝对路径；拒绝 `..`、链接祖先、组或其他用户可写祖先（root sticky 临时目录除外）。恢复、replica 和工具根为 0700。输入树内不得有软链接、硬链接、特殊节点或跨设备挂载。
- root 调用允许读取已知 uid 1000 的 replica；每个输入树内属主一致。可信工具和恢复包必须属于调用者。root 输出属 root。

读取输入时检查目录与文件 inode、属主、模式、链接数、大小、mtime/ctime，并在复制完成及验证完成后重读、重新计算摘要，重新核验路径绑定。检测到变化即失败。调用方仍须先冻结写者；工具不会替代停写与跨库一致恢复点管理。

## 命令

以下路径是待替换的占位符，不是生产命令已执行的记录：

```sh
sudo python3 -I -B /trusted/pack_offline.py pack \
  --recovery /private/frozen-recovery \
  --replica /private/frozen-replica \
  --tools /private/trusted-dr-tools \
  --runtime-libraries /private/runtime-libraries.txt \
  --os-release /usr/lib/os-release \
  --output /private/new-offline-bundle

sudo python3 -I -B /trusted/pack_offline.py verify \
  /private/new-offline-bundle --tools /private/trusted-dr-tools
```

输出必须不存在。父目录必须已存在。即使失败也不会覆盖、删除或清理目录；若已经开始输出，保留 `INCOMPLETE` 与已复制文件。`verify` 拒绝不完整包。错误和成功输出均不打印文件内容、token 或其摘要。

## 输出与验证

```text
new-offline-bundle/
  recovery/                    原恢复包完整布局，含其 manifest.json
  replica/                     接收端完整根及全部现存代次
  tools/                       可信工具、探针及显式提供的依赖
  system/runtime-libraries.txt
  system/os-release
  manifest.json                外层严格清单
```

目录全部 0700；文件全部 0600，schema1的唯一例外 `recovery/bin/studiolo-agent` 为 0700；schema2另外保留 `human/release/bin/` 下已核验的0700/0755，以便内层恢复包始终能直接通过原验证器。工具通过 `python3` 调用，不依赖执行位。

外层 schema 1 清单包含 `state=complete`、UTC `created_at`、每个目录的 `mode`，以及每个普通文件的相对路径、`size`、`sha256`、`mode`。JSON 中 mode 是整数，例如 0600 记录为 384。清单自身不自哈希，根目录模式由校验器固定为 0700。重复键、未知字段、缺失或多余文件/目录、大小/摘要/模式差异均失败。

`verify` 先核外层清单，再使用 `--tools` 指定的独立可信验证器检查恢复库与 replica 协议，最后重新核外层及清单稳定性。包内程序不会被自动执行。清单摘要是完整性检测，不是数字签名；异地加密、密文留存及外部可信摘要证据由主线负责。

上限为每次扫描 100,000 个节点、单文件 1 GiB、总计 8 GiB；replica 原有更严格的每代预算仍生效。超过限额会失败并保留不完整输出。

## 恢复边界

此包不是 canonical 全库备份：replica 仍只是现有白名单副本。schema1不含人侧会话/因子，schema2可通过显式human输入纳入；两者都不自动纳入Control/blackboard、Vault其他内容或完整系统镜像。

它也不是自动身份失效实现。恢复时必须先在隔离副本中使恢复的原 token 失效，再由主线独立 CLI 重新建立获准身份，验证旧 token、旧实例/会话不能复活，之后才允许服务对外监听。不得仅因文件验证通过就上线恢复实例。

`runtime-libraries.txt` 和 `os-release` 本身只是环境重建依据；可另在tools/runtime显式提供已核验的实际动态库，本批T-094已如此封装。目标机仍须有匹配架构、内核和外部工具。此封装工具不证明异机可启动、RTO/RPO或完整灾备验收。

replica 的 `source_observed_at_ms`、`received_at_ms` 原样保留。过期副本允许被保存和验证，但仍应被运行时判定 stale；严禁修改时间戳伪造新鲜度。

age 加密与传输只由主线执行：私钥仅存 Mac 专用 0700/0600 目录，不放 NAS、169、CI 或共享 Docker；NAS 仅接收密文。不要把真实 token 或数据库交给本测试。

## 验收

AST 与107原生Linux的22项虚构夹具均通过。后续复验由主线在107原生Linux执行，`--source` 指向含原验证器的已审阅源码树：

```sh
python3 -I -B /path/to/studiolo-t093-dr/test_pack_offline.py \
  --source /path/to/studiolo-source -v
```

22 项夹具覆盖两代完整副本、stale 时间戳保留、内层恢复包验证、源不变、目标冲突、链接/特殊节点/权限、输入变化、不完整保留、损坏/缺失/额外文件或目录、重复清单键与同名系统输入。测试只建立和删除自己创建的临时虚构目录。

## schema2人侧封装

`pack ... --human /private/frozen-human` 添加固定 `human/` 根；顶层增加 `snapshot_id` 并与人侧严格清单相等，其余四根保持原契约。schema1仍只接受原四根，不通过改版本号偷加人侧文件。当前发行里的已登记 `.gitkeep` 叶文件允许保留，仍拒绝路径穿越和其他隐藏节点。

人侧数据库/文件语义、seed_ref=v2、原发行完整清单由外部可信human验证器执行；外层校验两次文件树与摘要，并保持内层执行位。两层清单只证明冻结输入的一致保全；调用者必须在同一停写窗口冻结agent/human/共享Policy，snapshot_id不能替代跨库停写证据。

portable-factor.age由外部可信流程提供，本工具不解密、不验证其真实因子绑定，也不使任何旧权限失效。包含密文文件不是已完成异机恢复。新恢复撤权API及CLI已获主人授权，并在108真实恢复副本验收；生产Auth未执行撤权。实际边界见[人侧恢复回执](../../human/offline/ACCEPTANCE.md)。

完整虚构回路通过如下入口运行，入口把固定源码组复制到自己的0700临时目录再运行测试，结束仅清理该副本；无需修改共享测试树权限：

```sh
bash scripts/test-linux.sh python3 -I -B deploy/agent/offline/run_snapshot_tests.py
```

测试入口输出冻结源码摘要并在结束时复查来源未变。人侧fixture只覆盖快照所需表形状，不证明Rust Auth全部语义或审计链验证。
