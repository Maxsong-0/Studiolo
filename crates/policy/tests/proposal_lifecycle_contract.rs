//! 提案名额、处理结果与授权快照：校验失败不释放名额，结果不回显正文或他人的草案。

use std::{fs, os::unix::fs::PermissionsExt};
use studiolo_policy::{
    DesiredInput, DesiredStatus, MAX_PENDING_PER_INSTANCE, MAX_PENDING_PER_TYPE,
    MAX_PENDING_PROPOSALS, PolicyAgentStore, PolicyError, PolicyKind, PolicyStore, ProposalInput,
    Submitter,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-13T10:00:00Z";

fn stores() -> (TempDir, PolicyStore, PolicyAgentStore) {
    let temp = TempDir::new_in(fs::canonicalize("/tmp").unwrap()).unwrap();
    let dir = temp.path().join("policy");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let owner = PolicyStore::open(&dir).unwrap();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    (temp, owner, agent)
}

fn input(subject: &str, body: &str) -> ProposalInput {
    ProposalInput {
        kind: PolicyKind::Rules,
        subject: subject.into(),
        body: body.into(),
    }
}

#[test]
fn every_open_status_counts_and_withdrawal_releases_one_slot() {
    for status in [
        DesiredStatus::Pending,
        DesiredStatus::Validated,
        DesiredStatus::Rejected,
    ] {
        let (_temp, owner, agent) = stores();
        let mut ids = Vec::new();
        for index in 0..MAX_PENDING_PER_INSTANCE {
            let body = if status == DesiredStatus::Rejected {
                " "
            } else {
                "# 私有正文\n"
            };
            let record = agent
                .propose(
                    "claude",
                    "instance",
                    input(&format!("r{index}.md"), body),
                    NOW,
                )
                .unwrap()
                .record;
            if status == DesiredStatus::Validated {
                assert_eq!(
                    owner.validate_desired(&record.id, NOW).unwrap().status,
                    status
                );
            } else {
                assert_eq!(record.status, status);
            }
            ids.push(record.id);
        }
        assert_eq!(
            agent.propose("claude", "instance", input("next.md", "# 下一份\n"), NOW),
            Err(PolicyError::TooManyProposals)
        );
        // 满额也允许同正文重提已校验／待处理提案；校验失败不去重，仍会被名额挡住。
        if status != DesiredStatus::Rejected {
            let retry = agent
                .propose("claude", "instance", input("r0.md", "# 私有正文\n"), NOW)
                .unwrap();
            assert!(retry.deduplicated);
            assert_eq!(retry.record.id, ids[0]);
        }
        owner.withdraw_desired(&ids[0], NOW).unwrap();
        agent
            .propose("claude", "instance", input("next.md", "# 下一份\n"), NOW)
            .unwrap();
        assert_eq!(
            agent.propose("claude", "instance", input("extra.md", "# 再一份\n"), NOW),
            Err(PolicyError::TooManyProposals)
        );
    }
}

#[test]
fn rejected_proposals_also_fill_the_global_quota_but_owner_drafts_do_not() {
    let (_temp, owner, agent) = stores();
    owner
        .submit_desired(
            DesiredInput {
                kind: PolicyKind::Rules,
                subject: "owner.md".into(),
                body: Some("# 主人\n".into()),
            },
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    let mut first = None;
    for index in 0..MAX_PENDING_PROPOSALS {
        // 每类型名额小于全库，分给几类 AI 才填得满全库。
        let result = agent
            .propose(
                &format!("type{}", index / MAX_PENDING_PER_TYPE),
                &format!("instance{}", index / MAX_PENDING_PER_INSTANCE),
                input(&format!("r{index}.md"), " "),
                NOW,
            )
            .unwrap();
        assert_eq!(result.record.status, DesiredStatus::Rejected);
        first.get_or_insert(result.record.id);
    }
    // 全库满了：哪一类都交不进，包括一份都没交过的类型。
    assert_eq!(
        agent.propose("codex", "fresh", input("new.md", "# 新\n"), NOW),
        Err(PolicyError::TooManyProposals)
    );
    owner
        .withdraw_desired(first.as_ref().unwrap(), NOW)
        .unwrap();
    agent
        .propose("codex", "fresh", input("new.md", "# 新\n"), NOW)
        .unwrap();
}

#[test]
fn one_types_unfinished_proposals_fill_only_its_own_type_quota() {
    // T-248：每次连接都是新实例，换实例就绕得过每实例名额；校验失败的又占名额，一类 AI 出了毛病
    // 换着实例连交废提案，没有每类型名额就能占满全库、挡住别的类型。
    let (_temp, owner, agent) = stores();
    let mut first = None;
    for index in 0..MAX_PENDING_PER_TYPE {
        let result = agent
            .propose(
                "codex",
                &format!("instance{}", index / MAX_PENDING_PER_INSTANCE),
                input(&format!("r{index}.md"), " "),
                NOW,
            )
            .unwrap();
        assert_eq!(result.record.status, DesiredStatus::Rejected);
        first.get_or_insert(result.record.id);
    }
    // 本类型换一个新实例也交不进，什么都没写。
    let before = owner.desired_list(None, 1_000).unwrap().len();
    assert_eq!(
        agent.propose("codex", "fresh", input("new.md", "# 新\n"), NOW),
        Err(PolicyError::TooManyProposals)
    );
    assert_eq!(owner.desired_list(None, 1_000).unwrap().len(), before);
    // 别的类型照常能交。
    agent
        .propose("claude", "fresh", input("new.md", "# 新\n"), NOW)
        .unwrap();
    // 主人撤回本类型的一份，就放回一个名额，只放回一个。
    owner
        .withdraw_desired(first.as_ref().unwrap(), NOW)
        .unwrap();
    agent
        .propose("codex", "fresh", input("new.md", "# 新\n"), NOW)
        .unwrap();
    assert_eq!(
        agent.propose("codex", "another", input("more.md", "# 再一份\n"), NOW),
        Err(PolicyError::TooManyProposals)
    );
}

#[test]
fn results_follow_publication_rejection_withdrawal_and_supersession_without_body() {
    let (_temp, owner, agent) = stores();
    let allowed = vec!["a.md".into(), "b.md".into(), "c.md".into()];
    let older = agent
        .propose("claude", "one", input("a.md", "# 正文不回流\n"), NOW)
        .unwrap()
        .record;
    let published = agent
        .propose("claude", "one", input("a.md", "# 新正文不回流\n"), NOW)
        .unwrap()
        .record;
    let rejected = agent
        .propose("claude", "one", input("b.md", " "), NOW)
        .unwrap()
        .record;
    let withdrawn = agent
        .propose("claude", "one", input("c.md", "# 退回正文\n"), NOW)
        .unwrap()
        .record;
    owner.withdraw_desired(&withdrawn.id, NOW).unwrap();
    let activation = owner
        .activate(
            &published.id,
            &owner.effective().unwrap().effective_hash,
            "owner",
            NOW,
        )
        .unwrap();
    let hash = &activation.effective_hash;
    for (record, status, reason) in [
        (
            &older,
            DesiredStatus::Superseded,
            "superseded_by_publication",
        ),
        (&published, DesiredStatus::Activated, "published"),
        (&rejected, DesiredStatus::Rejected, "validation_failed"),
        (
            &withdrawn,
            DesiredStatus::Withdrawn,
            "owner_withdrawn_no_reason_recorded",
        ),
    ] {
        let result = agent
            .proposal_result("claude", &record.id, &allowed, hash)
            .unwrap()
            .unwrap();
        assert_eq!(result.proposal.status, status);
        assert_eq!(result.reason, reason);
        assert_eq!(
            result.activated_version,
            (status == DesiredStatus::Activated).then_some(activation.version)
        );
        assert_eq!(
            !result.problems.is_empty(),
            status == DesiredStatus::Rejected
        );
        let json = serde_json::to_value(&result).unwrap();
        for absent in [
            "body",
            "diff",
            "canonical",
            "expansions",
            "report",
            "superseded_desired_ids",
        ] {
            assert!(json.get(absent).is_none(), "{json}");
        }
        assert!(!json.to_string().contains("正文不回流"));
        assert!(
            agent
                .proposal_result("codex", &record.id, &allowed, hash)
                .unwrap()
                .is_none()
        );
        assert!(
            agent
                .proposal_result("claude", &record.id, &[], hash)
                .unwrap()
                .is_none()
        );
    }
    let mine = owner
        .submit_desired(
            DesiredInput {
                kind: PolicyKind::Rules,
                subject: "a.md".into(),
                body: Some("# 主人私有草案\n".into()),
            },
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    for id in [&mine.id, "not-found"] {
        assert!(
            agent
                .proposal_result("claude", id, &allowed, hash)
                .unwrap()
                .is_none()
        );
    }
    assert!(
        agent
            .open_proposals("claude", None, None)
            .unwrap()
            .proposals
            .is_empty()
    );
}

#[test]
fn a_changed_authorization_snapshot_refuses_submission_and_result_reading() {
    let (_temp, owner, agent) = stores();
    let before = owner.effective().unwrap().effective_hash;
    let existing = agent
        .propose("claude", "one", input("a.md", "# 草案\n"), NOW)
        .unwrap()
        .record;
    owner
        .bootstrap(
            DesiredInput {
                kind: PolicyKind::Rules,
                subject: "b.md".into(),
                body: Some("# 新策略\n".into()),
            },
            NOW,
        )
        .unwrap();
    assert!(matches!(
        agent.propose_at_effective("claude", "one", input("a.md", "# 新提案\n"), NOW, &before),
        Err(PolicyError::HashMismatch { .. })
    ));
    assert!(matches!(
        agent.proposal_result("claude", &existing.id, &["a.md".into()], &before),
        Err(PolicyError::HashMismatch { .. })
    ));
    assert_eq!(owner.desired_list(None, 100).unwrap().len(), 1);
}

#[test]
fn draft_listing_uses_insertion_order_even_for_equal_or_reversed_timestamps() {
    let (_temp, owner, agent) = stores();
    let mut ids = Vec::new();
    for now in [
        "2026-11-13T10:00:01Z",
        "2026-11-13T10:00:00.09Z",
        "2026-11-13T10:00:00.09Z",
    ] {
        let record = agent
            .propose(
                "claude",
                "one",
                input("a.md", &format!("# {}\n", ids.len())),
                now,
            )
            .unwrap()
            .record;
        ids.push(record.id);
    }
    ids.reverse();
    assert_eq!(
        owner
            .desired_list(None, 100)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(
        owner
            .desired_queue(1)
            .unwrap()
            .0
            .into_iter()
            .map(|r| r.id)
            .collect::<Vec<_>>(),
        ids
    );
    // 接入方 `list_proposals` 的「新到旧」也是入库先后（T-248 补）：时间回拨、同一时刻都不乱。
    assert_eq!(
        agent
            .open_proposals("claude", None, None)
            .unwrap()
            .proposals
            .into_iter()
            .map(|proposal| proposal.id)
            .collect::<Vec<_>>(),
        ids
    );
}
