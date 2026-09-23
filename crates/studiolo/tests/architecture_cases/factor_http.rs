//! 因子网络动作不获得原始来源工厂、秘密环境或通用文件权限。
//!
//! 秘密环境守卫按记号判定（记号化见 `support::rust_lexer`）：先剥掉注释、字符串与字符字面量，
//! 再认 `env::` 路径、`var_os`／`vars_os` 裸名、`use` 里对 `std::env` 的分组、通配与别名导入，
//! 以及编译期读环境或把文件嵌进产物的内建宏 `env!`、`option_env!`、`include!`、`include_str!`、
//! `include_bytes!`（带路径前缀、`use … as` 起别名、宏里 `$m!` 拼出调用名都算）；换行、空白、
//! 原始标识符、turbofish 与函数指针都绕不开。
//!
//! 扫描范围是 local-server 经 `[dependencies]`／`[build-dependencies]` 传递可达的整个工作区
//! 依赖闭包：闭包内每个 crate 的 `src/**/*.rs` 与 `build.rs`。现有的合法命中逐文件、逐次钉死在
//! `ALLOWED_ENVIRONMENT_READS`，多一处或少一处都红；唯一的秘密种子读取点仍是因子装配。

use crate::support::{
    rust_lexer::{is_word, lex},
    rust_sources, source_violations, workspace_dependency_closure, workspace_root,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// 能读出任意进程环境变量的 std 入口；`args_os` 读命令行参数、`temp_dir` 等只读固定的
/// 非秘密变量，不在此列。
const ENVIRONMENT_READS: [&str; 4] = ["var", "var_os", "vars", "vars_os"];

/// 编译期读环境（`env!`／`option_env!`）或把任意文件嵌进产物（`include*!`）的内建宏。
const COMPILE_TIME_READS: [&str; 5] = [
    "env",
    "option_env",
    "include",
    "include_str",
    "include_bytes",
];

/// 依赖闭包里允许的命中，路径相对仓库根，命中序列按源码次序；多一处或少一处都红。
const ALLOWED_ENVIRONMENT_READS: [(&str, &[&str]); 6] = [
    // 唯一的秘密种子装配点：读当前与替换两个 TOTP 种子槽。
    (
        "crates/local-server/src/startup/factor_input.rs",
        &["var_os", "var_os"],
    ),
    // 只判断 SMTP 配置在不在，不留值。
    ("crates/observability/src/alert/transport.rs", &["env::var"]),
    // 沙箱会话路径 XDG_RUNTIME_DIR、DBUS_SESSION_BUS_ADDRESS，非秘密；另嵌入仓库内 deploy/runner/rg_probe.py 比对完整性。
    (
        "crates/runner-adapter/src/isolation.rs",
        &["var_os", "var_os", "include_bytes!"],
    ),
    // 嵌入仓库内 deploy/runner/launch.py，用于完整性比对。
    ("crates/runner-adapter/src/real.rs", &["include_bytes!"]),
    // 测试专用（只在 cfg(test) 编译）：受控父测试拉起子进程时设的标记，非秘密。
    (
        "crates/vault-broker/src/file_operation/tests/large.rs",
        &["env::var"],
    ),
    // 测试专用（只在 cfg(test) 编译）：受控父测试拉起子进程时设的标记，非秘密。
    (
        "crates/vault-broker/src/stream/tests/memory.rs",
        &["env::var"],
    ),
];

#[test]
fn unix_socket_adapter_does_not_authorize_other_file_or_database_writers() {
    let path = Path::new("crates/local-server/src/unix_ingress/layout.rs");
    let source = fs::read_to_string(workspace_root().join(path)).unwrap();
    assert!(source_violations("studiolo-local-server", path, &source).is_empty());
    for extra in [
        "fs::write(path, bytes);",
        "File::create(path);",
        "open(other, flags, mode);",
        "unlinkat(root, unrelated, AtFlags::empty());",
    ] {
        assert!(
            !source_violations("studiolo-local-server", path, &format!("{source}\n{extra}"))
                .is_empty()
        );
    }
    assert!(
        !source_violations(
            "studiolo-local-server",
            Path::new("crates/local-server/src/handler.rs"),
            &source
        )
        .is_empty()
    );
    assert!(!source_violations("studiolo-auth", path, &source).is_empty());
}

#[test]
fn source_proofs_and_seed_environment_have_a_single_trusted_construction_boundary() {
    let root = workspace_root();
    for path in rust_sources(&root.join("crates/local-server/src")) {
        let source = fs::read_to_string(&path).unwrap();
        if !path.ends_with("unix_ingress/peer.rs") {
            assert!(
                !source.contains("TrustedNetworkSource {"),
                "来源不能由普通处理器构造：{path:?}"
            );
            assert!(!source.contains("UnixPeer {"), "连接凭据构造边界：{path:?}");
        }
    }
    let peer =
        fs::read_to_string(root.join("crates/local-server/src/unix_ingress/peer.rs")).unwrap();
    assert!(peer.contains(".peer_cred()"));
    assert!(!peer.contains("pub fn source"));
    let input =
        fs::read_to_string(root.join("crates/local-server/src/startup/factor_input.rs")).unwrap();
    // 种子只在因子装配里构造；它从环境读取、且只在那里读，由下面的钉死表证明。
    assert!(input.contains("TotpSeed::from_hex("));
    assert!(!input.contains("TotpEnrollment"));
}

#[test]
fn secret_environment_is_read_only_at_pinned_points_across_the_dependency_closure() {
    let root = workspace_root();
    let allowed = ALLOWED_ENVIRONMENT_READS
        .into_iter()
        .map(|(path, reads)| (PathBuf::from(path), reads))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        allowed.len(),
        ALLOWED_ENVIRONMENT_READS.len(),
        "钉死表重复登记"
    );
    let mut matched = BTreeSet::new();
    for directory in workspace_dependency_closure(&root, "studiolo-local-server") {
        let build_script = directory.join("build.rs");
        let sources = rust_sources(&directory.join("src"))
            .into_iter()
            .chain(build_script.is_file().then_some(build_script));
        for path in sources {
            let relative = path.strip_prefix(&root).unwrap().to_path_buf();
            let reads = environment_reads(&fs::read_to_string(&path).unwrap());
            let expected = allowed.get(&relative).copied().unwrap_or_default();
            assert_eq!(
                reads, expected,
                "读环境或编译期嵌文件只许出现在钉死的位置与次数：{relative:?}"
            );
            if !reads.is_empty() {
                matched.insert(relative);
            }
        }
    }
    // 表里每一项都必须在闭包里真实存在且被命中，防止表过期。
    let stale = allowed
        .keys()
        .filter(|path| !matched.contains(*path))
        .collect::<Vec<_>>();
    assert!(
        stale.is_empty(),
        "钉死表过期（不在依赖闭包里或已无命中）：{stale:?}"
    );
}

