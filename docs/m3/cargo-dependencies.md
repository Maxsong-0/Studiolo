# T-043 Cargo 清单依赖派生

在项目运作页上查看 Cargo 显式成员之间的本地包依赖。清单派生保留包名、依赖别名、
种类、目标条件、可选标记与来源行；这是清单证据，不是源码调用、feature 解析后的
构建图或运行轨迹。不会执行 Cargo、构建脚本或插件，不读源码、Cargo.lock、用户
Cargo 配置或远端包；生产仓库隔离接入仍另验。

## 能力与读取

`--cargo-dependencies` 是无值、不可重复开关，必须配合 `--project-repository`。
原目录绑定默认不读 Cargo 清单；库装配通过
`ProjectRepository::open(...).with_cargo_dependencies()` 明确启用。沿用既有本机/
Access 只读模式限制，不允许与 session/canonical/factor 模式组合。HTTP 不接收
清单开关、宿主路径、任意文件参数，也不提供原始清单正文读取端点。

只读根 `Cargo.toml` 及该根 `workspace.members` 明确列出的目录中 `Cargo.toml`。
纯解析器先完成整个成员计划验证，采集器再使用原固定根的 Broker `read_file`。
保留符号链接/特殊文件/多链接/名称碰撞/跨设备/读取变化拒绝；请求前后重验绑定根，
两轮完整清单的逻辑路径、正文与 revision 必须一致。两轮一致是有界稳定观测，不是事务
快照，也不保证之后不变。源文 generation 与仓库观测时间分别表达。

每包目录最长 1024 UTF-8 字节、16 个分量；最多 64 包、65 清单（含虚拟根），
单清单 64 KiB、每轮总正文 2 MiB，两轮最多 4 MiB。包依赖条目合计最多 2048，
根 workspace.dependencies 模板另最多 2048，不计作包依赖；内部边最多 256。
纯派生预算超限为 unsupported；读取内核超限为 unavailable。
Broker 原有每层 50,000 项名称检查上限继续生效，不将字节预算说成
目录元信息工作量上限。任何超限原子拒绝，不裁剪图。

## 受支持的清单语义

使用固定版本 `toml_edit` 的不可变 `Document` 与 span；支持合法 TOML 表、inline
表、dotted keys、注释和引号，不用逐行字符串拆分代替 TOML 解析。解析器原始错误
可能含源码值，不能出现在 API/日志；失败只输出受控中文原因。

根可以是 package、virtual workspace，或二者同存；`members` 只接受显式目录。
计划含根清单且路径唯一稳定排序；重复目录、大小写碰撞、损坏字段或空包集合拒绝。
规范化允许 `.` 和 `..`，但最终必须在根内、没有隐藏段/反斜杠/控制字符且是合法
VaultPath；传入 Broker 的路径不含 `..`。根包 ID 用 `.`，其他包 ID 用规范目录。
包名/alias 为 1–64 字节 ASCII 字母、数字、下划线或连字符；包名不得重复。

支持 normal (`dependencies`)、dev、build 及各 `target.<条件>` 下的同类依赖。
target 保留为最多 256 UTF-8 字节的非空单行文本，不判断条件真假。optional 必须
为布尔；features/default-features 只校验类型，不选择或展开 feature。所有条件和
可选依赖仍保留在图里。依赖为 version 字符串或表，路径相对当前清单目录；使用
`workspace = true` 时从根 workspace.dependencies 按 alias 继承，path 相对根，
同时保留成员引用行与根定义行。继承表不能再指定 path/git/version/package；允许
成员追加 features 与 optional；根 workspace 依赖不允许 optional。

只有 path 指向已采集 package 且 package 名与 alias/`package` rename 一致时连边。
非 path 的 registry/git 依赖按声明条数计入 `external_dependencies`，同名不能连到
本地包。计数按 alias/kind/target 的声明次数，包含继承；不暴露 URL、版本或正文。
同一身份 `(from,to,alias,kind,target)` 不得重复；循环/自环正常呈现。

成员 glob、非空 exclude/patch/replace、嵌套 workspace、外部 package.workspace、
路径 base、集合外 path 依赖、旧式 dev_dependencies/build_dependencies 表及
不支持的依赖语义标为 unsupported，清空整图。
package.workspace 若存在只能规范指回已绑定根。未指定 dependency 来源、类型错误、
重复/悬空/名称不符等标为 invalid。只验证本图使用的字段，不冒充完整 Cargo 检查。
未使用的 workspace.dependencies 不计作包依赖，但不能通过继承掩盖不支持的语义。

## 纯派生接口与 HTTP

workbench 不增加 I/O：

