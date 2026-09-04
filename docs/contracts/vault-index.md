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
}
```

`MarkdownDocument::new(VaultPath, contents)` 要求调用方显式供给 UTF-8 文本；索引没有环境
路径或 Vault 文件句柄。`IndexedDocument` 统一返回 path、标题及来源、归一 frontmatter、
平面/树形标题、出链、来源 profile 与非致命 warnings。

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
