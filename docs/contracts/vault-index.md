# VaultIndex Interface 契约

> Module：`studiolo-vault-index`。它把调用方提供的 Markdown/YAML 收口成所有派生视图共用
> 的读侧语义。M0 只做可重建内存索引，不打开或写入 canonical Vault。

## 1. M0 类型化 Interface

```rust
pub trait VaultIndex: Send + Sync {
    fn rebuild(
        &mut self,
        documents: &[MarkdownDocument],
    ) -> Result<IndexGeneration, IndexError>;
    fn document(&self, path: &VaultPath) -> Result<Option<IndexedDocument>, IndexError>;
    fn heading_tree(&self, path: &VaultPath) -> Result<Vec<HeadingNode>, IndexError>;
    fn resolve_link(
        &self,
        from: &VaultPath,
        target: &WikilinkTarget,
    ) -> Result<LinkResolution, IndexError>;
    fn backlinks(&self, to: &VaultPath) -> Result<Vec<Backlink>, IndexError>;
    fn link_health(&self) -> Result<LinkHealth, IndexError>;
    fn source_profile(&self, path: &VaultPath) -> Result<SourceProfile, IndexError>;
    fn generation(&self) -> IndexGeneration;
    fn diagnostics(&self) -> Vec<IndexDiagnostic>;
}
```

`MarkdownDocument::new(VaultPath, contents)` 要求调用方显式供给 UTF-8 文本；索引没有环境
路径或 Vault 文件句柄。读不进来的笔记走 `MarkdownDocument::unavailable(VaultPath,
DocumentUnavailable)`（见 §8）。`IndexedDocument` 统一返回 path、标题及来源、归一
frontmatter、平面/树形标题、出链、来源 profile、非致命 warnings 与 `unavailable`。

## 2. rebuild 与 generation

- `rebuild` 先构造完整替代 generation，再一次替换旧派生状态；同一批出现重复 path 返回
  `IndexError::DuplicatePath(path)`，旧 generation 保持可用。
- 每次成功 rebuild 将进程内 generation 单调加一，并返回
  `IndexGeneration { generation, documents, links }`；它不是跨重启 cursor 或权威 revision。
- `document` 对未索引路径返回 `Ok(None)`；要求既有源文档的 heading/resolve/source 查询返回
  `IndexError::NotIndexed(path)`。
- 标题选择顺序固定为归一 frontmatter `title` → 第一条 H1 → 去掉 `.md` 的文件名。
- 解析 warning 属于该文档的新结果，不能沿用旧解析值冒充当前 generation。

M0 不提供泛化 `apply(IndexChange)` / `query(IndexQuery)`、增量 remove、watcher gap/freshness、
持久索引或搜索；这些能力有真实 M1 消费者后再做兼容评估。

## 3. Wikilink、反链与健康度

`WikilinkTarget` 解析 `[[target#heading|alias]]` 与 `![[...]]`，保留 raw、target、heading、
alias 和 embed；缺分隔符或空目标分别为 `WikilinkParseError::MissingDelimiters | Empty`。
索引跳过 fenced code block 与行内 code 中的伪链接，并保留一基行/Unicode 字符列和上下文。

解析顺序：

1. 相对源文档目录精确路径；
2. Vault 根精确路径；
3. 以文件名、展示标题和 frontmatter aliases 为候选，从源目录开始逐级向根寻找第一个唯一
   命中；
4. Vault 范围仍有多个候选为 `Ambiguous { candidates }`，零候选为 `Dangling`。

成功结果携带 `ResolutionRule::{ExactRelative, VaultRootPath, NearestUnique { ancestor_depth }}`。
候选按 `VaultPath` 稳定排序；歧义/悬空是 `LinkResolution`，不是 `IndexError`，不得猜选。
`backlinks` 只含成功解析到目标的入链；`link_health` 汇总 dangling、ambiguous 与按规则计数。

解析走预聚合的 wikilink 索引，不再逐条链接遍历全库：每篇笔记贡献一个「整条逻辑路径」键
和一组「名称」键（文件名去 `.md`、展示标题、frontmatter `aliases`），两侧都经同一个折叠
函数（去首尾空白 + Unicode 小写）产生比较键。单条链接的解析代价只与同键候选数和源文档的
目录层数有关，与 Vault 规模无关；就近唯一等价于「取候选首次落入的最小祖先层级，该层唯一
即命中，该层已有多个则各层都不可能唯一，直接按 Vault 全域报歧义」。

索引是可丢弃的派生物，内容态仍然只在文件里。每代 rebuild 只推进差异：本轮消失的文档立即
退出候选集合，比较键未变的文档不重新索引。**逐代增量维护的结果必须与只拿本代文件从零重建
的结果完全一致**，索引丢失时由当前文档集合完整重建即可恢复。

`VaultPath` 只接受 NFC，因此路径、文件名与标题侧的比较键天然是 NFC；正文里手写成分解拼写
的链接文本目前折不成同一个键，会判为 `Dangling`（已登记缺口，修法见 `link_index.rs` 中
`comparison_key` 的说明）。

## 4. frontmatter 归一

权威别名来自 vault `01-框架/frontmatter键别名表.md`，M0 至少覆盖：

```text
状态 -> status       日期 -> date       项目 -> project       类型 -> type
编号 -> id           严重度/严重级 -> severity
更新/更新日期 -> updated               创建 -> created
发现日期 -> discovered_at              修复日期 -> fixed_at
```

