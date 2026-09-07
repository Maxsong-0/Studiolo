# M1 T-007 · 搜索与文档导航

本片接续 T-006，只扩展当前只读快照的查询与导航。沿用原分支，不接入真实 Vault，
不提供编辑、通用目录授权、外部搜索或模型调用。当前实施与验收状态见 [M1 总览](README.md)。

## 查询 Interface

`GET /api/search?q=<字面查询>&mode=full_text|path&project=<可选项目ID>&generation=<代次>`

响应：`{generation,hits:[{path,title,line,snippet,kind}],truncated}`。

- 全文模式同时匹配完整文件路径和正文，采用不区分大小写的单行字面匹配，不执行用户正则。
  中文子串和含标点的连续查询保持原义；不跨换行，不做分词、拼音或语义搜索。
- 路径模式供快速切换器复用同一 Interface、同一文档集。用普通空格分隔多个路径片段，
  各片段都须出现在完整路径中；不是通配符 glob 或拼写纠错。结果始终展示完整路径。
- `project` 必须精确对应快照中的项目 ID；未知项目返回 400，不能悄悄扩大到全库。
  不传则查询已读入快照的全部 Markdown（包括非项目目录），不搜索附件二进制。
- 查询最多 256 个 Unicode 字符且最多 1024 个 UTF-8 字节，不含控制字符。
  全文空查询为空结果；路径空查询为首批文档，供空切换器与文档列表使用。
- 最多返回 100 条，只有确实存在后续命中才置 `truncated`。截断时应收窄项目或增加查询。
  `kind=path` 的第 1 行是打开文件入口，不宣称正文该行命中；`kind=content` 指向真实命中行。
  摘要和标题各最多 320 个字符，摘要覆盖首个命中附近，纯文本转义输出；完整路径不裁剪。
- 过期代次返回 409；2 个共享搜索/文档工作槽已满时返回 429。后台工作真正结束才释放槽，
  客户端取消请求不能绕过上限。工作只持有当前不可变 generation，不阻塞 HTTP 运行线程。

## 为什么使用 ripgrep 库而非再次读取磁盘

功能清单要求 ripgrep 全扫而非默认中文不可用的分词检索。本片锁定 ripgrep 的
[grep-regex 0.1.14](https://docs.rs/grep-regex/latest/grep_regex/struct.RegexMatcherBuilder.html)、
[grep-searcher 0.1.17](https://docs.rs/crate/grep-searcher/0.1.17) 与
[grep-matcher 0.1.9](https://docs.rs/grep-matcher/0.1.9/grep_matcher/)。
只对 Broker 已读取并校验的内存内容执行 `search_slice`，不调用系统 rg、不重新遍历路径。
这样搜索摘要、原文与导航目标属于同一代，路径安全仍集中在 Broker。

## 文档导航 Interface

`GET /api/document?path=<逻辑路径>&generation=<代次>` 保留 `{path,title,content}`，
增加 `navigation:{outline,outgoing,backlinks,truncated}`。不引入第二次导航数据请求。

- 大纲：`{title,level,source:{path,line}}`，沿用 VaultIndex 标题与源行。
- 出链：`{label,raw,source,status,targets:[{path,line,title}]}`，页面解析仍由 VaultIndex 决定。
  状态为 `resolved`、`ambiguous`、`missing`、`missing_heading`、`ambiguous_heading`。
- 标题片段先匹配标题原文，再匹配派生 anchor；重复原文给行号候选，缺标题不跳第 1 行冒充成功。
  歧义页面必须用户选择完整路径，不因为候选顺序或筛选后只剩一项就自动替用户决策。
- 反链：`{title,source,context}`，指向引用文档的真实源行。以已有页级解析为准，
  即使引用中的标题缺失，也可能作为指向该页的反链存在；不能把反链存在解释为标题有效。
- 大纲、出链、反链各最多 200，单条目标候选最多 50；显示文本限幅，完整路径不裁剪。
  超限明示，不伪造“没有更多”。原文保持第一片的完整纯文本或大文件窗口降级。
- 页面内出链/反链/大纲均用内部按钮导航，保留同代次的后退历史；刷新会关闭旧来源。
  笔记中的 HTML/脚本不执行，缺页不会自动创建文件，嵌入不执行资源加载。

当前扫描器并非完整 CommonMark 解析或渲染器；本片修复围栏标记/长度、等长反引号和
转义伪链接的已知回归，不宣称覆盖所有多行行内代码或 HTML 注释语义。

## 界面与验收范围

沿用第一片三导航与白底、灰侧栏、青绿设计系统。文档页新增全文搜索和项目选择，
全局新增“快速切换”入口与 ⌘K/CtrlK；原文新增后退、大纲、出链和反链。
这些是本片功能必要扩展，不把第一片概念图当成已经包含新增界面的证明。

验收使用 `examples/m1-vault/` 中虚构的阅读体验、同名配色方案和搜索说明：
“连续阅读”跨项目命中 → 纸页收窄 → 原文命中行 → 字体与间距标题 → 反链/后退；
另验证代码围栏伪链接、缺页/缺标题、同名歧义、路径片段快速切换与旧代请求拒绝。

文件树（含纯附件目录）、变更流、规则漂移、持久索引和 watcher 仍未纳入本片。
文件树后来由[T-010](file-tree.md)完成；本节保留T-007当时的验收范围。
