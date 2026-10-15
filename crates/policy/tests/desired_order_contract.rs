//! 同一对象有多份未发布草案时的发布先后（T-181／BUG-031）：只能发布最新的那一份。
//! 较早那份被拒、什么都不写；「谁更新」按入库先后判，不看提交时间字符串；
//! 列表给界面的「更新的草案」与发布时的拦截同一口径，历史窗口截不掉任何一份未发布草案；
//! 判定找不到要发的那份就报库损坏，不放行。
//! 2026-10-15 生产事故：第一、二轮各两份规则提案都待处理，发布请求带的是第一轮 CLAUDE.md，
//! 发布事务照发，还把更新的第二轮 CLAUDE.md 标成了「已取代」。

use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

use studiolo_policy::{
    Activation, DesiredInput, DesiredRecord, DesiredStatus, PolicyAgentStore, PolicyError,
    PolicyKind, PolicyStore, ProposalInput, Submitter,
};
use tempfile::TempDir;

const CLAUDE: &str = "CLAUDE.md";
const RULES: &str = "通用工作规则.md";

fn private_dir() -> (TempDir, PathBuf) {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let dir = temp.path().join("policy-data");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    (temp, dir)
}

fn submit(
    store: &PolicyStore,
    kind: PolicyKind,
    subject: &str,
    body: &str,
    at: &str,
) -> DesiredRecord {
    store
        .submit_desired(
            DesiredInput {
                kind,
                subject: subject.into(),
                body: Some(body.into()),
            },
            Submitter::Owner,
            at,
        )
        .unwrap()
}

fn propose(agent: &PolicyAgentStore, subject: &str, body: &str, at: &str) -> DesiredRecord {
    agent
        .propose(
            "claude",
            "01INSTANCE",
            ProposalInput {
                kind: PolicyKind::Rules,
                subject: subject.into(),
                body: body.into(),
            },
            at,
        )
        .unwrap()
}

/// 按当前 effective 发布；调用方不必先读哈希。
fn publish(store: &PolicyStore, id: &str, at: &str) -> Result<Activation, PolicyError> {
    let hash = store.effective().unwrap().effective_hash;
    store.activate(id, &hash, "owner", at)
}

fn status(store: &PolicyStore, id: &str) -> DesiredStatus {
    store.desired(id).unwrap().unwrap().status
}

/// 发布被拒，拒绝原因按入库先后列出同一对象更新的那几份。
fn assert_blocked(result: Result<Activation, PolicyError>, newer: &[&DesiredRecord]) {
    let newer = newer.iter().map(|record| record.id.clone()).collect();
    assert_eq!(result, Err(PolicyError::NewerDesiredExists { newer }));
}