```text
CargoManifest { path: String, content: String }
cargo_manifest_paths(root: &str) -> Result<Vec<String>, CargoDependencyError>
derive_cargo_dependencies(manifests: &[CargoManifest])
  -> Result<CargoDependencyGraph, CargoDependencyError>
CargoDependencyError { kind: Invalid | Unsupported, message: String }
ManifestSource { path: String, line: usize }
CargoDependencyGraph {
  packages: [{ id, name, source: ManifestSource }],
  dependencies: [{ from, to, alias, kind: normal|dev|build, target: string|null,
    optional: bool, source: ManifestSource, inherited_from: ManifestSource|null }],
  external_dependencies: u32
}
```

输入集合必须与计划完全相等；所有来源是原清单中的真实一基行号。包来源指向
`package.name`，边来源指向依赖 alias；继承定义来源来自根 Cargo.toml。包按 ID，
依赖按 from/to/kind/target/alias 排序；根路径固定 `Cargo.toml`，其余为目录加文件名。

现有 `GET /api/project-operations?project=...&generation=N` 新增：

```text
cargo_dependencies: {
  status: disabled | unbound | unavailable | invalid | unsupported | ready,
  observed_at: string | null,
  graph: CargoDependencyGraph | null,
  diagnostics: string[]
}
```

无/不匹配仓库绑定 unbound；匹配但未启用 disabled（完全不读清单）；缺失/不可读/
变动的清单 unavailable；解析错误 invalid；不支持语义 unsupported。只有 ready
带图和非空观测秒时间，其他状态 graph/observed_at 均 null。失败与 T-041 目录及
T-042 设计声明独立，沿用同代校验、认证/来源门禁和查询槽，不改变原 HTTP 错误码。

## 交互与验证

沿用 Apple 浅色、原生 DOM 文本和已有 Motion/reduced-motion；包选择、种类筛选、
方向、条件/可选标记与所有来源路径/行号可见。Cargo 来源属于仓库，与 Vault 原文
按钮分开；不把它伪装成可读取的笔记来源。按包 ID 与 T-042 节点目录精确匹配显示
全部关联声明及原文入口；无精确匹配保持无关联，不以名称或路径前缀猜关系。
这种关联不代表设计依赖已与实现一致，也不扩大 T-041 目录比较范围。

图形异常回退有来源的文本列表；换项目、刷新/重试或代次变化清理旧选择。旧响应
缺少此字段显示不可用；字段存在但损坏时明确响应错误，不能默默隐藏。

虚构示例见 [独立 Cargo 示例](../../examples/m3-cargo-dependencies/README.md)。

## 本轮验证（2026-09-11）

macOS 的 workbench/local-server 377 项 Rust 测试、另 26 项架构守卫通过；fmt、
workspace all-targets 严格 Clippy 与 Auth production 特性 dev 构建通过。
收敛检查告警后，受影响的 43 项 Cargo 派生/HTTP/进程/采集契约复验通过。
Linux aarch64 Docker 单构建作业：相关 Rust 378 项、另 26 项架构守卫、fmt、
workspace all-targets 严格 Clippy 与 Auth production 特性 dev 构建全部通过。
两平台构建均不是 release 运行包或服务器原生验收。
前端 44 个测试文件共 299 项、lint、format、tsc 与 Vite 生产构建通过。
本片新增 47 项 Rust 与 78 项前端契约，包含预算、权限、两轮变化、失败原子性、
真实进程开关接线、来源行、Unicode 路径、损坏响应拒绝与图形异常文本降级。

真实浏览器使用独立虚构预览 http://127.0.0.1:4326/，内容视口 1440×1000
与 390×844 均无页面横向溢出。三个包、五条内部依赖、两条外部声明显示正确；
普通/开发/构建筛选分别为 2/2/1 条，optional、cfg(unix)、rename 与继承来源均可见。
包来源第 2 行，成员引用第 7/10/11/14 行及根继承第 7 行与原清单匹配。
Cargo 路径是仓库文本；检索接口关联 Vault 原文第 10 行，Escape 恢复来源按钮焦点。
重新核验保持按钮焦点并重置选包/筛选；刷新从快照 1 到 2 同样清理旧选择。
旧 4325 服务缺少 Cargo 字段时正常降级，原目录/架构仍可用。浏览器无 warn/error，
34 项独立虚构夹具目录/文件指纹不变，临时视口已恢复。

用短命只读进程另行绑定 Studiolo 自身仓库，得到 16 个包、44 条内部依赖、127 条
外部依赖声明；包集合及每条 alias/kind/target/optional 与本轮锁定 Cargo metadata
逐项一致，所有来源行与实际清单相符。metadata 仅用于开发对照，服务运行时不执行
Cargo。验收进程已停止；此检查不是生产隔离接入、服务器原生或完整 Cargo 语义验收。

## 后续边界

本片仅完成显式 Cargo 成员清单证据。源码调用派生、真实运行事件、生产仓库隔离
接入、真实客户端安装与上线仍独立验收；完整图 2、M2/M3 未完成。

语义依据：[Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
与[依赖声明](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html)。
本片显式成员范围是受控子集，不声称覆盖 Cargo 自动成员发现、配置覆盖或解析器。