#[test]
fn dependency_closure_covers_the_local_server_runtime() {
    let root = workspace_root();
    let closure = workspace_dependency_closure(&root, "studiolo-local-server");
    for member in [
        "local-server",
        "auth",
        "host-config",
        "supply",
        "policy",
        "runner-adapter",
        "observability",
    ] {
        let directory = root.join("crates").join(member);
        assert!(closure.contains(&directory), "依赖闭包漏了 {member}");
    }
    // 读种子是其本职的 CLI 与带版本号宏的集成 crate 不在闭包里：闭包确有取舍，守卫不是空转。
    for outsider in ["auth-admin", "studiolo"] {
        let directory = root.join("crates").join(outsider);
        assert!(!closure.contains(&directory), "{outsider} 不该在依赖闭包里");
    }
}

#[test]
fn environment_guard_catches_every_spelling_and_ignores_comments_and_literals() {
    for source in [
        r#"std::env::var_os("X")"#,
        r#"env::var_os("X")"#,
        r#"std::env::var("X")"#,
        r#"env::var("X")"#,
        "std::env::vars_os()",
        "for (key, value) in env::vars() {}",
        "std :: env\n    :: var\n    (\"X\")",
        r#"std::env::/* 注释 */var("X")"#,
        r#"std::env::var::<&str>("X")"#,
        "let read = std::env::var;",
        r#"::std::env::r#var("X")"#,
        r#"use std::env::var; fn f() { var("X"); }"#,
        "use std::env::*; fn f() { vars(); }",
        "use std::env::{args_os, var as read};",
        "use std::{env::{self as e}, fs};",
        "use std::env as e;",
        "pub(crate) use std::env::vars_os;",
        r#"use crate::startup::factor_input::var_os; fn f() { var_os("X"); }"#,
        r#"macro_rules! m { ($f:ident) => { std::env::$f("X") } }"#,
        // 生命周期与各类字面量不能吞掉后面的真代码。
        "fn f<'a>(x: &'a str) { env::var(x); }",
        r#"let q = '"'; env::var("X");"#,
        r#"let s = "a\"b"; env::var("X");"#,
        r##"let s = r#"say "hi""#; env::var("X");"##,
        r#"let s = br"\"; env::var("X");"#,
        // 编译期宏：带不带路径、隔不隔空白、起别名或由宏拼出调用名都算。
        r#"env!("X")"#,
        r#"option_env!("X")"#,
        r#"core::option_env!("X")"#,
        r#"include_str!("secret.txt")"#,
        r#"include_bytes!("k.bin")"#,
        r#"include!("gen.rs")"#,
        "env ! (\"X\")",
        r#"use core::include_bytes as embed; fn f() { embed!("k.bin"); }"#,
        r#"macro_rules! m { ($f:ident) => { $f!("X") } } m!(env);"#,
    ] {
        assert!(!environment_reads(source).is_empty(), "漏抓：{source}");
    }
    for source in [
        r#"// std::env::var_os("X")"#,
        r#"/* env::var("X") /* 嵌套 */ env::vars() */"#,
        r#"let s = "std::env::var_os(\"X\")";"#,
        r##"let s = r#"env::vars()"#;"##,
        "let args = std::env::args_os();",
        "use std::env; use std::env::{self, args_os};",
        r#"command.env("PATH", "/bin"); let vars = config.vars(); let n = env as u32;"#,
        r#"// env!("X")"#,
        r#"let s = "include_str!(\"x\")";"#,
        "my_env!()",
        "environment!()",
        "if env != other && self.include != 0 {}",
        "macro_rules! m { () => { $crate::helper!() } }",
    ] {
        assert_eq!(
            environment_reads(source),
            Vec::<String>::new(),
            "误报：{source}"
        );
    }
}