/// 两份规则各发布一版，作为提案的比较基准。
fn store_with_rules() -> (TempDir, PathBuf, PolicyStore) {
    let (temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    for subject in [CLAUDE, RULES] {
        let record = submit(
            &store,
            PolicyKind::Rules,
            subject,
            "# 现行\n",
            "2026-10-15T00:00:00Z",
        );
        publish(&store, &record.id, "2026-10-15T00:00:00Z").unwrap();
    }
    (temp, dir, store)
}

#[test]
fn an_older_round_cannot_be_published_over_a_newer_round() {
    let (_temp, dir, store) = store_with_rules();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    // 与生产同形：两轮各两份，时间戳取自 2026-10-15 那四条提案。
    let r1_claude = propose(
        &agent,
        CLAUDE,
        "# 第一轮\n",
        "2026-10-15T17:55:04.842284466Z",
    );
    let r1_rules = propose(
        &agent,
        RULES,
        "# 第一轮\n",
        "2026-10-15T17:55:05.476768403Z",
    );
    let r2_claude = propose(
        &agent,
        CLAUDE,
        "# 第二轮\n",
        "2026-10-15T18:05:21.234156859Z",
    );
    let r2_rules = propose(
        &agent,
        RULES,
        "# 第二轮\n",
        "2026-10-15T18:05:21.800350501Z",
    );
    let before = store.effective().unwrap();
    let versions = store.versions(None, None, 100).unwrap().len();

    // 点中较早那份：拒绝，并说出更新的是哪一份。
    assert_blocked(
        publish(&store, &r1_claude.id, "2026-10-15T18:21:09Z"),
        &[&r2_claude],
    );
    // 什么都没写：effective、版本数、四份提案的状态都没变。
    assert_eq!(
        store.effective().unwrap().effective_hash,
        before.effective_hash
    );
    assert_eq!(store.versions(None, None, 100).unwrap().len(), versions);
    for record in [&r1_claude, &r1_rules, &r2_claude, &r2_rules] {
        assert_eq!(
            status(&store, &record.id),
            DesiredStatus::Pending,
            "{}",
            record.id
        );
    }

    // 发布最新那份：照旧发布，较早那份标为已取代。
    let activation = publish(&store, &r2_claude.id, "2026-10-15T18:21:09Z").unwrap();
    assert_eq!(
        activation.desired_id.as_deref(),
        Some(r2_claude.id.as_str())
    );
    assert_eq!(status(&store, &r2_claude.id), DesiredStatus::Activated);
    assert_eq!(status(&store, &r1_claude.id), DesiredStatus::Superseded);
    // 另一个对象各管各的：它自己较早那份同样发不出，最新那份照发。
    assert_blocked(
        publish(&store, &r1_rules.id, "2026-10-15T18:21:13Z"),
        &[&r2_rules],
    );
    publish(&store, &r2_rules.id, "2026-10-15T18:21:13Z").unwrap();
    assert_eq!(status(&store, &r1_rules.id), DesiredStatus::Superseded);
    assert_eq!(status(&store, &r2_rules.id), DesiredStatus::Activated);
}

#[test]
fn every_newer_open_draft_is_named_and_withdrawing_them_lets_the_older_one_publish() {
    let (_temp, dir, store) = store_with_rules();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    let first = propose(&agent, CLAUDE, "# 一\n", "2026-10-15T18:00:00Z");
    let second = propose(&agent, CLAUDE, "# 二\n", "2026-10-15T18:01:00Z");
    let third = submit(
        &store,
        PolicyKind::Rules,
        CLAUDE,
        "# 三\n",
        "2026-10-15T18:02:00Z",
    );
    assert_blocked(
        publish(&store, &first.id, "2026-10-15T18:03:00Z"),
        &[&second, &third],
    );
    // 中间那份只被最新的挡住。
    assert_blocked(
        publish(&store, &second.id, "2026-10-15T18:03:00Z"),
        &[&third],
    );
    // 人明确撤回更新的两份之后，最早那份就是最新的，可以发布。
    store
        .withdraw_desired(&third.id, "2026-10-15T18:04:00Z")
        .unwrap();
    store
        .withdraw_desired(&second.id, "2026-10-15T18:04:00Z")
        .unwrap();
    publish(&store, &first.id, "2026-10-15T18:05:00Z").unwrap();
    assert_eq!(status(&store, &first.id), DesiredStatus::Activated);
    assert_eq!(status(&store, &second.id), DesiredStatus::Withdrawn);
    assert_eq!(status(&store, &third.id), DesiredStatus::Withdrawn);
}

#[test]
fn a_newer_rejected_draft_also_blocks_until_it_is_withdrawn() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let valid = submit(
        &store,
        PolicyKind::CapabilitySet,
        "claude",
        &serde_json::json!({
            "ai_type": "claude",
            "read": {"project_whitelist": ["项目/Studiolo"], "deny_by_default": true},
            "write": {"board": false, "content_patch_only": true, "content_direct": false},
            "policy": {"propose_desired": false, "activate": false},
        })
        .to_string(),
        "2026-10-15T18:00:00Z",
    );
    assert_eq!(valid.status, DesiredStatus::Validated);
    let broken = submit(
        &store,
        PolicyKind::CapabilitySet,
        "claude",
        "{}",
        "2026-10-15T18:01:00Z",
    );
    assert_eq!(broken.status, DesiredStatus::Rejected);
    // 校验没过的新草案也是一份没处理的提议，不能被较早那份悄悄作废。
    assert_blocked(
        publish(&store, &valid.id, "2026-10-15T18:02:00Z"),
        &[&broken],
    );
    assert_eq!(status(&store, &broken.id), DesiredStatus::Rejected);
    store
        .withdraw_desired(&broken.id, "2026-10-15T18:03:00Z")
        .unwrap();
    publish(&store, &valid.id, "2026-10-15T18:04:00Z").unwrap();
    assert_eq!(status(&store, &valid.id), DesiredStatus::Activated);
}

#[test]
fn newer_means_inserted_later_even_when_the_timestamps_disagree() {
    let (_temp, dir, store) = store_with_rules();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    // 时钟回拨或调用方时钟不一致：后入库的那份时间戳反而更早，它仍是更新的那份。
    let earlier = propose(&agent, CLAUDE, "# 先入库\n", "2026-10-15T19:00:00Z");
    let later = propose(&agent, CLAUDE, "# 后入库\n", "2026-10-15T18:00:00Z");
    assert_blocked(
        publish(&store, &earlier.id, "2026-10-15T19:30:00Z"),
        &[&later],
    );
    // 同一时刻提交的两份同样按入库先后，不按 id 排（同一毫秒里生成的 id 先后随机）。
    let first = propose(&agent, RULES, "# 同刻一\n", "2026-10-15T18:00:00Z");
    let second = propose(&agent, RULES, "# 同刻二\n", "2026-10-15T18:00:00Z");
    assert_blocked(
        publish(&store, &first.id, "2026-10-15T19:30:00Z"),
        &[&second],
    );
    publish(&store, &later.id, "2026-10-15T19:30:00Z").unwrap();
    assert_eq!(status(&store, &earlier.id), DesiredStatus::Superseded);
}

