//! 死 socket 接管（T-231）只有一份、只在三个入口用、只凭「握着主库锁」的凭据：
//!
//! 1. 删残留的那一行只在 `unix_ingress/leftover.rs`，删之前的判断一步不少（权限扫描的余串核对）；
//! 2. 凭据 `SoleInstance` 只有一处造（`coordinator/sole.rs`），造之前核 Broker 还在、没在关、批准的根没变；
//! 3. 借凭据、接管、按容得下残留准备的调用处都是白名单：多一个入口就要先改这里，等人看过。
//!    Runner 中继的 socket 不在里面：执行体在独立 scope，人侧死了它们可能还活着，锁证明不了什么。
use crate::support::{
    is_test_fixture, rust_lexer::lex_product, rust_sources, source_violations, workspace_root,
};
use std::{collections::BTreeMap, fs, path::Path};

const CRATE: &str = "studiolo-local-server";
const LEFTOVER: &str = "crates/local-server/src/unix_ingress/leftover.rs";
const SOLE: &str = "crates/local-server/src/coordinator/sole.rs";
const INGRESS: &str = "crates/local-server/src/unix_ingress.rs";
const LAYOUT: &str = "crates/local-server/src/unix_ingress/layout.rs";
const LISTENER: &str = "crates/local-server/src/backup/listener.rs";
const SERVING: &str = "crates/local-server/src/startup/serving.rs";

/// 每个名字在产品代码里出现的地方和次数（定义、导入、调用都算；注释和字符串不算）。
const MENTIONS: [(&str, &str, usize); 17] = [
    ("Entrance", LEFTOVER, 2),
    ("Entrance", LAYOUT, 3),
    ("Entrance", LISTENER, 2),
    ("SoleInstance", "crates/local-server/src/coordinator.rs", 1),
    ("SoleInstance", LEFTOVER, 2),
    ("SoleInstance", SOLE, 3),
    ("bind_over_leftover", INGRESS, 1),
    ("bind_over_leftover", SERVING, 2),
    ("prepare_over_leftover", INGRESS, 1),
    (
        "prepare_over_leftover",
        "crates/local-server/src/startup/factor_input.rs",
        1,
    ),
    (
        "prepare_over_leftover",
        "crates/local-server/src/startup/plane_input.rs",
        1,
    ),
    ("retire_leftover", LEFTOVER, 2),
    ("sole_instance", LEFTOVER, 1),
    ("sole_instance", SOLE, 1),
    ("take_over", INGRESS, 1),
    ("take_over", LEFTOVER, 1),
    ("take_over", LISTENER, 1),
];

/// 造凭据之前要核的，按次序；最后一步才交出来。
const CREDENTIAL: [&str; 6] = [
    "letowner=self.core.owner.lock()",
    "letbroker=owner.as_ref().ok_or(LocalError::Unavailable)?;",
    ".closing{returnErr(LocalError::Unavailable);}",
    "self.core.roots.revalidate().map_err(|_|LocalError::Configuration)?;",
    "broker.validate_root_identities(self.core.roots.vault_identity(),self.core.roots.writer_data_identity(),)?;",
    "Ok(SoleInstance{_owner:owner})",
];

fn sources() -> BTreeMap<String, String> {
    let root = workspace_root();
    rust_sources(&root.join("crates"))
        .into_iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(&root).unwrap().to_owned();
            let name = relative.to_str().unwrap().to_owned();
            (name.starts_with("crates/local-server/src/") && !is_test_fixture(&relative))
                .then(|| (name, fs::read_to_string(&path).unwrap()))
        })
        .collect()
}

/// 名字出现的地方和钉住的不一样时，说出多了少了什么。
fn mention_violation(sources: &BTreeMap<String, String>) -> Option<String> {
    let mut names = MENTIONS.map(|(name, _, _)| name).to_vec();
    names.dedup();
    let mut found = Vec::new();
    for name in names {
        for (path, source) in sources.iter().filter(|(_, text)| text.contains(name)) {
            let count = lex_product(source)
                .iter()
                .filter(|token| *token == name)
                .count();
            if count > 0 {
                found.push((name, path.as_str(), count));
            }
        }
    }
    let mut expected = MENTIONS.to_vec();
    found.sort_unstable();
    expected.sort_unstable();
    (found != expected).then(|| {
        format!("死 socket 接管出现的地方变了（要加入口先想清楚再改 MENTIONS）：{found:?}")
    })
}