前八行公开在 `FRONTMATTER_KEY_ALIASES`，BUG 日期另公开在
`FRONTMATTER_SPECIAL_KEY_ALIASES`，不得折叠到 created/updated。同组规范键与别名并存时
规范键优先，全部原拼写和值仍保存在 `NormalizedFrontmatter.entries/raw`，并产生
`FrontmatterWarning::DuplicateAliasGroup`。其他键原样透传；因此英文 `title`、`aliases`、
`tags`、`source`、`source_type` 可被相应读侧逻辑消费，但中文便利拼写不会被暗中归一。
畸形 YAML 与未闭合分隔符分别产生 `MalformedYaml` / `Unclosed`，不触发 Vault 写回。

`NormalizedFrontmatter` 公开 raw、canonical map、逐项 entries、alias hits 与 warnings；每个
entry 保留 `raw_key`、`raw_value`、`canonical_key` 和类型化 value。

`status` 去掉行首状态表情与 `**`，在首个 `（`、`(`、`·`、`#`、`+` 处截断后做前缀匹配。
闭合集是 `Adopted | Fixed | Done | InProgress | Pending | Approved | Superseded |
Deprecated | Draft | Unknown(raw)`；`done-pending-restart` 和“代码完成…”必须归 Pending，
权威表未列的英文 `approved` / `pending` 必须为 Unknown。`type` 只把 `follow-up`、
`followup-suggestion(s)`、`follow-up-suggestion(s)` 与“后续建议”折叠为 `follow-up`，其余
自由文本原样保留。

## 5. 来源 profile 不是授权

`SourceProfile` 返回显示用的 `provenance`、声明的 source/source_type、可识别的声明分类及
`safe_default`。provenance 为 `HumanUnclassified | StudioloGenerated | AgentArtifact |
ExternalDeclared`；没有 manifest/policy 证明时 safe default 固定为 `Restricted`。

该 DTO 故意不同于 `DataLabels`。frontmatter、路径或 source_type 只能影响派生视图，不能
授予模型外发、同步、任务、审批或写入权限。

## 6. 错误、替身与验收

M0 的 `IndexError` 只有 `DuplicatePath(VaultPath)` 与 `NotIndexed(VaultPath)`；解析问题以
warning/结果表达，不用字符串错误控制流程。`InMemoryVaultIndex` 是 M0 Adapter，解析器、
归一、link resolution 和 source profile 就是共享生产逻辑，不另写“总成功” fake。

契约测试至少覆盖：公开 trait 全部方法、重复 path 的原子 rebuild、generation 计数、标题
fallback、Unicode 标题树、权威中英文别名/规范键优先/raw/未知键、全部 status 闭合集、
相对/根/就近/歧义/悬空链接、只对 resolved link 建反链、health 计数、四种 source profile
及 Restricted 默认，并以源码扫描证明 crate 没有 canonical 文件写 API。

T-054 另加三类：逐代增量索引与从零重建的公开查询结果逐项一致（含删除、改名、别名反复
变更与空代次）；降级诊断不影响正常项且渲染文案不含物理前缀；单条链接解析的候选访问量在
500 / 4000 / 16000 篇三种库规模下逐例相等。

## 7. M3 单篇解析缓存（T-039）

`InMemoryVaultIndex::rebuild_cached` 接收本次 `MarkdownDocument` 和可选 `ParseCache`，
返回代次、下一批缓存与 `ParseCacheStats { reused, parsed }`。原 `rebuild` 保持兼容。
匹配路径、精确内容摘要与长度后只复用单篇解析；全局链接、反链和来源 profile 均重新派生。
`ParseCache::encode/decode` 是纯有界编码，不读取或写入任何路径；正文始终由本次输入提供。
格式/解析版本不匹配应丢弃缓存；预算超限只停止持久化，不阻止有效文档构建内存视图。
缓存信任边界、私有 SQLite 和完整副本验证见 [M3 持久派生索引](../m3/persistent-index.md)。

## 8. 降级诊断（T-054）

符号链接、超大笔记与读不出 UTF-8 的笔记**不得让整批 rebuild 失败**。调用方用
`MarkdownDocument::unavailable(path, reason)` 把它们登记成逻辑路径，`reason` 取自闭合枚举
`DocumentUnavailable::{Symlink, TooLarge, Unreadable}`。

- 降级笔记仍是本代次里的一条 `IndexedDocument`，`unavailable` 为 `Some(reason)`；正文缺席，
  因此没有标题树、出链或 frontmatter warning，标题只能退回文件名。
- 它仍然占住自己的逻辑名，指向它的入链照常建立——降级只说明本轮读不到正文，不代表这条
  路径不存在。
- `diagnostics()` 返回本代次全部降级项，按逻辑路径稳定排序；诊断属于当代次派生状态，
  上一代的结论绝不沿用。笔记恢复可读后诊断随之消失。
- `DocumentUnavailable` 故意是不带自由字符串的闭合枚举：诊断会渲染给用户，携带 `io::Error`
  文案就可能把 Vault 之外的物理路径漏出去。`IndexDiagnostic` 只携带 `VaultPath`，其
  `Display` 输出只含逻辑路径与固定原因文案。
- `rebuild_cached` 同样透传 `unavailable`。它是**本轮输入事实**而不是解析结果，因此不进
  `ParseCache`，上一轮的缓存行不能把本轮的降级盖过去。
