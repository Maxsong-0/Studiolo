//! 字节与小写十六进制互转、SHA-256 文本形状的判断只在 `studiolo-core` 定义一次（T-246）：
//! 原来 22 个包各写一份，有两处解码还顺带收了大写和前导 `+`。
//! 这里钉住：各包 `src/` 的产品代码（不含测试模块和测试文件）不再手写这几种写法，改调
//! `studiolo_core::{to_lower_hex, is_lower_hex, is_sha256_hex, from_lower_hex, sha256_hex}`；
//! 只放行 core 那一份和逐条登记、次数写死的变体。
use crate::support::rust_lexer::{enclosed, lex, lex_product_with_literals, lex_with_literals};
use crate::support::{rust_sources, workspace_root};
use std::{collections::BTreeMap, fs};

/// 唯一定义所在的文件。
const DEFINITION: &str = "crates/core/src/hex.rs";

/// 手写写法的名字。
const TABLE: &str = "查表 b\"0123456789abcdef\"";
const RANGE: &str = "字符范围 b'a'..=b'f'";
const NOT_UPPER: &str = "is_ascii_hexdigit() && !…is_ascii_uppercase()";
const FORMAT: &str = "逐字节 {:02x}";
const RADIX: &str = "from_str_radix(…, 16)";

const HINT: &str = "改调 studiolo_core：转文本用 to_lower_hex（普通 SHA-256 再转用 sha256_hex），\
    判形状用 is_sha256_hex／is_lower_hex，解回字节用 from_lower_hex；\
    接受集或输出本来就不同的变体才在本文件 VARIANTS 登记";

/// 登记的变体：(文件, 写法, 次数, 为什么不收)。次数写死，多一处、少一处都判红。
const VARIANTS: [(&str, &str, usize, &str); 7] = [
    (
        "crates/auth/src/session/secrets.rs",
        RADIX,
        1,
        "16 位序号按 i64 解成正整数，不是字节",
    ),
    (
        "crates/auth/src/session/recovery/secrets.rs",
        RADIX,
        2,
        "恢复码里 generation、slot 两个整数",
    ),
    (
        "crates/auth/src/session/recovery/secrets.rs",
        FORMAT,
        1,
        "恢复码里 slot 整数定宽两位",
    ),
    (
        "crates/sync-launcher/src/facts.rs",
        RADIX,
        1,
        "boot_id：32 位、收大写和 UUID 连字符",
    ),
    (
        "crates/vault-admin/src/lib.rs",
        RADIX,
        1,
        "命令行参数：收大写再解",
    ),
    (
        "crates/agent-server/src/connector.rs",
        FORMAT,
        1,
        "use 名单被 connector_entry 守卫逐行封住，改调 core 要先放开那份名单",
    ),
    (
        "crates/local-server/src/board/presence.rs",
        RANGE,
        1,
        "同上：presence 的 use 名单也被 connector_entry 守卫封住",
    ),
];

/// 各包 `src/` 下的产品源码：不含 `tests/` 子目录、`tests.rs`、`*_tests.rs`。
fn is_product(relative: &str) -> bool {
    let parts = relative.split('/').collect::<Vec<_>>();
    let [crates, _, src, inner @ .., file] = parts.as_slice() else {
        return false;
    };
    *crates == "crates"
        && *src == "src"
        && !inner.contains(&"tests")
        && *file != "tests.rs"
        && !file.ends_with("_tests.rs")
}

/// 产品代码里每种手写写法出现几次；测试模块、注释不算，普通字符串里写着同样的字也不算。
fn spellings(source: &str) -> BTreeMap<&'static str, usize> {
    let tokens = lex_product_with_literals(source);
    let mut found = BTreeMap::new();
    for (name, spelling) in [
        (TABLE, "b\"0123456789abcdef\""),
        (RANGE, "b'a'..=b'f'"),
        (NOT_UPPER, "is_ascii_hexdigit() && !"),
    ] {
        let needle = lex_with_literals(spelling);
        let count = tokens
            .windows(needle.len())
            .filter(|window| *window == needle.as_slice())
            .count();
        found.insert(name, count);
    }
    // 只有字面量记号里才会有 `:02x}`：标识符和标点切出来的记号不含冒号和花括号连在一起。
    let formats = tokens.iter().map(|token| token.matches(":02x}").count());
    found.insert(FORMAT, formats.sum());
    let radix = (0..tokens.len())
        .filter(|&at| tokens[at] == "from_str_radix")
        .filter(|&at| {
            matches!(
                enclosed(&tokens[at + 1..], "(", ")"),
                [open, .., comma, base, _] if open == "(" && comma == "," && base == "16"
            )
        })
        .count();
    found.insert(RADIX, radix);
    found.retain(|_, count| *count > 0);
    found
}

