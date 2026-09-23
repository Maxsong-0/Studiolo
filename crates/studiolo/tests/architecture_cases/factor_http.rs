//! 因子网络动作不获得原始来源工厂、秘密环境或通用文件权限。
//!
//! 秘密环境守卫按记号判定：先剥掉注释、字符串与字符字面量，再认 `env::` 路径、
//! `var_os`／`vars_os` 裸名，以及 `use` 里对 `std::env` 的分组、通配与别名导入；换行、空白、
//! 原始标识符、turbofish 与函数指针都绕不开。它只看 local-server 自己的源码。

use crate::support::{rust_sources, source_violations, workspace_root};
use std::{fs, path::Path};

/// 能读出任意进程环境变量的 std 入口；`args_os` 读命令行参数、`temp_dir` 等只读固定的
/// 非秘密变量，不在此列。
const ENVIRONMENT_READS: [&str; 4] = ["var", "var_os", "vars", "vars_os"];

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
        if !path.ends_with("startup/factor_input.rs") {
            let reads = environment_reads(&source);
            assert!(
                reads.is_empty(),
                "环境只能在因子装配读取：{path:?} {reads:?}"
            );
        }
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
    assert!(input.contains("TotpSeed::from_hex("));
    assert!(!input.contains("TotpEnrollment"));
    // 判定器认得唯一允许的真实读取点，守卫不是空转。
    assert!(!environment_reads(&input).is_empty());
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
    ] {
        assert_eq!(
            environment_reads(source),
            Vec::<String>::new(),
            "误报：{source}"
        );
    }
}

/// 源码里读取进程环境变量的每一处写法。
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
            // `var`／`vars` 太常见，只认 `env::` 路径；`*` 是通配导入，`$` 是宏拼出的路径段。
            "env" if next == "::" && matches!(after, "var" | "vars" | "*" | "$") => {
                reads.push(format!("env::{after}"));
            }
            "env"
                if in_use && next == "::" && after == "{" && group_reads(&tokens[index + 2..]) =>
            {
                reads.push("env::{..}".to_owned());
            }
            // 给 std::env 起别名，后面的 `别名::var(` 就绕开了 `env::` 判定。
            "env" if in_use && next == "as" => reads.push("env as".to_owned()),
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

fn is_word(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// 切成标识符与标点记号：跳过空白、注释（块注释可嵌套）、字符串与字符字面量，
/// `::` 合成一个记号，原始标识符 `r#名字` 记作 `名字`。
fn lex(source: &str) -> Vec<String> {
    let chars = source.chars().collect::<Vec<_>>();
    let (mut tokens, mut index) = (Vec::new(), 0);
    while index < chars.len() {
        let rest = &chars[index..];
        index += match rest {
            [c, ..] if c.is_whitespace() => 1,
            ['/', '/', ..] => rest.iter().take_while(|&&c| c != '\n').count(),
            ['/', '*', ..] => block_comment_len(rest),
            ['"', tail @ ..] => 1 + quoted_len(tail, None),
            // 字符字面量；转义形式从被转义字符之后找收尾引号。其余 `'` 是生命周期或标签。
            ['\'', '\\', _, tail @ ..] => 4 + tail.iter().take_while(|&&c| c != '\'').count(),
            ['\'', _, '\'', ..] => 3,
            [':', ':', ..] => {
                tokens.push("::".to_owned());
                2
            }
            [c, ..] if is_word(*c) => word_len(rest, &mut tokens),
            [c, ..] => {
                tokens.push(c.to_string());
                1
            }
            [] => unreachable!("循环条件保证还有字符"),
        };
    }
    tokens
}

/// 块注释的长度（含嵌套）。
fn block_comment_len(rest: &[char]) -> usize {
    let (mut depth, mut index) = (0_usize, 0);
    while index < rest.len() {
        match &rest[index..] {
            ['/', '*', ..] => (depth, index) = (depth + 1, index + 2),
            ['*', '/', ..] if depth == 1 => return index + 2,
            ['*', '/', ..] => (depth, index) = (depth - 1, index + 2),
            _ => index += 1,
        }
    }
    index
}

/// 开引号之后到收尾引号（含）的长度；原始字符串带 `hashes` 个 `#` 收尾且不认转义。
fn quoted_len(rest: &[char], hashes: Option<usize>) -> usize {
    let mut index = 0;
    while let Some(&c) = rest.get(index) {
        match (c, hashes) {
            ('\\', None) => index += 2,
            ('"', None) => return index + 1,
            ('"', Some(count))
                if rest
                    .get(index + 1..index + 1 + count)
                    .is_some_and(|tail| tail.iter().all(|&c| c == '#')) =>
            {
                return index + 1 + count;
            }
            _ => index += 1,
        }
    }
    index
}

/// 标识符；`r`／`br`／`cr` 紧跟（可带 `#` 的）引号是原始字符串，`r#名字` 是原始标识符。
fn word_len(rest: &[char], tokens: &mut Vec<String>) -> usize {
    let len = rest.iter().take_while(|&&c| is_word(c)).count();
    let word = rest[..len].iter().collect::<String>();
    let hashes = rest[len..].iter().take_while(|&&c| c == '#').count();
    match (word.as_str(), rest.get(len + hashes)) {
        ("r" | "br" | "cr", Some('"')) => {
            len + hashes + 1 + quoted_len(&rest[len + hashes + 1..], Some(hashes))
        }
        // 只跳过 `r#`，名字在下一轮照常切出。
        ("r", Some(&c)) if hashes == 1 && is_word(c) => len + 1,
        _ => {
            tokens.push(word);
            len
        }
    }
}
