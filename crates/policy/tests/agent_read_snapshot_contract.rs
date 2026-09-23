//! 接入方读取的一致性契约：规则母本、skills 与它们声明的 effective 哈希取自同一读事务。
//!
//! 人侧进程随时可能发布新版本；接入方若先读哈希、再另开事务读条目，版本行与条目就会各属一版。

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use studiolo_policy::{DesiredInput, PolicyAgentStore, PolicyKind, PolicyStore, Submitter};
use tempfile::TempDir;

const NOW: &str = "2026-09-22T22:00:00Z";
const SUBJECT: &str = "通用工作规则.md";

fn private_dir() -> (TempDir, PathBuf) {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let dir = temp.path().join("policy-data");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    (temp, dir)
}

/// 发布一版并返回 (新 effective 哈希, 版本号)。
fn publish(store: &PolicyStore, kind: PolicyKind, subject: &str, body: &str) -> (String, u64) {
    let record = store
        .submit_desired(
            DesiredInput {
                kind,
                subject: subject.into(),
                body: Some(body.into()),
            },
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    let hash = store.effective().unwrap().effective_hash;
    let activation = store.activate(&record.id, &hash, "owner", NOW).unwrap();
    (activation.effective_hash, activation.version)
}

fn rule(generation: usize) -> String {
    format!("# 规则\n\n## 第 {generation} 版\n\n禁止绕过式修复。\n")
}

fn skill(markdown: &str) -> String {
    serde_json::json!({
        "name": "react-perf",
        "summary": "React 性能规范",
        "triggers": ["写或改 React / Next.js 代码"],
        "markdown": markdown,
    })
    .to_string()
}

#[test]
fn every_agent_read_carries_the_hash_its_entries_belong_to() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let (first_hash, first_version) = publish(&store, PolicyKind::Rules, SUBJECT, &rule(1));
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();

    let rules = agent.rules().unwrap();
    assert_eq!(rules.effective_hash, first_hash);
    assert_eq!(rules.value.len(), 1);
    assert_eq!(rules.value[0].meta.version, first_version);
    let skills = agent.skills().unwrap();
    assert_eq!(skills.effective_hash, first_hash);
    assert!(skills.value.is_empty());

    let (second_hash, _) = publish(&store, PolicyKind::Skills, "react-perf", &skill("正文"));
    assert_ne!(second_hash, first_hash);
    let both = agent.rules_and_skills().unwrap();
    assert_eq!(both.effective_hash, second_hash);
    assert_eq!(both.value.rules[0].meta.version, first_version);
    assert_eq!(both.value.skills.len(), 1);
    assert_eq!(both.value.skills[0].0.name, "react-perf");
    assert_eq!(agent.skills().unwrap().effective_hash, second_hash);
    assert_eq!(agent.rules().unwrap().effective_hash, second_hash);
}

#[test]
fn a_publish_racing_the_read_never_tears_hash_from_entries() {
    const PUBLISHES: usize = 40;
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let (first_hash, first_version) = publish(&store, PolicyKind::Rules, SUBJECT, &rule(0));
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();

    let started = Arc::new(Barrier::new(2));
    let done = Arc::new(AtomicBool::new(false));
    let reader = {
        let (started, done) = (Arc::clone(&started), Arc::clone(&done));
        thread::spawn(move || {
            let observe = || {
                let read = agent.rules_and_skills().unwrap();
                assert_eq!(read.value.rules.len(), 1);
                (read.effective_hash, read.value.rules[0].meta.version)
            };
            // 发布开始前先读一次，结束后再读一次：至少见到首尾两个不同的哈希，证明读与写确实交错过。
            let mut seen = vec![observe()];
            started.wait();
            while !done.load(Ordering::Acquire) {
                seen.push(observe());
            }
            seen.push(observe());
            seen
        })
    };
    started.wait();
    let mut published = BTreeMap::from([(first_hash, first_version)]);
    for generation in 1..=PUBLISHES {
        let (hash, version) = publish(&store, PolicyKind::Rules, SUBJECT, &rule(generation));
        assert!(published.insert(hash, version).is_none());
    }
    done.store(true, Ordering::Release);
    let seen = reader.join().unwrap();

    // 每次读到的哈希都必须正是它所附条目那一版发布后的哈希。
    for (hash, version) in &seen {
        assert_eq!(
            published.get(hash),
            Some(version),
            "撕裂读：{hash} 配 v{version}"
        );
    }
    let distinct = seen.iter().map(|(hash, _)| hash).collect::<BTreeSet<_>>();
    assert!(
        distinct.len() >= 2,
        "读取没有与发布交错：只见到 {distinct:?}"
    );
    eprintln!(
        "a_publish_racing_the_read: {} 次读取，见到 {} 个不同哈希（共发布 {} 版）",
        seen.len(),
        distinct.len(),
        published.len()
    );
}
