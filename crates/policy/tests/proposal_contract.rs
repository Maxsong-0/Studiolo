//! T-182 接入方提案的断线重提：本类型在同一 kind/subject 上最新的草案若正是同一规范化正文、仍待处理的
//! 那份，再提时原样返回、不新建、不占名额；主人和别的类型的草案不影响判定；其余情况照常有界入库。
//! 列表只给本类型仍待处理（pending／validated）的接入方提案，新到旧，有上限。

use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, sync::Barrier, thread};

use studiolo_policy::{
    DesiredInput, DesiredRecord, DesiredStatus, MAX_BODY_BYTES, MAX_LISTED_PROPOSALS,
    MAX_PENDING_PER_INSTANCE, MAX_PROPOSAL_BYTES, PolicyAgentStore, PolicyError, PolicyKind,
    PolicyStore, Proposal, ProposalInput, Submitter, sha256_hex,
};
use tempfile::TempDir;

const NOW: &str = "2026-10-15T20:04:20Z";

struct Stores {
    _temp: TempDir,
    /// 策略库目录：并发用例在这里另开接入方句柄（各自一条连接）。
    dir: PathBuf,
    owner: PolicyStore,
    agent: PolicyAgentStore,
}

fn stores() -> Stores {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let dir: PathBuf = temp.path().join("policy-data");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let owner = PolicyStore::open(&dir).unwrap();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    Stores {
        _temp: temp,
        dir,
        owner,
        agent,
    }
}

impl Stores {
    fn propose(
        &self,
        agent_type: &str,
        instance: &str,
        kind: PolicyKind,
        subject: &str,
        body: &str,
    ) -> Proposal {
        self.agent
            .propose(
                agent_type,
                instance,
                ProposalInput {
                    kind,
                    subject: subject.into(),
                    body: body.into(),
                },
                NOW,
            )
            .unwrap()
    }

    fn rule(&self, agent_type: &str, instance: &str, subject: &str, body: &str) -> Proposal {
        self.propose(agent_type, instance, PolicyKind::Rules, subject, body)
    }

    /// 主人在人侧提交的一份规则草案。
    fn owner_rule(&self, subject: &str, body: &str) -> DesiredRecord {
        self.owner
            .submit_desired(
                DesiredInput {
                    kind: PolicyKind::Rules,
                    subject: subject.into(),
                    body: Some(body.into()),
                },
                Submitter::Owner,
                NOW,
            )
            .unwrap()
    }

    /// 某 subject 下所有 desired 的 id（只用来数这个 subject 有几条、是不是那一条）。
    fn ids_of(&self, subject: &str) -> Vec<String> {
        self.owner
            .desired_list(None, 1000)
            .unwrap()
            .into_iter()
            .filter(|record| record.subject == subject)
            .map(|record| record.id)
            .collect()
    }
}

fn skill(name: &str, summary: &str) -> String {
    serde_json::json!({
        "name": name,
        "summary": summary,
        "triggers": ["写或改 React 代码"],
        "markdown": "# 正文\n",
    })
    .to_string()
}

/// claude 的能力集；`deny_by_default: false` 结构合法、能规范化，但语义校验必拒。
fn capability_set(deny_by_default: bool) -> serde_json::Value {
    serde_json::json!({
        "ai_type": "claude",
        "read": {"project_whitelist": ["项目/Studiolo"], "deny_by_default": deny_by_default},
        "write": {"board": false, "content_patch_only": true, "content_direct": false},
        "policy": {"propose_desired": true, "activate": false},
        "hard_bound_skills": [],
    })
}

