# 人侧冻结快照

`human_snapshot.py` 是仅使用 Python 标准库的 Linux 封装工具。调用方必须先共同停止所有 Auth、Audit、Policy 及相关管理写者，并在整个 create 期间保持停写；它不会自行停止服务。SQLite backup API 只提供单库快照，文件清单也不能证明跨库一致性、因子可解密或恢复后可登录。

从独立可信来源整组部署这四个文件，不从待解密包里选择验证代码：

- `human_snapshot.py`
- `human_snapshot_io.py`
- `human_snapshot_format.py`
- `human_snapshot_core.py`

入口以自身目录句柄固定配套模块；拒绝链接、非普通/多链接/可组写或他人写的工具文件，不从工作目录或 PYTHONPATH 寻找配套模块。包内 launcher、binary、脚本和 unit 全部只当数据复制，从不执行。独立工具 provenance 应记录上述四文件的可信摘要。

## CLI

以下为占位路径，不是生产操作命令：

```sh
python3 -I -B /trusted/human_snapshot.py create \
  --auth-dir /private/frozen/auth \
  --audit-dir /private/frozen/audit \
  --config /private/config/owner.json \
  --release /private/frozen/release \
  --portable-factor /private/export/portable-factor.age \
  --snapshot-id Snapshot-20260920-1 \
  --output /private/new-human
python3 -I -B /trusted/human_snapshot.py verify /private/new-human
```

输出根必须不存在，既有目录/文件/链接不覆盖。snapshot-id 只接受 1–64 个 ASCII 字母、数字或横线。创建输出后所有失败保留 `INCOMPLETE` 和本次已产生内容；在创建根之前拒绝的参数/既有目标不产生或修改目标。verify 拒绝 `INCOMPLETE`、缺失/额外文件和目录、错 mode、摘要或版本，不自动修复。

输入根均为0700，owner为当前 euid（root调用还可接受uid1000）；文件owner与其根一致。绝对祖先可含 `.local`、`.config`，但逐级拒绝链接、组/他人可写，只有非最终祖先的 root-owned sticky 临时目录例外。树内跨设备或跨挂载（含同设备bind mount）拒绝；不同输入根可以分别位于不同挂载。拒绝特殊文件和硬链接。归档路径须NFC、无控制字符、无反斜杠、无空/点/点点段；只允许 `example-vault/**/.gitkeep` 这一有manifest记录的隐藏叶文件（输出位于 `release/example-vault/`），其他隐藏名拒绝。

## 输入与固定输出

| 输入 | 校验与输出 |
|---|---|
| Auth目录 | 仅 `auth.sqlite3` 和成对存在的 `-wal`/`-shm`；0600；快照 user_version=4、quick_check/foreign_key_check通过、store_id形状有效、恰一个v2因子且 enrollment/两回执/confirmed_at存在，TOTP及Recovery pending均空；输出 `auth/auth.sqlite3` |
| Audit目录 | 同样封闭的SQLite文件集；user_version=2、quick_check/foreign_key_check通过；输出 `audit/audit.sqlite3` |
| owner配置 | 0600、重复JSON键拒绝、HostConfig顶层固定必需/可选字段；version=1、seed_ref=`studiolo-human-totp-v2`，auth_directory/audit_directory逐字等于实参；原字节输出 `owner.json` |
| 冻结release | manifest顶层严格 `build/source_commit/assembly/files`，前三项为有界字符串；files条目严格 `{bytes,sha256}`，完整树与其逐项匹配，无额外文件或目录；完整复制至 `release/`（包括原manifest） |
| portable因子 | 0600普通文件、64至65536字节、开头为 `age-encryption.org/v1\n` 二进制age头；原密文字节输出 `portable-factor.age`，不解密、不认证MAC、不验证recipient或对应因子 |

release必须含 `bin/studiolo-local-server`、`launch-human.py`、`studiolo-human.service`、`web/index.html` 与至少一个example-vault文件；清单决定其余完整文件集，不只挑binary。目录全部0700，普通输出文件0600。`release/bin/` 文件须原本0700或0755，输出及外层manifest保留该mode，显式fchmod避免umask077将0755缩成0700。源release内部允许不可组/他人写的普通权限（如0644网页）。

源树先扫描摘要和inode/dev/mode/uid/gid/nlink/size/mtime/ctime；复制和最终发布前反复核对。时间访问atime不作为冻结证据。SQLite先把主库及WAL/SHM复制到本次输出的私有暂存区并核摘要，然后从暂存区通过backup API生成DELETE-journal目标库，绝不让SQLite打开真实源库、改写源SHM。保留原store_id和全部逻辑行，不清会话、恢复码、限流或任何历史。SQL查询不等于Auth的完整密码学/审计绑定验证，须由独立可信Auth工具另验。