/// 源码里读取进程环境变量、或在编译期读环境与嵌入文件的每一处写法。
fn environment_reads(source: &str) -> Vec<String> {
    let tokens = lex(source);
    let at = |index: usize| tokens.get(index).map_or("", String::as_str);
    let (mut reads, mut in_use) = (Vec::new(), false);
    for (index, token) in tokens.iter().enumerate() {
        let (next, after) = (at(index + 1), at(index + 2));
        match token.as_str() {
            "use" => in_use = true,
            ";" => in_use = false,
            // 这两个名字只属于环境读取：带路径、裸调用（经 `use` 引入或再导出）、分组导入都抓。
            "var_os" | "vars_os" => reads.push(token.clone()),
            // 编译期宏：`std::`／`core::` 前缀不影响判定，`!=` 已由记号化与宏调用分开。
            name if next == "!" && COMPILE_TIME_READS.contains(&name) => {
                reads.push(format!("{name}!"));
            }
            // 宏把调用名当参数拼进来（`$m!(..)`），拼进来的可能正是上面这几个宏。
            "$" if after == "!" && next.starts_with(is_word) => reads.push(format!("${next}!")),
            // `var`／`vars` 太常见，只认 `env::` 路径；`*` 是通配导入，`$` 是宏拼出的路径段。
            "env" if next == "::" && matches!(after, "var" | "vars" | "*" | "$") => {
                reads.push(format!("env::{after}"));
            }
            "env"
                if in_use && next == "::" && after == "{" && group_reads(&tokens[index + 2..]) =>
            {
                reads.push("env::{..}".to_owned());
            }
            // 给 std::env 或上面这几个宏起别名，后面的 `别名::var(`／`别名!(` 就绕开了前面的判定。
            name if in_use && next == "as" && COMPILE_TIME_READS.contains(&name) => {
                reads.push(format!("{name} as"));
            }
            _ => {}
        }
    }
    reads
}

/// `env::{..}` 分组导入里有读取函数、通配或 `self as` 别名。
fn group_reads(group: &[String]) -> bool {
    let mut depth = 0;
    for (index, token) in group.iter().enumerate() {
        match token.as_str() {
            "{" => depth += 1,
            "}" if depth == 1 => return false,
            "}" => depth -= 1,
            "*" => return true,
            "self" if group.get(index + 1).is_some_and(|next| next == "as") => return true,
            name if ENVIRONMENT_READS.contains(&name) => return true,
            _ => {}
        }
    }
    false
}