#[test]
fn a_retry_of_the_newest_identical_proposal_returns_it_without_a_new_row() {
    let s = stores();
    let crlf = "# 提议\r\n\r\n断线前提交的正文。\r\n";
    let lf = crlf.replace("\r\n", "\n");
    let first = s.rule("claude", "01INSTANCEA", "提议.md", crlf);
    assert!(!first.deduplicated);
    assert_eq!(first.record.status, DesiredStatus::Pending);
    assert_eq!(
        first.body_sha256.as_deref(),
        Some(sha256_hex(lf.as_bytes()).as_str())
    );
    assert_eq!(
        first.body_sha256.as_deref(),
        Some(first.record.body_sha256.as_str())
    );

    // 断线重连后是另一个实例；只差换行也是同一份规范化正文。
    let retry = s.rule("claude", "01INSTANCEB", "提议.md", &lf);
    assert!(retry.deduplicated);
    assert_eq!(retry.record, first.record, "原样返回，不改任何字段");
    assert_eq!(retry.body_sha256, first.body_sha256);
    assert_eq!(s.ids_of("提议.md"), [first.record.id.as_str()]);
    assert_eq!(s.owner.effective().unwrap().pending_desired, 1);
    // 入库的仍是第一次提交的原文。
    assert_eq!(first.record.body.as_deref(), Some(crlf));
}

#[test]
fn skills_and_capability_sets_compare_by_canonical_json() {
    let s = stores();
    let compact = skill("tdd", "红绿重构");
    let mut value: serde_json::Value = serde_json::from_str(&compact).unwrap();
    let spaced = serde_json::to_string_pretty(&value).unwrap();
    let first = s.propose("claude", "01A", PolicyKind::Skills, "tdd", &compact);
    let retry = s.propose("claude", "01B", PolicyKind::Skills, "tdd", &spaced);
    assert!(retry.deduplicated);
    assert_eq!(retry.record.id, first.record.id);
    assert_eq!(
        first.body_sha256.as_deref(),
        Some(first.record.body_sha256.as_str())
    );
    // 内容真的变了才新建。
    value["summary"] = "另一份摘要".into();
    let changed = s.propose(
        "claude",
        "01B",
        PolicyKind::Skills,
        "tdd",
        &value.to_string(),
    );
    assert!(!changed.deduplicated);
    assert_ne!(changed.record.id, first.record.id);

    let set = capability_set(true);
    let first = s.propose(
        "claude",
        "01A",
        PolicyKind::CapabilitySet,
        "claude",
        &set.to_string(),
    );
    assert_eq!(
        first.record.status,
        DesiredStatus::Pending,
        "{:?}",
        first.record.report
    );
    let retry = s.propose(
        "claude",
        "01B",
        PolicyKind::CapabilitySet,
        "claude",
        &serde_json::to_string_pretty(&set).unwrap(),
    );
    assert!(retry.deduplicated);
    assert_eq!(retry.record.id, first.record.id);
}

#[test]
fn a_full_quota_does_not_block_a_retry_but_still_blocks_a_new_body() {
    let s = stores();
    let mut last = None;
    for index in 0..MAX_PENDING_PER_INSTANCE {
        last = Some(s.rule(
            "claude",
            "01FULL",
            &format!("q{index}.md"),
            &format!("# q{index}\n"),
        ));
    }
    let last = last.unwrap();
    let subject = last.record.subject.clone();
    let body = last.record.body.clone().unwrap();
    // 名额已满：断线后重提最后那份照样拿回原提案。
    let retry = s.rule("claude", "01FULL", &subject, &body);
    assert!(retry.deduplicated);
    assert_eq!(retry.record.id, last.record.id);
    // 新正文仍然挡在上限外。
    assert!(matches!(
        s.agent.propose(
            "claude",
            "01FULL",
            ProposalInput {
                kind: PolicyKind::Rules,
                subject: subject.clone(),
                body: "# 新的\n".into(),
            },
            NOW,
        ),
        Err(PolicyError::TooManyProposals)
    ));
}

