//! nextest 每个测试一个进程，vault-broker 那把进程内的大文件闸门（`blob::LARGE_IO`）管不到它们；
//! `.config/nextest.toml` 用测试组把这些测试重新排成一次只跑一个（T-187）。
//!
//! 这里核对两边一致：每个进闸门、默认会跑的测试都在名单里，名单里也没有已经不存在的名字。
//! 漏一个，换用 nextest 时就会有两个处理几百 MiB 正文的测试同时跑。

use crate::support::workspace_root;
use std::{collections::BTreeSet, fs, path::Path};

const CONFIG: &str = ".config/nextest.toml";
const CRATE: &str = "crates/vault-broker";
/// 闸门自己的实现文件：里面的 `enter` 是定义，不是使用。
const GATE: &str = "serial_gate.rs";

#[test]
fn nextest_large_io_group_lists_exactly_the_tests_that_enter_the_gate() {
    let root = workspace_root();
    let config = fs::read_to_string(root.join(CONFIG)).expect("nextest 配置必须可读");
    assert!(
        config.contains("vault-broker-large-io = { max-threads = 1 }"),
        "大文件测试组必须一次只跑一个"
    );
    assert!(
        config.contains("test-group = 'vault-broker-large-io'"),
        "名单必须挂在大文件测试组上"
    );
    assert!(
        config.contains("\nretries = 0\n"),
        "不开重试：偶发失败不许被掩盖"
    );

    let mut entering = BTreeSet::new();
    for directory in ["src", "tests"] {
        collect(&root.join(CRATE).join(directory), &mut entering);
    }
    assert!(!entering.is_empty(), "没找到任何进闸门的测试，守卫失效");
    assert_eq!(
        listed(&config),
        entering,
        "{CONFIG} 的大文件测试名单要和 {CRATE} 里进 LARGE_IO 闸门的测试一致"
    );
}

/// 配置里那条过滤式列出的测试名：`test(/::(a|b|c)$/)`。
fn listed(config: &str) -> BTreeSet<String> {
    let (_, rest) = config
        .split_once("test(/::(")
        .expect("名单必须写成 test(/::(名字|名字)$/)");
    let (names, _) = rest.split_once(")$/)").expect("名单必须以 )$/) 收尾");
    names.split('|').map(str::to_owned).collect()
}

/// 收集目录下所有进闸门、且没有标 `#[ignore]` 的测试函数名。
fn collect(directory: &Path, entering: &mut BTreeSet<String>) {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .expect("源码目录必须可枚举")
        .map(|entry| entry.expect("目录项必须可读").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, entering);
        } else if path.extension().is_some_and(|extension| extension == "rs")
            && path.file_name().is_some_and(|name| name != GATE)
        {
            scan(&path, entering);
        }
    }
}

fn scan(path: &Path, entering: &mut BTreeSet<String>) {
    let source = fs::read_to_string(path).expect("源码必须可读");
    let lines: Vec<&str> = source.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        if !(line.contains("LARGE_IO.enter()") || line.contains("LARGE.enter()")) {
            continue;
        }
        // 往上找到它所在的函数，再看函数头上面紧挨着的属性。
        let head = (0..index)
            .rev()
            .find(|&at| function_name(lines[at]).is_some())
            .unwrap_or_else(|| panic!("{}:{} 进闸门的地方不在函数里", path.display(), index + 1));
        let attributes: Vec<&str> = lines[..head]
            .iter()
            .rev()
            .map(|line| line.trim())
            .take_while(|line| line.starts_with("#[") || line.starts_with("//"))
            .collect();
        assert!(
            attributes
                .iter()
                .any(|line| line.starts_with("#[test]") || line.starts_with("#[tokio::test")),
            "{}:{} 进闸门的不是测试函数：辅助函数进闸门时，名单没法按名字核对",
            path.display(),
            head + 1
        );
        if !attributes.iter().any(|line| line.starts_with("#[ignore")) {
            entering.insert(function_name(lines[head]).expect("上面已确认").to_owned());
        }
    }
}

fn function_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("fn ")?;
    let end = rest.find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))?;
    (end > 0).then(|| &rest[..end])
}
