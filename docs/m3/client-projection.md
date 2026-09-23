# 客户端资料投影（T-038）

依据 Vault ADR-015。将已授权的规则和 Markdown skills 参考导出成独立的 Claude/Codex
资料目录，支持离线读取、同步预览、显式更新和漂移检查。所有线上导出读取都经过
T-037 HTTP 外壳的同一类型身份、白名单、完整副本验证和审计，不直接读取本机 Vault。

2026-09-11，CLI 与参考文档已随 T-040 源码包 `m3-t040-20260910-read01` 发布，
依赖的 T-037 鉴权 HTTP 供给已上线并通过公网复验，见 [发布记录](read-supply-release.md)。
真实 Claude/Codex 资料工作区尚未安装，既有项目配置未修改，未新建客户端后台同步任务。

## 输出与来源

每次生成只选择一个客户端格式；实际读取类型及实例来自握手，记录在 manifest，
不因为选择了 Claude/Codex 模板而改写身份。应为实际接入方配置对应的独立类型凭据。

| 输出 | 用途 |
|---|---|
| `CLAUDE.md`、`.mcp.json`、`.claude/skills/studiolo-context/SKILL.md` | Claude 项目指令、HTTP MCP 配置及参考技能入口 |
| `AGENTS.md`、`.codex/config.toml`、`.agents/skills/studiolo-context/SKILL.md` | Codex 对应配置；不会与 Claude 文件一起生成 |
| `CONTEXT.md`、`sources/` | 项目标识、来源摘要、离线限制及保持原字节/相对目录的选定资料 |
| `manifest.json` | schema、来源 URL/代次/摘要/确认时间、导出时间、读取身份、源 revision 及逐文件 SHA256 |