/// 凭据的核对缺一步、换了次序、或者多造一处，都说出来。
fn credential_violation(source: &str) -> Option<String> {
    let compact = lex_product(source).concat();
    let mut at = 0;
    for step in CREDENTIAL {
        match compact[at..].find(step) {
            Some(found) => at += found + step.len(),
            None => return Some(format!("造凭据之前少了或者挪了：{step}")),
        }
    }
    (compact.matches("SoleInstance{").count() != 1).then(|| "凭据只许有一处造".to_owned())
}

#[test]
fn leftover_takeover_has_one_remover_one_credential_and_three_entrances() {
    let mut sources = sources();
    assert_eq!(mention_violation(&sources), None);
    for (path, extra) in [
        (LISTENER, "fn other(e: &Entrance<'_>) {}"),
        (SERVING, "fn other(w: &W) { w.sole_instance(); }"),
        (INGRESS, "fn other(e: &E, w: &W) { e.take_over(w); }"),
        (
            "crates/local-server/src/board/mod.rs",
            "fn other() { prepare_over_leftover(a, b, c); }",
        ),
        (
            "crates/local-server/src/board/mod.rs",
            "fn other(i: I, w: &W) { i.bind_over_leftover(w); }",
        ),
    ] {
        let original = sources[path].clone();
        sources.insert(path.to_owned(), format!("{original}\n{extra}"));
        assert!(mention_violation(&sources).is_some(), "{path} {extra}");
        sources.insert(path.to_owned(), original);
    }
    assert_eq!(mention_violation(&sources), None);

    let sole = &sources[SOLE];
    assert_eq!(credential_violation(sole), None);
    for removed in [
        "let broker = owner.as_ref().ok_or(LocalError::Unavailable)?;",
        ".closing",
        ".map_err(|_| LocalError::Configuration)?",
        "broker.validate_root_identities(",
    ] {
        assert!(sole.contains(removed), "{removed}");
        assert!(
            credential_violation(&sole.replacen(removed, "", 1)).is_some(),
            "{removed}"
        );
    }
    assert!(
        credential_violation(&format!(
            "{sole}\nfn forged() {{ SoleInstance {{ _owner: guard }}; }}"
        ))
        .is_some()
    );
}

#[test]
fn leftover_removal_keeps_every_judgement_and_grants_nothing_else() {
    let path = Path::new(LEFTOVER);
    let source = fs::read_to_string(workspace_root().join(path)).unwrap();
    assert!(source_violations(CRATE, path, &source).is_empty());
    // 每一步判断都钉住：少一步、放松一步都拿不到放行。
    for anchor in [
        "FileType::from_raw_mode(stat.st_mode) != FileType::Socket",
        "stat.st_uid != geteuid().as_raw()",
        "stat.st_nlink != 1",
        "!same(&stat, &by_path)",
        "if !probe(self.path) {",
        "if !same(&found, &self.candidate()?) {",
        "workbench.sole_instance().map_err(|_| Refused)?",
        "self.retire_leftover(&sole, unheard)?",
        "_sole: &SoleInstance<'_>",
        "let found = self.judge(probe)?;",
        "left.st_ctime_nsec == right.st_ctime_nsec",
        "left.st_ino == right.st_ino",
        "SocketFlags::NONBLOCK",
        "Err(Errno::CONNREFUSED)",
    ] {
        assert!(source.contains(anchor), "{anchor}");
        assert!(
            !source_violations(CRATE, path, &source.replacen(anchor, "removed", 1)).is_empty(),
            "{anchor}"
        );
    }
    for extra in [
        "unlinkat(other, name, AtFlags::empty());",
        "std::fs::remove_file(path);",
        "rustix::fs::unlink(path);",
        "fn other(e: Errno) -> bool { matches!(e, Errno::CONNREFUSED | Errno::AGAIN) }",
        "connect(&other, &address);",
    ] {
        assert!(
            !source_violations(CRATE, path, &format!("{source}\n{extra}")).is_empty(),
            "{extra}"
        );
    }
    // 换一个文件名、换一个 crate 都拿不到放行。
    for other in [
        "crates/local-server/src/unix_ingress/other.rs",
        "crates/local-server/src/leftover.rs",
    ] {
        assert!(!source_violations(CRATE, Path::new(other), &source).is_empty());
    }
    assert!(!source_violations("studiolo-auth", path, &source).is_empty());
}
