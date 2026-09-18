# T-044 Rust 调用位置证据

在显式白名单文件中查看“文件 → 词法上下文 → 调用语法位置”，用于定位需要人工
审查的源码。这里没有名字解析、类型推导、函数目标边或运行轨迹；`handler()` 也可能
是函数指针或闭包，方法同名不能据此合并成同一目标。

T-045 在同批观测中另加可选的受限词法关联，规则见 [函数词法关联](rust-call-links.md)。
本文件保留 T-044 原调用图契约；旧纯接口不输出关联，不能据显示名称自行连边。

## 能力与读取

`--rust-call-sites '["src/lib.rs","src/query.rs"]'` 接受一个 JSON 字符串数组，
参数正文最多 32 KiB，不可重复，数组非空，必须配合 `--project-repository`。
仅限原本机/旧 Access 只读模式；与 Cargo 开关独立，默认不读任何源码。
库装配使用 `ProjectRepository::with_rust_call_sites(Vec<String>) -> Result<Self, LocalError>`。
启用时只验证路径，不读正文；监听前仍走原根重验。HTTP 不能开启能力或指定文件。

完整白名单先验证，再使用既有固定根 Broker `read_file`。两轮全部路径、正文及
revision 必须相等，请求前后重验根身份，随后才发布；这不是文件系统事务快照。
不跟随 `mod`、`include!`、生成文件、Cargo 配置或网络，不执行项目代码、构建脚本、
Cargo、rustc、宏和插件。API 不提供源码正文、参数内容或原始解析错误。

路径必须是规范可见 NFC 相对 VaultPath，精确 `.rs` 后缀；不规范化点段，不接受
绝对/盘符/隐藏/空段/反斜杠/编码分隔符/控制与不可见格式字符。禁止重复及所有
前缀的大小写碰撞。每路径最多 1024 UTF-8 字节、16 分量；排序后稳定处理。
数量、字节或结构预算超限为 unsupported；路径/语法无效为 invalid；读取缺失、
不安全、预算拒绝或变化为 unavailable。所有失败原子清空本次证据。

## 解析与预算

使用固定 `tree-sitter =0.26.13`、`tree-sitter-rust =0.24.2`；只使用编译进程序的
Rust 语法，不加载动态语法/WASM、查询脚本、日志回调或文件句柄。
每个文件使用新的 Parser/Tree；ERROR、MISSING 或合作式解析取消整批拒绝。

| 项目 | 上限 |
|---|---|
| 显式文件 | 16 |
| 单文件 / 每轮正文 | 64 KiB / 512 KiB（两轮最多 1 MiB） |
| 每文件解析进度回调 | 256 次，第 257 次中止 |
| 整批语法节点 / 单棵树深度 | 32768 / 64（根深度 1） |
| 整批词法上下文（含文件根） / 调用位置 | 256 / 1024 |
| 名称 | 256 UTF-8 字节；不可为空或含控制/不可见格式字符 |

使用迭代 cursor 先验证完整树预算，再迭代提取；不递归遍历、释放自有结构或保留树。
解析步数回调是第三方解析器的合作式预算，输入/输出上限不等于进程级硬时间或堆
配额。它不替代未来生产接入的运行隔离验收。Broker 原目录元信息预算保持原样。

## 语法证据与遗漏边界

文件根、inline module、impl、trait、function/method、闭包、async block、const
（含 const block）、static 分别建立词法上下文。每个调用属于最近上下文，嵌套
定义不会被当成外层函数立即执行；上下文包含关系同样不代表执行顺序。
gen block 暂不支持，出现时整批 unsupported；匿名 `const _` 的名称为 null。
没有函数体的声明不生成函数上下文；所有结果来自未展开、未类型检查的语法树。

调用分四类：

- path：callee 可安全提取为标识路径（保留 `r#`，泛型实参不输出）。
- method：字段式 callee 的方法标识；不输出 receiver 或推断其类型。
- expression：其他 callee（如 `factory()()` 的外层、`(callback)()`）；名称为 null。
- macro：只记录 invocation 的宏标识路径；不进入 token tree 伪造内部调用。

路径中含无法仅按标识提取的限定类型等写法时归 expression，不显示部分伪路径。
宏定义及属性 subtree 不作调用；记录每文件宏定义、未展开 invocation、属性、
条件属性（cfg/cfg_attr）和外部 mod 的数量。cfg 不求值，属性宏不执行。
属性计数仅含显式 `#[...]` / `#![...]`；文档注释保持注释，不换算成隐式 doc 属性。
不推导运算符、解引用、隐式转换、Drop、await 等隐式调用。
空文件/无可提取项仍可 ready；零结果只表示白名单中没有提取到调用位置。

## 冻结的纯接口

```text
RustSourceInput { path: String, content: String }
validate_rust_source_paths(paths: &[String]) -> Result<Vec<String>, RustSourceError>
derive_rust_call_sites(inputs: &[RustSourceInput]) -> Result<RustCallGraph, RustSourceError>
RustSourceError { kind: RustSourceErrorKind::Invalid | Unsupported, message: String }
RustSourceLocation { path: String, line: u32, column: u32 }
RustCallGraph { files: Vec<RustSourceCoverage>, contexts: Vec<RustCallContext>,
                calls: Vec<RustCallSite> }
RustSourceCoverage { path: String, bytes: u32, attributes: u32,
                    conditional_attributes: u32, unexpanded_macros: u32,
                    macro_definitions: u32, external_modules: u32 }
RustCallContext { id: String, parent: Option<String>, kind: String,
                  name: Option<String>, source: RustSourceLocation }
RustCallSite { id: String, context: String, kind: String, name: Option<String>,
               source: RustSourceLocation }
```

