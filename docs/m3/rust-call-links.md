# T-045 Rust 同文件函数词法关联

在 T-044 显式源码证据中，从调用位置查看同文件自由函数定义，并反查引用。
结果为受限词法关联，不是编译器名字解析、类型分派、执行轨迹或完整调用图。
不新增 CLI 开关、文件发现、Cargo 权限、正文端点或代码执行能力。

## 输入与原子性

沿用 [T-044 白名单、读取和预算](rust-call-sites.md)。两轮相同正文、路径和
revision 捕获后，仅读取该不可变输入；每文件一次解析，同一已验证 CST 同时提取
原调用图、作用域与关联。最后固定根身份重验成功才一起发布。任何分析或捕获失败
均使本次 graph、links、observed_at 为 null，不发布部分边或上一轮结果。

分析沿用节点 32768、深度 64、上下文 256、调用 1024 的整批上限。
作用域/阻断项仅由已验证节点派生且有界，使用迭代遍历；不添加递归分析、动态语法、
语义引擎、宏展开、构建脚本或网络。合作式解析预算不等于进程级时间/堆隔离。

## 受支持规则

先收集完整文件作用域、定义和阻断信息，再解析所有调用，顺序不决定是否安全关联。
只接受 CST 中直接裸 identifier callee；generic_function、路径、方法、表达式及
宏均未解析。只指向最近允许作用域中唯一、带函数体且无泛型参数的自由函数 context。
递归和同一 block 中后声明的函数可关联；导入和其他文件永远不作为目标。

source_file、inline module、block、函数/闭包及模式范围建立保守词法作用域。
模块是查找硬边界，不从子模块向父模块找裸名称。impl/trait/const/static 任意
祖先中的调用不纳入本片。caller 必须有自由 function 祖先；其内闭包和 async
可查找祖先定义。文件级 type、enum discriminant 等无自由函数祖先的调用为 context。

标识比较为去 r# 后 NFC，区分大小写；显示名保留 T-044 原文。
同名参数、let/if-let/while-let/for/match 模式、泛型名和其他具名非函数 item
保守阻断向外查找；忽略精确声明顺序，可以少连，不能猜测。普通 use 子树全部标识
可保守作名字阻断（含原名和别名），不因任意 use 使整个文件不透明。
同名多个候选保留歧义；目标带泛型不纳入。没有函数体的声明也不能忽略后向外找。

glob use、extern 声明、显式属性和宏 invocation 使所在词法作用域及其子孙不透明，
包括源位置在它之前的调用。前置属性按其 sibling 所属 scope 处理；宏定义与文档
注释不产生阻断项，不进入其正文。无法可靠覆盖的绑定语法可使所属范围不透明。

## 冻结的纯接口与 HTTP

```text
derive_rust_call_analysis(&[RustSourceInput]) -> Result<RustCallAnalysis, RustSourceError>
RustCallAnalysis { graph: RustCallGraph, links: RustCallLinks }
RustCallLinks { version: u32, batch_id: String, associations: Vec<RustCallAssociation> }
RustCallAssociation { call_id: String, status: String,
                      target: Option<String>, reason: Option<String> }
```

旧 derive_rust_call_sites 返回类型和成功/失败语义保持原样；共享单次解析流程，
不通过第二遍解析或二次 IO 补关联。links.version 固定 1。
batch_id 为 64 个小写十六进制字符的 SHA-256，输入为：

```text
b"studiolo-rust-call-links-v1\0"
+ 文件数(u64 big endian)
+ 按规范路径排序逐文件拼接：
  路径UTF-8字节数(u64 big endian) + 路径UTF-8
  + 正文字节数(u64 big endian) + 完整正文UTF-8
```

批标识说明输入内容身份，不是文件系统快照、权限凭据或新鲜度证明。
它不同于 Vault generation，也不与独立 Cargo 观测绑定。
graph 和 links 由同一 RustCallAnalysis 生成；前端没有正文，不能独立重算内容哈希。

每个 graph.calls 恰有一项 association，顺序与 calls 相同，不允许重复、遗漏、
悬空或跨文件引用。linked 必须有 target 且 reason=null；target 是同文件 kind=function
的 context ID。unresolved 必须 target=null、reason 属于下列闭集：

