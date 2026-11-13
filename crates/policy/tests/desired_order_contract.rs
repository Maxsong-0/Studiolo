//! 同一对象有多份未发布草案时的发布先后（T-181／BUG-031）。
//! 同一提交者（主人，或同一类 AI）有更新的未发布草案，较早那份就被拒、什么都不写；不同提交者
//! 互不阻挡（用户 2026-10-16 定：AI 交新提案挡不住主人）。发布只取代比它早的，比它晚的留着待处理。
//! 「谁更新」按入库先后判，不看提交时间字符串；列表给界面的先后与发布时的拦截同一口径，历史窗口
//! 截不掉任何一份未发布草案；判定找不到要发的那份就报库损坏，不放行。断线重提（T-182）拿回的
//! 也正是这一提交者能发布的那份：两边的「谁更新」出自同一处。
//! 2026-10-15 生产事故：同一类 AI 第一、二轮各两份规则提案都待处理，发布请求带的是第一轮
//! CLAUDE.md，发布事务照发，还把更新的第二轮 CLAUDE.md 标成了「已取代」。

use std::{collections::BTreeSet, fs, os::unix::fs::PermissionsExt, path::PathBuf};

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

/// AI（claude 类型的同一个实例）交一份规则提案。
fn propose(agent: &PolicyAgentStore, subject: &str, body: &str, at: &str) -> DesiredRecord {
    propose_kind(agent, PolicyKind::Rules, subject, body, at)
}

/// AI（claude 类型的同一个实例）交一份任意类别的提案；结构无效的入库为 rejected。
fn propose_kind(
    agent: &PolicyAgentStore,
    kind: PolicyKind,
    subject: &str,
    body: &str,
    at: &str,
) -> DesiredRecord {
    propose_from(agent, ("claude", "01INSTANCE"), kind, subject, body, at)
}

/// 指定 AI 类型与实例交一份提案。「同一提交者」按类型认，与实例无关。
fn propose_from(
    agent: &PolicyAgentStore,
    (agent_type, instance): (&str, &str),
    kind: PolicyKind,
    subject: &str,
    body: &str,
    at: &str,
) -> DesiredRecord {
    agent
        .propose(
            agent_type,
            instance,
            ProposalInput {
                kind,
                subject: subject.into(),
                body: body.into(),
            },
            at,
        )
        .unwrap()
        .record
}

/// claude 类型的能力集正文：只读这几个项目，别的一律关。
fn capability(projects: &[&str]) -> String {
    serde_json::json!({
        "ai_type": "claude",
        "read": {"project_whitelist": projects, "deny_by_default": true},
        "write": {"board": false, "content_patch_only": true, "content_direct": false},
        "policy": {"propose_desired": false, "activate": false},
    })
    .to_string()
}

/// 按当前 effective 发布；调用方不必先读哈希。
fn publish(store: &PolicyStore, id: &str, at: &str) -> Result<Activation, PolicyError> {
    let hash = store.effective().unwrap().effective_hash;
    store.activate(id, &hash, "owner", at)
}

fn status(store: &PolicyStore, id: &str) -> DesiredStatus {
    store.desired(id).unwrap().unwrap().status
}