#[test]
fn hex_helpers_are_written_once_in_core() {
    let root = workspace_root();
    let (mut found, mut scanned) = (BTreeMap::new(), 0);
    for path in rust_sources(&root.join("crates")) {
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if relative == DEFINITION || !is_product(&relative) {
            continue;
        }
        scanned += 1;
        for (name, count) in spellings(&fs::read_to_string(&path).unwrap()) {
            found.insert((relative.clone(), name), count);
        }
    }
    assert!(scanned > 300, "只扫到 {scanned} 个产品源码文件，守卫失效");
    let registered = VARIANTS
        .iter()
        .map(|(path, name, count, _)| (((*path).to_owned(), *name), *count))
        .collect::<BTreeMap<_, _>>();
    let unregistered = found
        .iter()
        .filter(|(key, count)| registered.get(*key) != Some(count))
        .collect::<Vec<_>>();
    assert!(
        unregistered.is_empty(),
        "产品代码里手写了十六进制转换或判断：{unregistered:?}。{HINT}"
    );
    let stale = registered
        .iter()
        .filter(|(key, count)| found.get(*key) != Some(count))
        .collect::<Vec<_>>();
    assert!(
        stale.is_empty(),
        "登记的变体和源码对不上（改掉了就删登记；次数变了先核是不是还是变体）：{stale:?}"
    );
}

#[test]
fn core_defines_and_exports_every_shared_helper() {
    let root = workspace_root();
    let read = |path: &str| lex(&fs::read_to_string(root.join(path)).unwrap()).concat();
    let definition = read(DEFINITION);
    let exports = read("crates/core/src/lib.rs");
    for name in [
        "to_lower_hex",
        "is_lower_hex",
        "is_sha256_hex",
        "from_lower_hex",
        "sha256_hex",
    ] {
        assert!(
            definition.contains(&format!("pubfn{name}")),
            "{DEFINITION} 里要有 pub fn {name}"
        );
    }
    assert!(
        exports.contains(
            "pubusehex::{from_lower_hex,is_lower_hex,is_sha256_hex,sha256_hex,to_lower_hex};"
        ),
        "studiolo-core 要从 hex 模块导出这五个函数"
    );
}

#[test]
fn each_hand_written_spelling_is_caught() {
    for (planted, name) in [
        (
            "fn f(b: u8) -> u8 { b\"0123456789abcdef\"[usize::from(b >> 4)] }",
            TABLE,
        ),
        (
            "fn f(v: &str) -> bool { v.bytes().all(|b| (b'a'..=b'f').contains(&b)) }",
            RANGE,
        ),
        (
            "fn f(v: &str) -> bool { v.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) }",
            RANGE,
        ),
        (
            "fn f(b: u8) -> bool { b.is_ascii_hexdigit() && !b.is_ascii_uppercase() }",
            NOT_UPPER,
        ),
        (
            "fn f(v: &[u8]) -> String { v.iter().map(|b| format!(\"{b:02x}\")).collect() }",
            FORMAT,
        ),
        (
            "fn f(v: &[u8]) -> String { v.iter().map(|b| format!(\"{:02x}\", b)).collect() }",
            FORMAT,
        ),
        (
            "fn f(o: &mut String, b: u8) { let _ = write!(o, \"{b:02x}\"); }",
            FORMAT,
        ),
        (
            "fn f(v: &str) -> Option<u8> { u8::from_str_radix(&v[0..2], 16).ok() }",
            RADIX,
        ),
        // 测试模块后面的产品代码照样查。
        (
            "#[cfg(test)] mod t {} fn f(b: u8) -> String { format!(\"{b:02x}\") }",
            FORMAT,
        ),
    ] {
        assert_eq!(
            spellings(planted),
            BTreeMap::from([(name, 1)]),
            "应当拦下：{planted}"
        );
    }
}

#[test]
fn comments_strings_tests_variants_and_core_calls_are_not_caught() {
    for allowed in [
        "// 别写 b\"0123456789abcdef\"、(b'a'..=b'f')、{b:02x}",
        "/* u8::from_str_radix(x, 16) */",
        "const NOTE: &str = \"b'a'..=b'f' is_ascii_hexdigit() && !\";",
        "fn f(n: u64) -> String { format!(\"{n:016x}\") }",
        "fn f(b: u8) -> String { format!(\"%{b:02X}\") }",
        "fn f(v: &str) -> bool { v.bytes().all(|b| b.is_ascii_hexdigit()) }",
        "fn f(v: &str) -> Option<i64> { i64::from_str_radix(v, 10).ok() }",
        "fn f(v: &str) -> bool { studiolo_core::is_sha256_hex(v) }",
        "#[cfg(test)]\nmod tests { fn f(b: u8) -> String { format!(\"{b:02x}\") } }",
        "#[cfg(test)]\n#[path = \"x.rs\"]\nmod tests;",
    ] {
        assert!(spellings(allowed).is_empty(), "不该拦：{allowed}");
    }
}

#[test]
fn only_product_files_under_src_are_scanned() {
    for product in [
        "crates/core/src/hex.rs",
        "crates/local-server/src/sync_plane/driver/evidence.rs",
    ] {
        assert!(is_product(product), "{product}");
    }
    for other in [
        "crates/core/tests/hex_contract.rs",
        "crates/vault-broker/src/mutation/tests/stages.rs",
        "crates/local-server/src/board/presence/tests.rs",
        "crates/auth/src/session/recovery/secrets_tests.rs",
        "crates/studiolo/tests/architecture_cases/hex_single_definition.rs",
        "crates/core/build.rs",
    ] {
        assert!(!is_product(other), "{other}");
    }
}