| reason | 说明 |
|---|---|
| callee_form | 非直接裸标识调用形式（含泛型 callee） |
| context | 调用不在本片允许的上下文中 |
| opaque_scope | 属性、宏、glob、extern 或未覆盖语法使作用域不透明 |
| shadowed_name | 同名绑定、导入或非函数声明阻断 |
| ambiguous_name | 最近作用域有多个同名函数候选 |
| generic_target | 最近唯一函数定义带泛型参数 |
| not_found | 当前允许的同文件范围未找到函数定义 |

优先检查 callee_form → context → opaque_scope，再从最近 scope 开始查找；
同一 scope 的同名 blocker 优先于候选/歧义/泛型。未解析不附猜测目标。

原 rust_call_sites 对象增加 links: RustCallLinks | null；ready 时 graph 和 links
同时非 null，其他状态 links=null。旧服务缺 links 字段时前端正常显示 T-044；
字段存在但状态、版本、批标识或引用损坏时拒绝整个响应，不静默吞掉。

## 界面和验证

保留文件、上下文和种类筛选；关联项显示“词法关联”，可查看定义位置及该定义的
全部同批引用，引用提供调用名称和位置。零引用/未解析原因明确可见；仓库位置仅是
来源文本，不能走 Vault 笔记读取端点。文本降级也保留关联与未解析说明。
换项目、代次、文件和重新核验清理相应旧选择；沿用 Apple 浅色和已有 Motion。

验收覆盖正向和反向引用、递归/嵌套/模块边界、各类同名绑定、导入/属性/宏/extern、
原始标识及 Unicode NFC、泛型、排序、同长改名换批、读取/root 变化与失败后恢复。
需要虚构示例和真实白名单助手函数正向证据，不以全部未解析作为能力验收。

完整跨文件符号图、方法/trait/类型分派、运行全链路、生产解析进程隔离、真实客户端
安装及完整图2/M2/M3仍未完成。进一步扩展应单独评估成熟语义引擎及其加载/执行边界。

规则依据：[Rust 作用域](https://doc.rust-lang.org/reference/names/scopes.html)、
[标识符](https://doc.rust-lang.org/reference/identifiers.html)。

## 开发验收记录（2026-09-12）

本片基于 T-044 `82a620d`，本地分支 `codex/m3-rust-call-links`，未推送、合并或部署。

- macOS：workbench/local-server 472 项相关 Rust、另 26 项架构守卫通过。
- Linux aarch64 Docker 单作业：对应 473 项相关 Rust、另 26 项架构守卫通过。
- 两平台：fmt、workspace all-targets 严格 Clippy、Auth production 特性 dev 构建通过；
  这不是 release 运行包、服务器原生或部署验收。
- 本片新增 34 项纯层及 11 项采集/HTTP 契约。旧 derive 图和错误行为、黄金批哈希、
  排序/改名/参数正文换批、遮蔽/宏/属性/导入、异常后恢复与门禁均验证。
- 前端 54 文件 482 项测试（新增 66 项）、lint、Prettier、tsc/Vite 构建通过。
- 独立真实 HTTP：虚构三文件 29 上下文、28 调用、15 关联；collect 有 4 处引用，
  unused 为 0。自身 rust_names.rs/rust_extract.rs 两文件 30 上下文、172 调用、
  2 关联，实际 callee → identifier/path 定义已核对。Python 独立重算全部批哈希，
  核对所有来源字节范围/行/字节列及文件只读指纹；两个短命进程已停止。
- 浏览器 [4328](http://127.0.0.1:4328/) 在 1440×1000、390×844 无横向溢出；
  正反向选择、递归、零引用、未解析、空筛选、切文件清理、键盘 Tab、原文第11行与
  Escape 焦点恢复、重新核验及快照1→2刷新均通过，控制台无 warn/error。
  临时视口已恢复，保留独立虚构预览，未重启共享服务。

可重跑样例及真实 HTTP 核验见 [示例说明](../../examples/m3-rust-call-links/README.md)。
保守规则会少连；此结果不覆盖完整符号语义或生产解析隔离。
