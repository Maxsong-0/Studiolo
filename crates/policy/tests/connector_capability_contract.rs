//! 连接器能力集（ADR-046）：`read.board_projects`、`write.presence_projects` 两张名单。
//!
//! 不用它们的类型规范化正文与摘要不变；用了其中一张，别的权限必须全空、两张不能同时非空；
//! 旧程序读到带新字段的正文只坏那一类。

use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

use serde_json::{Value, json};
use studiolo_policy::{
    CapabilitySet, DesiredInput, DesiredRecord, DesiredStatus, Expansion, PolicyAgentStore,
    PolicyKind, PolicyStore, Submitter,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-24T22:00:00Z";
const STUDIOLO: &str = "项目/Studiolo-Docs";
const HOMELAB: &str = "项目/HomeLab-Center-Docs";

fn private_dir() -> (TempDir, PathBuf) {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let dir = temp.path().join("policy-data");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    (temp, dir)
}

/// 一份什么都没有的能力集（和线上 monitor 同形）。
fn bare(ai_type: &str) -> Value {
    json!({
        "ai_type": ai_type,
        "read": {"project_whitelist": [], "deny_by_default": true, "rules_whitelist": []},
        "write": {"board": false, "board_projects": [], "content_patch_only": false, "content_direct": false},
        "policy": {"propose_desired": false, "activate": false},
        "hard_bound_skills": [],
    })
}

fn board(projects: &[&str]) -> Value {
    let mut set = bare("homelab-board");
    set["read"]["board_projects"] = json!(projects);
    set
}

fn presence(projects: &[&str]) -> Value {
    let mut set = bare("homelab-status");
    set["write"]["presence_projects"] = json!(projects);
    set
}

fn submit(store: &PolicyStore, body: &Value) -> DesiredRecord {
    store
        .submit_desired(
            DesiredInput {
                kind: PolicyKind::CapabilitySet,
                subject: body["ai_type"].as_str().unwrap().into(),
                body: Some(body.to_string()),
            },
            Submitter::Owner,
            NOW,
        )
        .unwrap()
}

fn publish(store: &PolicyStore, body: &Value) -> DesiredRecord {
    let record = submit(store, body);
    assert_eq!(
        record.status,
        DesiredStatus::Validated,
        "{:?}",
        record.report
    );
    let hash = store.effective().unwrap().effective_hash;
    store.activate(&record.id, &hash, "owner", NOW).unwrap();
    record
}

fn problems(record: &DesiredRecord) -> Vec<(String, String)> {
    record
        .report
        .as_ref()
        .unwrap()
        .problems
        .iter()
        .map(|problem| (problem.path.clone(), problem.message.clone()))
        .collect()
}

fn items<'a>(items: &'a [Expansion], path: &str) -> Vec<(Option<&'a str>, Option<&'a str>)> {
    items
        .iter()
        .filter(|item| item.path == path)
        .map(|item| (item.before.as_deref(), item.after.as_deref()))
        .collect()
}

#[test]
fn types_without_the_rosters_keep_their_canonical_body_byte_for_byte() {
    // 线上 claude／codex／monitor 三份的形状：规范化正文里不得冒出新字段，否则摘要与 effective 哈希会变。
    let claude = json!({
        "ai_type": "claude",
        "read": {"project_whitelist": [STUDIOLO], "deny_by_default": true, "rules_whitelist": ["CLAUDE.md"]},
        "write": {"board": false, "board_projects": [], "content_patch_only": true, "content_direct": false},
        "policy": {"propose_desired": true, "activate": false},
        "hard_bound_skills": [],
    });
    for body in [claude, bare("monitor")] {
        let set: CapabilitySet = serde_json::from_value(body.clone()).unwrap();
        assert!(!set.connector_only());
        assert_eq!(serde_json::to_value(&set).unwrap(), body);
        let text = serde_json::to_string(&set).unwrap();
        assert!(!text.contains("presence_projects"), "{text}");
        assert_eq!(text.matches("board_projects").count(), 1, "{text}");
    }
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    publish(&store, &bare("monitor"));
    let (set, _) = store.capability_set("monitor").unwrap().unwrap();
    assert!(!set.connector_only());
}

#[test]
fn a_connector_set_publishes_and_every_project_is_an_expansion() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let record = publish(&store, &board(&[STUDIOLO]));
    let report = record.report.as_ref().unwrap();
    assert_eq!(
        items(&report.expansions, "read.board_projects"),
        [(None, Some(STUDIOLO))]
    );
    let (set, _) = store.capability_set("homelab-board").unwrap().unwrap();
    assert!(set.connector_only());
    assert_eq!(set.read.board_projects, [STUDIOLO]);

    let record = publish(&store, &presence(&[STUDIOLO, HOMELAB]));
    let report = record.report.as_ref().unwrap();
    assert_eq!(
        items(&report.expansions, "write.presence_projects"),
        [(None, Some(STUDIOLO)), (None, Some(HOMELAB))]
    );

    // 换一个项目：一进一出各报一条；撤空名单只报收窄，且不再是连接器类型。
    let swapped = submit(&store, &board(&[HOMELAB]));
    let report = swapped.report.as_ref().unwrap();
    assert_eq!(
        items(&report.expansions, "read.board_projects"),
        [(None, Some(HOMELAB))]
    );
    assert_eq!(
        items(&report.narrowings, "read.board_projects"),
        [(Some(STUDIOLO), None)]
    );
    let emptied = submit(&store, &presence(&[]));
    let report = emptied.report.as_ref().unwrap();
    assert!(report.ok, "{report:?}");
    assert!(items(&report.expansions, "write.presence_projects").is_empty());
    assert_eq!(
        items(&report.narrowings, "write.presence_projects"),
        [(Some(STUDIOLO), None), (Some(HOMELAB), None)]
    );
}