#[test]
fn the_queue_marks_newer_open_drafts_with_the_same_rule_as_publishing() {
    let (_temp, dir, store) = store_with_rules();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    let r1_claude = propose(
        &agent,
        CLAUDE,
        "# 第一轮\n",
        "2026-10-15T17:55:04.842284466Z",
    );
    let r1_rules = propose(
        &agent,
        RULES,
        "# 第一轮\n",
        "2026-10-15T17:55:05.476768403Z",
    );
    let r2_claude = propose(
        &agent,
        CLAUDE,
        "# 第二轮\n",
        "2026-10-15T18:05:21.234156859Z",
    );
    let (records, newer) = store.desired_queue(200).unwrap();
    // 列表照旧含两份已发布的现行规则；只有三份未发布的带表，最新那份是空表。
    assert_eq!(records.len(), 5);
    assert_eq!(newer.len(), 3);
    assert_eq!(newer[&r1_claude.id], [r2_claude.id.as_str()]);
    assert!(newer[&r2_claude.id].is_empty());
    assert!(newer[&r1_rules.id].is_empty());
    // 表里说有更新的那份，发布时就被拦住；空表的那份能发。
    assert_blocked(
        publish(&store, &r1_claude.id, "2026-10-15T18:21:09Z"),
        &[&r2_claude],
    );
    publish(&store, &r2_claude.id, "2026-10-15T18:21:09Z").unwrap();
    let (_, newer) = store.desired_queue(200).unwrap();
    assert_eq!(newer.keys().collect::<Vec<_>>(), [&r1_rules.id]);
    // limit 只截已结束的历史：落在窗口外的未发布草案照样列出（补在末尾），表里也有它。
    let (records, newer) = store.desired_queue(1).unwrap();
    let ids: Vec<&str> = records.iter().map(|record| record.id.as_str()).collect();
    assert_eq!(ids, [r2_claude.id.as_str(), r1_rules.id.as_str()]);
    assert_eq!(newer.keys().collect::<Vec<_>>(), [&r1_rules.id]);
}

#[test]
fn the_history_window_never_drops_an_open_draft() {
    let (_temp, dir, store) = store_with_rules();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    // 同一对象两份未发布提案，后入库那份时间戳反而更早（时钟回拨）。
    let older = propose(&agent, CLAUDE, "# 先入库\n", "2026-10-15T10:00:00Z");
    let newer = propose(&agent, CLAUDE, "# 后入库\n", "2026-10-15T09:00:00Z");
    // 之后另一对象发布了两版：按提交时间倒序的窗口（limit 2）只容得下这两份已发布的。
    for at in ["2026-10-15T12:00:00Z", "2026-10-15T13:00:00Z"] {
        let record = submit(&store, PolicyKind::Rules, RULES, &format!("# {at}\n"), at);
        publish(&store, &record.id, at).unwrap();
    }
    let (records, newer_open) = store.desired_queue(2).unwrap();
    let listed: Vec<(&str, DesiredStatus)> = records
        .iter()
        .map(|record| (record.id.as_str(), record.status))
        .collect();
    // 窗口里的两份已发布记录在前；窗口外的未发布草案按同一排序（提交时间倒序）补在末尾，
    // 窗口外已结束的旧记录不补。
    assert_eq!(listed.len(), 4, "{listed:?}");
    assert!(
        listed[..2]
            .iter()
            .all(|(_, status)| *status == DesiredStatus::Activated)
    );
    assert_eq!(
        listed[2..],
        [
            (older.id.as_str(), DesiredStatus::Pending),
            (newer.id.as_str(), DesiredStatus::Pending),
        ]
    );
    // 两份都在表里，先后照样按入库：挡住发布的那份在列表里找得到、撤得掉。
    assert_eq!(newer_open.len(), 2);
    assert_eq!(newer_open[&older.id], [newer.id.as_str()]);
    assert!(newer_open[&newer.id].is_empty());
    assert_blocked(
        publish(&store, &older.id, "2026-10-15T14:00:00Z"),
        &[&newer],
    );
}

#[test]
fn a_draft_the_ordering_cannot_place_is_refused_as_a_corrupt_store() {
    let (_temp, dir, store) = store_with_rules();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    let older = propose(&agent, CLAUDE, "# 较早\n", "2026-10-15T17:55:04Z");
    let newer = propose(&agent, CLAUDE, "# 更新\n", "2026-10-15T18:05:21Z");
    {
        // 绕过 CHECK 约束写坏类别：行映射把认不出的类别读成 rules，按类别查的先后判定就找不到这份。
        let connection = rusqlite::Connection::open(dir.join("policy.sqlite3")).unwrap();
        connection
            .execute_batch("PRAGMA ignore_check_constraints = ON")
            .unwrap();
        connection
            .execute(
                "UPDATE policy_desired SET kind = 'bogus' WHERE id = ?1",
                [&older.id],
            )
            .unwrap();
    }
    let before = store.effective().unwrap().effective_hash;
    let versions = store.versions(None, None, 100).unwrap().len();
    // 判定找不到它就报库损坏，绝不当成「没有更新的」照发、再把更新的那份标成已取代。
    assert_eq!(
        publish(&store, &older.id, "2026-10-15T18:21:09Z"),
        Err(PolicyError::CorruptStore)
    );
    assert_eq!(store.effective().unwrap().effective_hash, before);
    assert_eq!(store.versions(None, None, 100).unwrap().len(), versions);
    assert_eq!(status(&store, &newer.id), DesiredStatus::Pending);
}