上下文 kind：file/module/impl/trait/function/method/closure/async/const/static。
file/impl/closure/async 和匿名 const 的 name 为 null，其余来自原标识（无法命名的
上下文不从表达式拼名称）。文件根来源为第 1 行第 1 列；其余为语法节点起点。
line/column 都一基；column 按 UTF-8 字节，明确显示“字节列”，不冒充显示字符列。

排序：文件按 path；上下文与位置按 path/语法起点字节/终点字节。文件序号从 0 开始；ID 为稳定、不含
源码的 `f{文件序号}:context:{起点}:{终点}` 和 `f{文件序号}:call:{起点}:{终点}`；
文件根用 `f{文件序号}:root`。其他上下文必须有同文件 parent，引用不得悬空或成环。
调用位置指向同文件上下文；name/null 与 kind 一致，expression 只能 null。
各类数量必须与实际提取结果一致，重复文件或不完整输入不会裁剪为成功。

## HTTP 与界面

原 `GET /api/project-operations?project=...&generation=N` 增加可选字段：

```text
rust_call_sites: {
  status: disabled | unbound | unavailable | invalid | unsupported | ready,
  observed_at: string | null, graph: RustCallGraph | null, diagnostics: string[]
}
```

匹配绑定但未启用为 disabled（源码零读取）；无匹配绑定为 unbound。只有 ready
带图和秒时间，其他状态两个字段均 null；保留原代次、认证/来源、查询槽与错误码。
目录、设计声明、Cargo 和源码证据互不改写；不合成同一事务观测时间。
旧响应缺字段正常降级，字段存在但损坏必须报响应错误，不能静默吞掉。

面板显示全部覆盖文件、词法上下文、种类筛选、目标未解析提示与行/字节列。
文件位于某架构声明目录下（规范目录加 `/` 的路径前缀）时列出全部匹配声明与
Vault 原文入口，标注这只是目录包含关系。源码来源仅为仓库文本，不走 Vault 弹窗。
不输出参数、不执行不可信图定义。换项目/代次、重新核验或刷新清理选择；保留原
重新核验按钮焦点；图形异常提供带来源的文本降级。沿用 Apple 浅色与已有 Motion。

## 验证与后续

2026-09-12 本地开发验收完成，基于 T-043 `c1c2171`，分支
`codex/m3-rust-call-sites`。完整符号调用图、真实执行全链路、生产仓库隔离与上线
另验；完整图 2、M2/M3 未完成，未推送、合并或部署。

**2026-09-17 更正**：上段是 T-044 当时的记录。本片提交 `82a620d` 此后已随 PR #5 进入
`origin/main`（`297f126`），用 `git merge-base --is-ancestor` 核实。**但仍未部署、
未整体验收**；完整图 2 与 M2/M3 依旧未完成。

独立虚构示例及启动命令见 [示例说明](../../examples/m3-rust-call-sites/README.md)。

| 验证 | 结果 |
|---|---|
| macOS workbench/local-server | 427 项相关 Rust，另 26 项架构守卫通过 |
| Linux aarch64 Docker | 单构建作业，428 项相关 Rust，另 26 项架构守卫通过 |
| 两平台严格检查 | fmt、workspace all-targets Clippy `-D warnings`、Auth production 特性 dev 构建通过 |
| 前端 | 50 文件 416 项测试、lint、Prettier、tsc 与 Vite 生产构建通过 |

本片新增 50 项 Rust 与 117 项前端契约，覆盖路径/字节/结构/名称预算、两轮变化、
失败原子性、认证/代次/查询不能赋权、全部调用分类与来源、损坏响应拒绝、文本降级。
真实二进制验证 Cargo×Rust 四种开关组合；六类深层/超量语法输入（一元、括号、
宽调用列表、方法链、二元链、未闭合块）拒绝后，原服务进程均可恢复正常读取。
复核后修正了测试集合构造和检查写法；Mac 受影响纯解析 22 项再次通过，不重复累计。

浏览器采用独立虚构预览 `http://127.0.0.1:4327/`，内容视口 1440×1000 和
390×844 均无页面横向溢出。三个文件共 20 上下文/20 调用位置；表达式筛选两项、
闭包只显示 prepare、宏仅显示 invocation、方法来源和空筛选提示均符合原文。
目录包含关联打开 Vault 第 10/13 行；Escape 恢复原按钮焦点。重新核验清理
选择且保留按钮焦点，快照 1→2 刷新清理内部状态；同代刷新保留所选项目由集成
契约覆盖。控制台无 warn/error，35 项夹具文件/目录指纹不变，临时视口已恢复。
旧响应缺字段与图形异常降级由自动化契约验证，本轮未重启旧预览。

短命本机进程另行读取 Studiolo 自身三个显式文件：纯入口 `rust_call_sites.rs`、
`rust_extract.rs` 和采集 `project_rust.rs`，得到 50 上下文/197 调用位置；所有源
字节范围、文件字节数、行和 UTF-8 字节列逐项与磁盘核对，Cargo 能力保持关闭。
该进程已停止。以上不是 release 运行包、服务器原生或生产仓库隔离接入验收。

新增锁包 tree-sitter 0.26.13、tree-sitter-rust 0.24.2、tree-sitter-language 0.1.8、
streaming-iterator 0.1.9；其余复用既有版本。serde_json 的 indexmap 锁引用来自
Tree-sitter 构建依赖；没有新增前端依赖。临时原始日志位于
`/private/tmp/studiolo-t044-*.log`，主要结果保留在本文，不依赖临时日志长期存在。

语法与解析接口依据：[Tree-sitter Rust grammar](https://github.com/tree-sitter/tree-sitter-rust)、
[ParseOptions](https://docs.rs/tree-sitter/0.26.13/tree_sitter/struct.ParseOptions.html)。