#[test]
fn only_the_types_newest_open_draft_of_the_subject_absorbs_a_retry() {
    let s = stores();
    let first = s.rule("claude", "01A", "顺序.md", "# 第一版\n");
    let second = s.rule("claude", "01A", "顺序.md", "# 第二版\n");
    assert!(!second.deduplicated);
    // 本类型在第一版之后又提了第二版：再提第一版是要它重新成为本类型最新的提议，所以新建。
    let again = s.rule("claude", "01A", "顺序.md", "# 第一版\n");
    assert!(!again.deduplicated);
    assert_ne!(again.record.id, first.record.id);
    let retry = s.rule("claude", "01A", "顺序.md", "# 第一版\n");
    assert!(retry.deduplicated);
    assert_eq!(retry.record.id, again.record.id);

    // 更新的草案被拒（rejected）也算未发布的草案：旧的同正文提案接不住重提。
    let old = s.rule("claude", "01A", "被拒在后.md", "# 旧\n");
    let rejected = s.rule("claude", "01A", "被拒在后.md", "  \n");
    assert_eq!(rejected.record.status, DesiredStatus::Rejected);
    let resubmitted = s.rule("claude", "01A", "被拒在后.md", "# 旧\n");
    assert!(!resubmitted.deduplicated);
    assert_ne!(resubmitted.record.id, old.record.id);

    // 更新的草案被撤回后不再算数：旧的同正文提案重新是最新的，接得住重提。
    let kept = s.rule("claude", "01A", "撤回在后.md", "# 保留\n");
    let withdrawn = s.rule("claude", "01A", "撤回在后.md", "# 撤回\n");
    s.owner.withdraw_desired(&withdrawn.record.id, NOW).unwrap();
    let retry = s.rule("claude", "01A", "撤回在后.md", "# 保留\n");
    assert!(retry.deduplicated);
    assert_eq!(retry.record.id, kept.record.id);
}

#[test]
fn a_newer_draft_from_the_owner_or_another_type_does_not_change_what_a_retry_returns() {
    let s = stores();
    // 断线那几秒里主人提了一份更新的草案：重提照样拿回自己那份，不多出一条，也不排到主人那份后面。
    let mine = s.rule("claude", "01A", "主人在后.md", "# 接入方版\n");
    let owners = s.owner_rule("主人在后.md", "# 主人版\n");
    let retry = s.rule("claude", "01B", "主人在后.md", "# 接入方版\n");
    assert!(retry.deduplicated);
    assert_eq!(retry.record, mine.record);
    assert_eq!(s.ids_of("主人在后.md").len(), 2);
    // 主人那份原样不动，仍是该 subject 最新的草案。
    assert_eq!(s.owner.desired(&owners.id).unwrap(), Some(owners));

    // 别的类型提了更新的草案，同理。
    let mine = s.rule("claude", "01A", "别的类型在后.md", "# claude 版\n");
    s.rule("codex", "01X", "别的类型在后.md", "# codex 版\n");
    let retry = s.rule("claude", "01B", "别的类型在后.md", "# claude 版\n");
    assert!(retry.deduplicated);
    assert_eq!(retry.record, mine.record);
    assert_eq!(s.ids_of("别的类型在后.md").len(), 2);
}

#[test]
fn rejected_withdrawn_and_owner_desireds_never_absorb_a_retry() {
    let s = stores();
    // 过不了规范化：没有摘要，每次都新建一条 rejected，存原文。
    let first = s.propose("claude", "01A", PolicyKind::Skills, "tdd", "{}");
    let second = s.propose("claude", "01A", PolicyKind::Skills, "tdd", "{}");
    assert_eq!(first.record.status, DesiredStatus::Rejected);
    assert_eq!(first.body_sha256, None);
    assert!(!second.deduplicated);
    assert_ne!(second.record.id, first.record.id);
    assert_eq!(second.record.body.as_deref(), Some("{}"));

    // 主人撤回后，同一正文再提是新提案。
    let withdrawn = s.rule("claude", "01A", "撤回.md", "# 撤回\n");
    s.owner.withdraw_desired(&withdrawn.record.id, NOW).unwrap();
    let again = s.rule("claude", "01A", "撤回.md", "# 撤回\n");
    assert!(!again.deduplicated);
    assert_ne!(again.record.id, withdrawn.record.id);

    // 主人复核通过（validated）仍待发布，接得住重提，返回的状态就是 validated。
    let validated = s.rule("claude", "01A", "复核.md", "# 复核\n");
    s.owner.validate_desired(&validated.record.id, NOW).unwrap();
    let retry = s.rule("claude", "01B", "复核.md", "# 复核\n");
    assert!(retry.deduplicated);
    assert_eq!(retry.record.id, validated.record.id);
    assert_eq!(retry.record.status, DesiredStatus::Validated);

    // 主人自己提交的同一正文不是接入方提案：接入方照常新建自己的。
    let owned = s.owner_rule("主人.md", "# 主人\n");
    let proposed = s.rule("claude", "01A", "主人.md", "# 主人\n");
    assert!(!proposed.deduplicated);
    assert_ne!(proposed.record.id, owned.id);
}