单文件≤1GiB，每次树扫描/输入合计≤4GiB，节点≤16384，JSON≤4MiB，路径≤4096字节/64段。backup和SQLite校验各有15秒截止，连接busy timeout为3秒。所有错误仅返回固定失败文本；不输出数据库正文、配置内容、凭据或底层异常。

## 内层manifest schema 1

严格固定字段：`schema_version: 1`、`state: "complete"`、`snapshot_id`、`source_binding`、`source_release_manifest_sha256`、`files`、`directories`。

`source_binding`仅含非秘密引用 `auth_directory/audit_directory/config/release/portable_factor/seed_ref/credential_id`，其中credential_id固定`totp-next`；不把owner身份或任何因子摘要投影到此处。保存的owner.json仍为原配置，不自动改成恢复目标路径。verify只比对包内配置与引用，不访问原主机路径。

`files`映射每个载荷文件到 `{size, sha256, mode}`（mode为整数，如0600对应384、0755对应493）；`directories`映射完整目录集到 `{mode: 448}`。manifest自身不递归记录自身摘要，后续外层schema2应记录本human目录**包括本manifest**的全部文件摘要/大小/mode。创建阶段先写complete清单并完成内部verify，再移除并fsync `INCOMPLETE`；失败时存在complete清单也不代表完成。清单没有签名，包所有者同时重写文件和清单不在防篡改认证承诺内。

## 仅虚构测试

统一入口先把固定16个纯源码文件复制到本次自有 `/tmp` 0700目录，输出源码SHA并运行29项人侧、7项外层schema2、22项原schema1测试，最后复核源码未变并清理副本。不改共享目录权限，不使用真实Auth、age材料或生产配置。仓库共享测试缓存的祖先可能可组写，不应从那里直接调用可信工具加载器。

```sh
bash scripts/test-linux.sh python3 -I -B deploy/agent/offline/run_snapshot_tests.py
```

已有合法私有 Linux 源码树也可直接运行 `python3 -I -B deploy/agent/offline/run_snapshot_tests.py`。三个测试入口和共享 `test_human_snapshot_fixture.py` 必须一起复制。Fixture只构造满足本封装结构检查的最小SQLite表和虚构回执，不声称通过Rust Auth完整校验。覆盖整包、原行/元数据不变、WAL/SHM保全、版本/因子/pending/配置、清单和重复键、跨挂载判定、路径/链接/特殊文件、输出不覆盖、来源/输出并发变化及篡改拒绝。所有CLI子进程有45秒超时。

外层集成可从此可信目录导入 `test_human_snapshot_fixture.Fixture` 与 `MODULE`：构造 `fixture = Fixture(methodName="runTest")`，`fixture.setUp()`，调用 `MODULE["create"](fixture.args)`，使用 `fixture.output`。完成后在finally执行 `fixture.doCleanups()`，只清本fixture临时目录并恢复umask。不要把这一简化fixture称为已登记真实因子或可用恢复点。


## T-094 实际演练与操作器

2026-09-20 的实际结果见 [ACCEPTANCE.md](ACCEPTANCE.md)。[PORTABLE.md](PORTABLE.md)
解释加密流；[Auth 恢复契约](../../../docs/contracts/auth-restore.md)定义撤权与旧运行程序兼容边界。

`full_export.py` 是本批169共同停写操作器，钉定当时的agent/human摘要；失败始终恢复原两个服务。
Type=simple的active先于launcher exec，启动后有界等待正式程序，未放宽摘要校验。
`finalize_export.py` 使用同一已冻结快照重封装r2工具，不再停写；旧工件保留。
`unpack.py` 只将已经认证的流解包到新root私有目录，不执行包内代码；之后必须运行独立可信清单验证器。
`restore_human.py` 是本批r2的固定路径/摘要验收器，只在root独立netns构造新human-work，
从原生解密管道给CLI送入因子，逐表核对允许变化、原因子/审计/时钟/限流保留及同ID幂等。
`cleanup_drill.py` 只清理本批列明目录，先确认原生测试单元退出、节点属主/设备/类型和无进程引用。
这些脚本保留实际批次常量作为证据，不能不经重新核验就对其他批次直接执行。

原 `source-sha256.json` 仅对应58项快照测试检查点；`restore-tool-source-sha256.json` 记录本轮操作器源文件。
快照中的唯一会话已过期，因此实际撤权新增会话数为0；1条旧L3记录和10枚恢复码被清除，epoch2→3。
完整真人重新登录/L3、新agent签发正向、原Cookie/MCP会话重放与RTO/RPO仍未验，不能用本结果替代。