#[test]
fn any_other_permission_next_to_a_roster_is_rejected_at_publish() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    type Change = fn(&mut Value);
    let mixes: [(&str, Change); 7] = [
        ("read.project_whitelist", |set| {
            set["read"]["project_whitelist"] = json!([STUDIOLO]);
        }),
        ("read.rules_whitelist", |set| {
            set["read"]["rules_whitelist"] = json!(["CLAUDE.md"]);
        }),
        ("write.board", |set| set["write"]["board"] = json!(true)),
        ("write.board_projects", |set| {
            set["write"]["board_projects"] = json!([STUDIOLO]);
        }),
        ("write.content_patch_only", |set| {
            set["write"]["content_patch_only"] = json!(true);
        }),
        ("policy.propose_desired", |set| {
            set["policy"]["propose_desired"] = json!(true);
        }),
        ("hard_bound_skills", |set| {
            set["hard_bound_skills"] = json!([{"when": "总是", "must_use": "x"}]);
        }),
    ];
    for base in [board(&[STUDIOLO]), presence(&[STUDIOLO])] {
        for (path, change) in mixes {
            let mut mixed = base.clone();
            change(&mut mixed);
            let record = submit(&store, &mixed);
            assert_eq!(record.status, DesiredStatus::Rejected, "{path}");
            assert!(
                problems(&record)
                    .contains(&(path.to_owned(), "连接器能力集里别的权限必须全空".to_owned())),
                "{path}: {:?}",
                problems(&record)
            );
        }
        // 对照：不混任何别的权限就放行。
        assert_eq!(submit(&store, &base).status, DesiredStatus::Validated);
    }
    let mut both = board(&[STUDIOLO]);
    both["write"]["presence_projects"] = json!([STUDIOLO]);
    let record = submit(&store, &both);
    assert_eq!(record.status, DesiredStatus::Rejected);
    assert_eq!(
        problems(&record),
        [(
            "write.presence_projects".to_owned(),
            "看黑板和推状态不能在同一份能力集里".to_owned()
        )]
    );
}

#[test]
fn roster_entries_must_be_unique_full_project_paths() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let bad = [
        "项目",
        "项目/",
        "项目/A/B",
        "_规则",
        "../x",
        ".obsidian/x",
        STUDIOLO,
        STUDIOLO,
    ];
    for (record, path) in [
        (submit(&store, &board(&bad)), "read.board_projects"),
        (submit(&store, &presence(&bad)), "write.presence_projects"),
    ] {
        assert_eq!(record.status, DesiredStatus::Rejected);
        let expected = (0..6)
            .map(|index| {
                (
                    format!("{path}[{index}]"),
                    "必须是合法的完整项目路径".to_owned(),
                )
            })
            .chain([(format!("{path}[7]"), "重复的项目".to_owned())])
            .collect::<Vec<_>>();
        assert_eq!(problems(&record), expected);
    }
}

/// 2026-11 之前发行的能力集形状：没有两张连接器名单。
mod before {
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    pub struct Read {
        project_whitelist: Vec<String>,
        deny_by_default: bool,
        #[serde(default)]
        rules_whitelist: Vec<String>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    pub struct Write {
        board: bool,
        #[serde(default)]
        board_projects: Vec<String>,
        content_patch_only: bool,
        content_direct: bool,
    }
}

#[test]
fn an_older_binary_reads_every_other_type_and_fails_closed_on_connector_types() {
    for (body, old_reads) in [
        (bare("monitor"), true),
        (board(&[]), true),
        (board(&[STUDIOLO]), false),
        (presence(&[STUDIOLO]), false),
    ] {
        let set: CapabilitySet = serde_json::from_value(body).unwrap();
        let canonical = serde_json::to_value(&set).unwrap();
        let read = serde_json::from_value::<before::Read>(canonical["read"].clone()).is_ok();
        let write = serde_json::from_value::<before::Write>(canonical["write"].clone()).is_ok();
        assert_eq!(read && write, old_reads, "{canonical}");
    }
}

#[test]
fn a_damaged_set_only_fails_its_own_type() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    publish(&store, &bare("monitor"));
    publish(&store, &board(&[STUDIOLO]));
    drop(store);
    // 换成这一版程序也读不懂的正文，等价于旧程序遇到新字段。
    let connection = rusqlite::Connection::open(dir.join("policy.sqlite3")).unwrap();
    let body = board(&[STUDIOLO])
        .to_string()
        .replace("board_projects", "future_projects");
    let changed = connection
        .execute(
            "UPDATE policy_version SET body = ?1 WHERE subject = 'homelab-board'",
            [body],
        )
        .unwrap();
    assert_eq!(changed, 1);
    drop(connection);
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    assert!(agent.authorization("homelab-board", None).is_err());
    let monitor = agent.authorization("monitor", None).unwrap();
    assert!(monitor.denial.is_none() && monitor.grant.is_some());
}