#[test]
fn a_canonical_body_rejected_on_its_merits_has_a_digest_but_never_absorbs_a_retry() {
    let s = stores();
    // 能规范化、语义被拒：摘要是规范化正文的真摘要，不是空串摘要；状态是 rejected。
    let set = capability_set(false).to_string();
    let first = s.propose("claude", "01A", PolicyKind::CapabilitySet, "claude", &set);
    assert_eq!(first.record.status, DesiredStatus::Rejected);
    assert_eq!(
        first.body_sha256.as_deref(),
        Some(first.record.body_sha256.as_str())
    );
    assert_ne!(first.record.body_sha256, sha256_hex(b""));
    // 被拒的不拿回：重提要再校验一次，新建一条，摘要相同。
    let retry = s.propose("claude", "01B", PolicyKind::CapabilitySet, "claude", &set);
    assert!(!retry.deduplicated);
    assert_ne!(retry.record.id, first.record.id);
    assert_eq!(retry.body_sha256, first.body_sha256);
    assert!(
        s.agent
            .open_proposals("claude", None, None)
            .unwrap()
            .proposals
            .is_empty()
    );
}

#[test]
fn other_types_never_share_a_proposal() {
    let s = stores();
    let claude = s.rule("claude", "01A", "共用.md", "# 同一正文\n");
    let codex = s.rule("codex", "01X", "共用.md", "# 同一正文\n");
    assert!(!codex.deduplicated);
    assert_ne!(codex.record.id, claude.record.id);
    assert_eq!(codex.record.agent_type.as_deref(), Some("codex"));
    // 各自重提各自拿回自己那份。
    let retry = s.rule("codex", "01Y", "共用.md", "# 同一正文\n");
    assert!(retry.deduplicated);
    assert_eq!(retry.record.id, codex.record.id);
    let retry = s.rule("claude", "01B", "共用.md", "# 同一正文\n");
    assert!(retry.deduplicated);
    assert_eq!(retry.record.id, claude.record.id);
}

#[test]
fn the_owner_handle_routes_agent_submissions_through_the_same_rules() {
    let s = stores();
    let submit = |instance: &str, body: Option<&str>| {
        s.owner.submit_desired(
            DesiredInput {
                kind: PolicyKind::Rules,
                subject: "同一入口.md".into(),
                body: body.map(str::to_owned),
            },
            Submitter::Agent {
                agent_type: "claude".into(),
                instance_id: instance.into(),
            },
            NOW,
        )
    };
    let first = submit("01A", Some("# 一份\n")).unwrap();
    assert_eq!(first.status, DesiredStatus::Pending);
    let retry = submit("01B", Some("# 一份\n")).unwrap();
    assert_eq!(retry.id, first.id);
    assert_eq!(s.ids_of("同一入口.md"), [first.id.as_str()]);
    // 经接入方句柄重提，命中的是同一份。
    let via_agent = s.rule("claude", "01C", "同一入口.md", "# 一份\n");
    assert!(via_agent.deduplicated);
    assert_eq!(via_agent.record.id, first.id);
    assert!(matches!(
        submit("01A", None),
        Err(PolicyError::InvalidBody(_))
    ));
}