以下是缺省的 vault 模式；规则与 skills 改从策略平面读取见
[从策略平面 effective 导出](#从策略平面-effective-导出)。
配置只允许显式规则 Markdown（`_规则/` 内，或所选项目的 `CLAUDE.md`），
以及 `_规则/skills/<名称>` 下完整的 Markdown 技能参考目录。每个技能须有 `SKILL.md`；
发现附件、脚本或其他非 Markdown 文件会拒绝整批，避免生成缺失执行资源的技能包。
静态技能资源 `references/index.md` 可以导出；项目 INDEX、日志、TASKS、协调目录和
原始握手路牌不导出。规则引用的其他文件只有被显式选中时才会随包提供。

原始 skills 作为按需读取的参考，原生目录只注册 `studiolo-context` 入口。
这不是 M4 的技能发布、生效版本审批、客户端强制绑定或黑板同步。

## 生成、预览与应用

需要 macOS/Linux、Python 3 标准库，以及可用的托管副本 HTTP 入口。
全部命令路径使用绝对路径；macOS 临时目录使用 `/private/tmp`，避免 `/tmp` 符号链接。
文件和目标目录不得经符号链接定位。凭据从已经配置好的安全环境变量读取，不能写入 JSON。

以 [Claude 示例配置](client-projection.example.json) 为起点，确认选定资料和服务地址：

```json
{
  "schema_version": 1,
  "client": "claude",
  "project": "项目/Studiolo",
  "source_url": "https://studiolo-agent.maxsong.me/read",
  "token_env": "STUDIOLO_CLAUDE_TOKEN",
  "rules": ["_规则/通用工作规则.md", "项目/Studiolo/CLAUDE.md"],
  "skills": []
}
```

Codex 改 `client` 为 `codex`，并使用该接入方的 `token_env`。`source_url` 只能是
HTTPS `/read`；隔离本机测试另允许 `http://127.0.0.1:<端口>/read` 或 IPv6 loopback。
不接受 URL 凭据、查询参数、片段、HTTP 非 loopback、重定向或环境代理。

下列路径是可替换的示例。`--out` 必须尚不存在；其父目录必须已存在。
`--target` 是专用资料根，可以不存在或为空的 0700 目录，不应填写现有工作仓库。

```sh
python3 -B scripts/sync-agent-client.py generate \
  --config /absolute/path/client.json --out /private/tmp/studiolo-client-review-01
python3 -B scripts/sync-agent-client.py plan \
  --bundle /private/tmp/studiolo-client-review-01 --target /absolute/path/studiolo-client
python3 -B scripts/sync-agent-client.py apply \
  --bundle /private/tmp/studiolo-client-review-01 --target /absolute/path/studiolo-client
python3 -B scripts/sync-agent-client.py check --target /absolute/path/studiolo-client
python3 -B scripts/sync-agent-client.py check --target /absolute/path/studiolo-client --online
```

`plan` 完全只读，列出 create/changed/unchanged/conflict、涉及文件与旧/新摘要；
正文在 `--bundle` 指定的私有目录中供审阅，不自动打印到日志。`apply` 重新校验后才发布。
实际资料项目入口为 **`<target>/current`**；用客户端打开该目录。
它是独立资料工作区，不会把配置自动注入用户已有代码项目。
如需把资料嵌入已有项目，应另行审阅指令与配置的整合差异；本 CLI 不覆盖或合并用户配置。

## 更新、漂移和故障

目标结构为 `store.json`、`projection.lock`、`releases/<bundle_id>/` 和 `current`。
目录 0700、文件 0600。每批先完整写入未激活的 release，校验所有文件及目录，
然后一次原子替换 `current` 链接。保留旧批次；运行中的客户端应保持本次启动所用的
具体 release 路径，更新后重新打开资料项目，避免跨次读取主动跟随新指针。

- 更新前重新 `generate` 到新目录，再 `plan`、`apply`。同来源、内容及配置的重复导出
  按 `projection_key` 幂等，保留原批次与首次导出时间，不把安装时间伪装成刚确认来源。
- `bundle_id` 校验完整 manifest；`projection_key` 排除实例和时间变化。源 revision
  是 Vault/路径域分离版本标识，不等于正文 SHA256；manifest 同时记录两者。
- 本地逐文件长度、摘要、额外/缺失条目、权限或链接发生变化：`local_modified` /
  `conflict`，拒绝覆盖且没有 force。用户资料与损坏证据保留，需先审阅处理。
- 离线 `check` 的 `ok` 只证明本地完整性，`source_status=unknown_offline`。
  `--online` 重新完成鉴权导出，返回 `unchanged`、`source_changed` 或 `unavailable`；
  来源不可达、陈旧、同步失败、审计不可用或凭据失效都不能标为最新。
- 验证/暂存写入失败不会切换旧指针。残留的半成品 release 不自动删除；重新生成到
  一个新 `--out` 后再应用。相同损坏 bundle ID 不能被直接覆盖重试。
  若首次安装就留下 release 而没有 current，工具不能区分中断和用户删除指针，会保守
  拒绝接管；保留该目录，改用新的专用 `--target`。客户端若在 release 中增加本地配置
  或其他文件，也会触发漂移；先审阅这些改动，再选择新的资料根，工具不会搬移用户配置。
- 切换后若目录 fsync 失败，返回 `activated_durability_unknown`：当前指针已经更新，
  持久化结果不确定，不宣称回滚；检查实际 `current` 并保留证据。离线 `check` 只能确认
  当时可读的完整性，不能替代重启后的持久性验证。
- 同时应用用排他锁拒绝竞争；拒绝链接越界、根重绑定、硬链接以及未知非空目标。
  本地校验是误改/漂移检测，不是具有独立信任根的签名验证。

退出码：0 成功（含离线完整性通过）；1 导出/校验/IO失败（含 apply 重验拒绝）；
2 plan/check 报告冲突、本地或来源漂移；
3 在线来源不可用或发布持久化状态待确认。失败输出固定类别，不打印凭据、会话或资料正文。
首版最多 128 篇源文、单篇 4 MiB、正文合计 16 MiB；读取磁盘包按清单声明大小立即校验。
HTTP 每次超时 15 秒，单次导出接收总量有上限；只接受 ready 且新鲜的托管副本。
所有响应须同 generation/digest/max_age，确认时间可单调推进；包保留首次较保守的确认时间。
最后再握手复核，混批或中途变化整批拒绝，不发布半份资料。

离线只供规则和技能参考，不能据此认领任务、更新实时协调黑板或开展 A2A。
线上导出读取可审计，本地后续读取不经过服务器；已经导出的字节无法被在线吊销即时收回。

## 从策略平面 effective 导出

T-111（2026-09-22）。配置加可选键 `rules_source`：缺省或 `vault` 即上文行为，manifest 与输出
逐字节不变；`policy` 时规则与 skills 正文改取自 agent 的 `GET /read/policy`（策略平面现行
effective），不再读 Vault 文件。起点见 [策略模式示例配置](client-projection.policy.example.json)，
`schema_version` 仍为 1。

- 名字：`rules` 是规则母本 subject（单段 `.md` 文件名，≤128 字符，不含 `/`、`..`、控制字符，
  大小写不敏感去重），`skills` 是 skill 名（小写字母、数字、连字符，≤64）。未知取值或任一名字
  不合规，整份配置拒绝。
- 读取：握手 → `kind=rules` 清单 → 逐项 `kind=rules&subject=…` → 选了 skills 时再取 `kind=skills`
  清单与逐项 → 末次握手。每次读取都经能力集授权并审计为 `policy_read`
  （路径 `policy://rules/<subject>`、revision `v<版本>`）。格式唯一来源是
  `crates/agent-server/src/server.rs` 的 `text_policy_read`。
- 一致性：两份清单与每份正文首行 `# 策略版本 <12 位>` 必须相同，否则视为导出途中 effective
  变化，整批拒绝；所选名字不在清单直接拒绝，不发正文请求；逐项 `@v<版本>` 须与清单相同；
  规则正文 SHA256 须等于服务端声明，skill 按服务端规范 JSON（serde 字段序）复算后须相同。
- 输出：规则母本原字节落 `sources/policy/rules/<subject>`；skill 落
  `sources/policy/skills/<名>/SKILL.md`，内容是服务端返回的 `摘要：…`、`触发：…`、空行加
  Markdown 正文，原样不改写。入口文件标明策略版本，不写 Vault generation。
- manifest：顶层增 `rules_source: "policy"` 与 `policy_effective`（12 位）；每项以
  `policy_version`、`policy_sha12` 取代 vault 的 `revision`。规则的 `policy_sha12` 必须等于
  正文 SHA256 前 12 位；skill 的是规范 JSON 的摘要。
- 漂移：`projection_key` 以 `policy_effective` 取代 Vault generation/摘要，副本换代不算来源变化。
  策略版本或逐项版本/摘要一变，`plan` 为 `changed`，`check --online` 为 `source_changed`
  （退出码 2）；两者都附 `policy` 字段列出新旧策略版本与变化项。effective 覆盖全部规则、
  skills 与能力集，未选中的条目变化同样改变策略版本；此时 `items` 为空，表示所选资料本身未变。

限制：导出仍要求握手时托管副本 ready 且新鲜（manifest 照旧记录确认时间）。skill 的摘要或触发
含换行、触发含 ` | ` 时，文本协议无法唯一还原，复算对不上即整批拒绝；skills 清单里任一摘要
含换行也会让计数不符而拒绝。契约：schema 12、抓取 25、存储 21 项全部通过；另用本仓库构建的
agent 加隔离策略平面（`policy-init`、`policy-bootstrap`）做过一次真实联调：含特殊字符的 skill
摘要与服务端逐位一致，读取全部入审计，新增规则后 `check --online` 报出策略版本变化；
该联调脚本是临时产物，未入库。

## 客户端格式与验收边界

本机已核对 Codex CLI 0.134.0、Claude Code 2.1.236 的帮助及官方基础格式：

- Claude [`CLAUDE.md` import](https://code.claude.com/docs/en/memory)、
  [skills](https://code.claude.com/docs/en/skills) 和
  [HTTP MCP 环境变量头部](https://code.claude.com/docs/en/mcp)。
- Codex [AGENTS.md](https://developers.openai.com/codex/guides/agents-md)、
  [skills](https://developers.openai.com/codex/skills) 和
  [MCP 配置](https://developers.openai.com/codex/mcp)。

Claude 入口使用 `@CONTEXT.md`；Codex 明确要求先读取 CONTEXT，普通链接不冒充已加载正文。
MCP 配置仅含 `${ENV}` 或 `bearer_token_env_var` 变量名；Claude 项目 MCP 需要客户端确认，
Codex 项目配置需要受信任项目。技能元数据被发现不代表所有技能正文都已加载。
真实客户端的信任、环境继承、配置发现、模型读取和 MCP 连接仍需独立实操验收。

本片用虚构源、隔离身份库和本机 HTTP 服务验证。测试入口：

```sh
python3 -B scripts/test-client-projection-schema.py
python3 -B scripts/test-client-projection-fetch.py
python3 -B scripts/test-client-projection-store.py
python3 -B scripts/check-client-projection-pipeline.py target/debug/studiolo-agent
```

2026-09-10最终验收：macOS与Linux Docker aarch64均通过45项新增测试（schema/磁盘8、
抓取/模板19、存储18）、25项架构守卫，共70项；两端真实托管副本→HTTP→五份源文→
两类客户端安装管线通过，审计索引关联权威账本事件、混批/凭据/吊销/审计故障、来源更新、
本地修改和离线保留均实际验过。Linux构建了与当前Rust源码一致的测试服务。
CI已加入全部新测试与管线；本片没有修改Rust生产源码或新增依赖，不重复无关的前端验收。

初次真实联调发现托管路牌新鲜度后有额外空行，解析已修正并补契约；存储复审修正了缺失目标
误报、磁盘读取预算检查过晚、静态技能index误拒绝及删除current漏报。所有修正后重跑通过。
证据：`/private/tmp/studiolo-t038-macos-unit.log`、`studiolo-t038-macos-pipeline.log`、
`studiolo-t038-architecture.log`、`studiolo-t038-linux-final.log`；这些属于开发阶段证据。

2026-09-11，T-038 已随 T-040 合并 main `cbfb5df`，CLI 及测试随源码包发布；
服务器原生投影契约和虚构资料真实管线通过，供给服务的系统与公网 HTTP 验收通过，
详见 [T-040 发布记录](read-supply-release.md)。真实客户端资料安装、信任与配置发现、
启动后的模型读取及 MCP 连接仍需独立实操验收，不能用虚构管线替代。