/// 发布被拒，拒绝原因按入库先后列出挡住它的、同一提交者更新的那几份。
fn assert_blocked(result: Result<Activation, PolicyError>, newer: &[&DesiredRecord]) {
    let newer = newer.iter().map(|record| record.id.clone()).collect();
    assert_eq!(
        result,
        Err(PolicyError::NewerDesiredFromSameSubmitter { newer })
    );
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
fn only_the_same_submitters_newer_drafts_block_and_withdrawing_them_lets_the_older_one_publish() {
    let (_temp, dir, store) = store_with_rules();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    // 同一类 AI（claude）先后三份提案，中间夹着主人的一份。
    let first = propose(&agent, CLAUDE, "# 一\n", "2026-10-15T18:00:00Z");
    let second = propose(&agent, CLAUDE, "# 二\n", "2026-10-15T18:01:00Z");
    let owner = submit(
        &store,
        PolicyKind::Rules,
        CLAUDE,
        "# 主人\n",
        "2026-10-15T18:02:00Z",
    );
    let third = propose(&agent, CLAUDE, "# 三\n", "2026-10-15T18:03:00Z");
    // 挡住的只有同一提交者更新的那几份，按入库先后列出；主人那份不算。
    assert_blocked(
        publish(&store, &first.id, "2026-10-15T18:04:00Z"),
        &[&second, &third],
    );
    assert_blocked(
        publish(&store, &second.id, "2026-10-15T18:04:00Z"),
        &[&third],
    );
    // 人明确撤回同一提交者更新的两份之后，最早那份就能发。
    store
        .withdraw_desired(&third.id, "2026-10-15T18:05:00Z")
        .unwrap();
    store
        .withdraw_desired(&second.id, "2026-10-15T18:05:00Z")
        .unwrap();
    publish(&store, &first.id, "2026-10-15T18:06:00Z").unwrap();
    assert_eq!(status(&store, &first.id), DesiredStatus::Activated);
    // 主人那份比它晚入库，照旧待处理，没被悄悄作废。
    assert_eq!(status(&store, &owner.id), DesiredStatus::Validated);
    assert_eq!(status(&store, &second.id), DesiredStatus::Withdrawn);
    assert_eq!(status(&store, &third.id), DesiredStatus::Withdrawn);
}

#[test]
fn ai_types_are_separate_submitters_but_instances_of_one_type_are_the_same_submitter() {
    let (_temp, dir, store) = store_with_rules();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    // claude 的两个实例先后各交一份：同一类 AI 算同一提交者。
    let first = propose_from(
        &agent,
        ("claude", "01INSTANCEA"),
        PolicyKind::Rules,
        CLAUDE,
        "# claude 甲\n",
        "2026-10-15T18:00:00Z",
    );
    let second = propose_from(
        &agent,
        ("claude", "01INSTANCEB"),
        PolicyKind::Rules,
        CLAUDE,
        "# claude 乙\n",
        "2026-10-15T18:01:00Z",
    );
    // codex 之后交的一份：别的类型，不挡 claude 的，也不被 claude 的取代。
    let codex = propose_from(
        &agent,
        ("codex", "01INSTANCEC"),
        PolicyKind::Rules,
        CLAUDE,
        "# codex\n",
        "2026-10-15T18:02:00Z",
    );
    assert_blocked(
        publish(&store, &first.id, "2026-10-15T18:03:00Z"),
        &[&second],
    );
    let (_, _, blocked) = store.desired_queue(200).unwrap();
    assert_eq!(blocked[&first.id], [second.id.as_str()]);
    assert!(blocked[&second.id].is_empty());
    assert!(blocked[&codex.id].is_empty());
    publish(&store, &second.id, "2026-10-15T18:04:00Z").unwrap();
    assert_eq!(status(&store, &first.id), DesiredStatus::Superseded);
    assert_eq!(status(&store, &codex.id), DesiredStatus::Pending);
}

#[test]
fn a_newer_rejected_draft_from_the_same_submitter_also_blocks_until_withdrawn() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let valid = submit(
        &store,
        PolicyKind::CapabilitySet,
        "claude",
        &capability(&["项目/Studiolo"]),
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
    // 主人自己校验没过的新草案也算数：它说明较早那份已被改过，要发较早那份，先撤回它。
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
fn the_queue_marks_newer_and_blocking_drafts_with_the_same_rule_as_publishing() {
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
    let owner = submit(
        &store,
        PolicyKind::Rules,
        CLAUDE,
        "# 主人\n",
        "2026-10-15T18:10:00Z",
    );
    let (records, newer, blocked) = store.desired_queue(200).unwrap();
    // 列表照旧含两份已发布的现行规则；四份未发布的都带表，最新那份是空表。
    assert_eq!(records.len(), 6);
    assert_eq!(newer.len(), 4);
    assert_eq!(
        newer[&r1_claude.id],
        [r2_claude.id.as_str(), owner.id.as_str()]
    );
    assert_eq!(newer[&r2_claude.id], [owner.id.as_str()]);
    assert!(newer[&owner.id].is_empty());
    assert!(newer[&r1_rules.id].is_empty());
    // 挡住发布的只算同一提交者的：第一轮被第二轮挡；主人那份不挡别人，也不被别人挡。
    assert_eq!(blocked.len(), 4);
    assert_eq!(blocked[&r1_claude.id], [r2_claude.id.as_str()]);
    assert!(blocked[&r2_claude.id].is_empty());
    assert!(blocked[&owner.id].is_empty());
    assert!(blocked[&r1_rules.id].is_empty());
    // 表里说被挡的那份，发布时就被拦住；没被挡的能发，发了只取代比它早的。
    assert_blocked(
        publish(&store, &r1_claude.id, "2026-10-15T18:21:09Z"),
        &[&r2_claude],
    );
    publish(&store, &r2_claude.id, "2026-10-15T18:21:09Z").unwrap();
    assert_eq!(status(&store, &r1_claude.id), DesiredStatus::Superseded);
    assert_eq!(status(&store, &owner.id), DesiredStatus::Validated);
    let open = BTreeSet::from([r1_rules.id.clone(), owner.id.clone()]);
    let (_, newer, blocked) = store.desired_queue(200).unwrap();
    assert_eq!(newer.keys().cloned().collect::<BTreeSet<_>>(), open);
    assert_eq!(blocked.keys().cloned().collect::<BTreeSet<_>>(), open);
    // limit 只截已结束的历史：落在窗口外的未发布草案照样列出（补在末尾），表里也有它。
    let (records, newer, _) = store.desired_queue(1).unwrap();
    let ids: Vec<&str> = records.iter().map(|record| record.id.as_str()).collect();
    assert_eq!(ids, [owner.id.as_str(), r1_rules.id.as_str()]);
    assert_eq!(newer.keys().cloned().collect::<BTreeSet<_>>(), open);
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
    let (records, newer_open, blocked_by) = store.desired_queue(2).unwrap();
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
    assert_eq!(blocked_by[&older.id], [newer.id.as_str()]);
    assert_blocked(
        publish(&store, &older.id, "2026-10-15T14:00:00Z"),
        &[&newer],
    );
}

#[test]
fn an_agents_newer_proposals_neither_block_the_owner_nor_get_superseded() {
    // 安全审查（2026-10-15）：AI 能对任何对象提案（含能力集），被拒的提案还不占名额；要是谁更新
    // 都挡，AI 不停交新提案就能挡住主人，比如主人要收窄它的能力集。
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    let narrowed = submit(
        &store,
        PolicyKind::CapabilitySet,
        "claude",
        &capability(&["项目/Studiolo"]),
        "2026-10-15T18:00:00Z",
    );
    assert_eq!(narrowed.status, DesiredStatus::Validated);
    // 之后 AI 对同一能力集交了一份有效的、一份结构无效的（入库为被拒）。
    let wider = propose_kind(
        &agent,
        PolicyKind::CapabilitySet,
        "claude",
        &capability(&["项目/Studiolo", "项目/Semiment"]),
        "2026-10-15T18:01:00Z",
    );
    let broken = propose_kind(
        &agent,
        PolicyKind::CapabilitySet,
        "claude",
        "{}",
        "2026-10-15T18:02:00Z",
    );
    assert_eq!(wider.status, DesiredStatus::Pending);
    assert_eq!(broken.status, DesiredStatus::Rejected);
    // 主人发布自己较早的那份：不被挡。
    let activation = publish(&store, &narrowed.id, "2026-10-15T18:03:00Z").unwrap();
    assert_eq!(activation.desired_id.as_deref(), Some(narrowed.id.as_str()));
    // 两份更新的提案照旧在，没被悄悄作废，主人之后逐份处理。
    assert_eq!(status(&store, &wider.id), DesiredStatus::Pending);
    assert_eq!(status(&store, &broken.id), DesiredStatus::Rejected);
}

#[test]
fn publishing_supersedes_only_the_older_drafts_and_keeps_the_newer_ones() {
    let (_temp, dir, store) = store_with_rules();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    let oldest = propose(&agent, CLAUDE, "# 最早的提案\n", "2026-10-15T18:00:00Z");
    let owner = submit(
        &store,
        PolicyKind::Rules,
        CLAUDE,
        "# 主人改的\n",
        "2026-10-15T18:01:00Z",
    );
    let newest = propose(&agent, CLAUDE, "# 最新的提案\n", "2026-10-15T18:02:00Z");
    // 主人那份：同一提交者没有更新的，能发；比它早的那份标为已取代，比它晚的那份留着。
    publish(&store, &owner.id, "2026-10-15T18:03:00Z").unwrap();
    assert_eq!(status(&store, &owner.id), DesiredStatus::Activated);
    assert_eq!(status(&store, &oldest.id), DesiredStatus::Superseded);
    assert_eq!(status(&store, &newest.id), DesiredStatus::Pending);
    // 留下的那份之后照样能发。
    publish(&store, &newest.id, "2026-10-15T18:04:00Z").unwrap();
    assert_eq!(status(&store, &newest.id), DesiredStatus::Activated);
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

#[test]
fn a_retry_and_the_publish_rule_agree_on_the_submitters_newest_draft() {
    let (_temp, dir, store) = store_with_rules();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    let first = propose(&agent, CLAUDE, "# 一\n", "2026-10-15T18:00:00Z");
    let second = propose(&agent, CLAUDE, "# 二\n", "2026-10-15T18:01:00Z");
    let owner = submit(
        &store,
        PolicyKind::Rules,
        CLAUDE,
        "# 主人\n",
        "2026-10-15T18:02:00Z",
    );
    // 断线重连后是另一个实例。
    let retry = |body: &str, at: &str| {
        agent
            .propose(
                "claude",
                "01INSTANCEB",
                ProposalInput {
                    kind: PolicyKind::Rules,
                    subject: CLAUDE.into(),
                    body: body.into(),
                },
                at,
            )
            .unwrap()
    };
    // 重提第二版：主人之后交的那份不算，拿回的正是没被挡住、能发布的第二版。
    let again = retry("# 二\n", "2026-10-15T18:03:00Z");
    assert!(again.deduplicated);
    assert_eq!(again.record.id, second.id);
    let (_, _, blocked) = store.desired_queue(200).unwrap();
    assert!(blocked[&second.id].is_empty());
    assert_eq!(blocked[&first.id], [second.id.as_str()]);
    // 重提第一版：它已被同一提交者的第二版挡住，拿不回；新建的那份成了最新的，挡住前两份。
    let third = retry("# 一\n", "2026-10-15T18:04:00Z");
    assert!(!third.deduplicated);
    let (_, _, blocked) = store.desired_queue(200).unwrap();
    assert_eq!(
        blocked[&first.id],
        [second.id.as_str(), third.record.id.as_str()]
    );
    assert_eq!(blocked[&second.id], [third.record.id.as_str()]);
    assert!(blocked[&third.record.id].is_empty());
    assert_blocked(
        publish(&store, &second.id, "2026-10-15T18:05:00Z"),
        &[&third.record],
    );
    publish(&store, &third.record.id, "2026-10-15T18:06:00Z").unwrap();
    // 比它早入库的都被取代，主人那份也在内。
    for earlier in [&first, &second, &owner] {
        assert_eq!(status(&store, &earlier.id), DesiredStatus::Superseded);
    }
}