#[test]
fn concurrent_retries_of_the_same_body_leave_a_single_proposal() {
    const WRITERS: usize = 4;
    let s = stores();
    for round in 0..8 {
        let subject = format!("并发{round}.md");
        let start = Barrier::new(WRITERS);
        let results: Vec<Proposal> = thread::scope(|scope| {
            let workers: Vec<_> = (0..WRITERS)
                .map(|writer| {
                    let (dir, start, subject) = (&s.dir, &start, &subject);
                    scope.spawn(move || {
                        // 每条线程一个句柄，即各自一条数据库连接：模拟断线前后的几个会话同时到达。
                        let agent = PolicyAgentStore::open_existing(dir).unwrap();
                        start.wait();
                        agent
                            .propose(
                                "claude",
                                &format!("01R{round}W{writer}"),
                                ProposalInput {
                                    kind: PolicyKind::Rules,
                                    subject: subject.clone(),
                                    body: "# 同时到达\n".into(),
                                },
                                NOW,
                            )
                            .unwrap()
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect()
        });
        // 认重提与插入在同一个 BEGIN IMMEDIATE 写事务里：只有一个新建，其余都拿回它。
        let created: Vec<_> = results
            .iter()
            .filter(|result| !result.deduplicated)
            .collect();
        assert_eq!(created.len(), 1, "第 {round} 轮");
        assert!(
            results
                .iter()
                .all(|result| result.record == created[0].record)
        );
        assert_eq!(s.ids_of(&subject), [created[0].record.id.as_str()]);
    }
}

#[test]
fn open_proposals_lists_the_types_open_agent_proposals_newest_first() {
    let s = stores();
    let a = s.rule("claude", "01A", "a.md", "# A\n");
    let b = s.rule("claude", "01B", "b.md", "# B\n");
    let c = s.rule("claude", "01A", "c.md", "# C\n");
    let d = s.rule("claude", "01A", "d.md", "# D\n");
    let rejected = s.propose("claude", "01A", PolicyKind::Skills, "tdd", "{}");
    assert_eq!(rejected.record.status, DesiredStatus::Rejected);
    s.owner.withdraw_desired(&c.record.id, NOW).unwrap();
    s.owner.validate_desired(&d.record.id, NOW).unwrap();
    s.owner_rule("e.md", "# E\n");
    let x = s.rule("codex", "01X", "a.md", "# A\n");

    let listing = s.agent.open_proposals("claude", None, None).unwrap();
    assert!(!listing.more);
    let ids = listing
        .proposals
        .iter()
        .map(|proposal| proposal.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            d.record.id.clone(),
            b.record.id.clone(),
            a.record.id.clone()
        ]
    );
    let first = &listing.proposals[2];
    assert_eq!(first.kind, PolicyKind::Rules);
    assert_eq!(first.subject, "a.md");
    assert_eq!(first.status, DesiredStatus::Pending);
    assert_eq!(first.body_sha256, sha256_hex(b"# A\n"));
    assert_eq!(first.agent_instance.as_deref(), Some("01A"));
    assert_eq!(first.submitted_at, NOW);
    assert_eq!(listing.proposals[0].status, DesiredStatus::Validated);

    let narrowed = s
        .agent
        .open_proposals("claude", Some(PolicyKind::Rules), Some("b.md"))
        .unwrap();
    assert_eq!(narrowed.proposals.len(), 1);
    assert_eq!(narrowed.proposals[0].id, b.record.id);
    assert!(
        s.agent
            .open_proposals("claude", Some(PolicyKind::Skills), None)
            .unwrap()
            .proposals
            .is_empty()
    );
    let codex = s.agent.open_proposals("codex", None, None).unwrap();
    assert_eq!(codex.proposals.len(), 1);
    assert_eq!(codex.proposals[0].id, x.record.id);
    assert!(
        s.agent
            .open_proposals("grok", None, None)
            .unwrap()
            .proposals
            .is_empty()
    );
}

#[test]
fn open_proposals_stops_at_the_listing_bound_and_reports_more() {
    let s = stores();
    for index in 0..MAX_LISTED_PROPOSALS {
        s.rule(
            "claude",
            &format!("01I{:02}", index / MAX_PENDING_PER_INSTANCE),
            &format!("r{index:02}.md"),
            &format!("# r{index:02}\n"),
        );
    }
    let exact = s.agent.open_proposals("claude", None, None).unwrap();
    assert_eq!(exact.proposals.len(), MAX_LISTED_PROPOSALS);
    assert!(!exact.more);

    let index = MAX_LISTED_PROPOSALS;
    s.rule(
        "claude",
        &format!("01I{:02}", index / MAX_PENDING_PER_INSTANCE),
        &format!("r{index:02}.md"),
        &format!("# r{index:02}\n"),
    );
    let bounded = s.agent.open_proposals("claude", None, None).unwrap();
    assert_eq!(bounded.proposals.len(), MAX_LISTED_PROPOSALS);
    assert!(bounded.more);
    assert_eq!(bounded.proposals[0].subject, format!("r{index:02}.md"));
    assert!(
        bounded
            .proposals
            .iter()
            .all(|proposal| proposal.subject != "r00.md")
    );
}

#[test]
fn publishing_takes_proposals_off_the_list_and_a_later_retry_starts_a_new_one() {
    let s = stores();
    let older = s.rule("claude", "01A", "发布.md", "# 一\n");
    let newest = s.rule("claude", "01A", "发布.md", "# 二\n");
    let hash = s.owner.effective().unwrap().effective_hash;
    s.owner
        .activate(&newest.record.id, &hash, "owner", NOW)
        .unwrap();
    let status = |id: &str| s.owner.desired(id).unwrap().unwrap().status;
    assert_eq!(status(&newest.record.id), DesiredStatus::Activated);
    assert_eq!(status(&older.record.id), DesiredStatus::Superseded);
    // 已发布、被取代的都不再待处理，列表里没有。
    assert!(
        s.agent
            .open_proposals("claude", None, None)
            .unwrap()
            .proposals
            .is_empty()
    );
    // 发布后再提同一份正文：它已不在待处理里，新建一条与现行相同的提案（已知行为，提交方可先 policy_read 比对）。
    let again = s.rule("claude", "01B", "发布.md", "# 二\n");
    assert!(!again.deduplicated);
    assert_ne!(again.record.id, newest.record.id);
    let listed = s.agent.open_proposals("claude", None, None).unwrap();
    assert_eq!(listed.proposals.len(), 1);
    assert_eq!(listed.proposals[0].id, again.record.id);
}

#[test]
fn an_owner_revalidation_syncs_the_digest_the_listing_reports() {
    let s = stores();
    // 接入方正文上限低于主人的：超过接入方上限的提案入库时过不了规范化，摘要列是空串摘要。
    let line = "- 一行规则正文。\n";
    let body = line.repeat(MAX_PROPOSAL_BYTES / line.len() + 1);
    assert!(body.len() > MAX_PROPOSAL_BYTES && body.len() <= MAX_BODY_BYTES);
    let oversized = s.rule("claude", "01A", "超长.md", &body);
    assert_eq!(oversized.record.status, DesiredStatus::Rejected);
    assert_eq!(oversized.body_sha256, None);
    assert_eq!(oversized.record.body_sha256, sha256_hex(b""));
    // 主人按自己的上限复核通过：状态与摘要列一起更新，列表给出的摘要与正文对得上。
    let validated = s.owner.validate_desired(&oversized.record.id, NOW).unwrap();
    assert_eq!(validated.status, DesiredStatus::Validated);
    assert_eq!(validated.body_sha256, sha256_hex(body.as_bytes()));
    let listed = s
        .agent
        .open_proposals("claude", Some(PolicyKind::Rules), Some("超长.md"))
        .unwrap();
    assert_eq!(listed.proposals.len(), 1);
    assert_eq!(listed.proposals[0].body_sha256, validated.body_sha256);
}
