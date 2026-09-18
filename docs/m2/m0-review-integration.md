# M0 main 更新整合与增量验收（2026-09-05）

只读核对 Claude T-027 的 d24e56a194d1a5e9cb297487764caedc1eb0e2d4，基线为
当前开发分支 HEAD 0bd066d。用户随后确认该修复已合并 main 并通过 CI，当前本地
main/origin/main也均为d24e56a。T-029已认领并fresh fetch后实际快进合入，开发HEAD
现为d24e56a；autostash冲突已解决，原审计拆分保留、词法与契约已移植。
591个文件保全核对无丢失或意外变化。T-029已通过macOS747/Linux758全量与两端生产检查。以下保留合入时发现及复现，后附最终处理。

- control-db/task-engine可成对整合结构化错误；公开方法签名不变，DbError增加变体。
- Gateway新增调用等待占位后，等待结束直接重放或执行，缺少取消/fence/expiry复核。
  必须先补真实等待期间撤权契约，不能因新成功等待测试通过就宣称权限边界完整。
- AuditLedger的词法过滤应语义移到现有payload.rs，保留迭代深度/释放、固定编码和
  SQLite实现；不能替换旧巨型lib.rs或整份契约。新词法也影响既有账本/回执的反序列化，
  旧数据若含新增禁字段会拒读，不能只描述为未来append校验。当前Auth自产字段未见冲突。
- 新版schema拒绝的“任何DDL前、零写入”表述强于实现和证据；目前先设置WAL，迁移
  也先CREATE IF NOT EXISTS。测试只确认MAX(version)=99保持，需收紧表述或补真正检查。
  > **2026-09-17 状态更正（T-053 核实、T-047 收尾回写）**：本条所述现象已不成立。
  > `crates/control-db/src/lib.rs:320-321` 现在是先 `db.migrate()?` 再
  > `PRAGMA journal_mode=WAL;`，WAL 已在迁移之后；`docs/contracts/control-db.md`
  > 的对应表述也已改写为与实现一致。**原文按档案纪律保留，不改写历史记录。**
- .gitignore的.claude/与当前*.tsbuildinfo可共存，其余未改契约仍需按最终行为核对。

T-029首个真实复现：原始DELETE日志的新版schema99库被拒绝后已变WAL，WAL对照通过；
2项契约1过1失败，日志在/private/tmp/studiolo-m2-t029-schema-reproduction.log。
初次保全目录/private/tmp/studiolo-m2-t029-premerge-2xa7udf4与autostash保留供恢复，
不以删除保全、覆盖拆分或撤销用户新main来处理冲突。Gateway确定等待复现随后实跑8项中7失败/1正对照通过；修正后9项新契约全部通过。

本核查不是T-027结论的重写，也不是允许直接发布部分M2。Broker未被该更新修改，
canonical单写者和未决幂等已由T-028独立验收；新鲜度/CAS仍后续实现。

## T-029最终处理

审计完整43项覆盖main词法、T012固定编码旧记录正负兼容与超深提前拒绝释放；Gateway25项
验证等待后三种准入在锁外重验、仅自有占位释放及拒绝审计冲突不重复记账；ControlDb20项
验证持久配置/DDL前拒绝与事务内再核，保留旧迁移与WAL正常初始化。所有新增模块≤251行。
macOS747/Linux758、两端规范/生产/CLI/23启动正反检查通过，详见[完整记录](README.md)。
未新增业务依赖，web沿用T025证据。原591文件保全与autostash未删，未新提交推送或部署。
