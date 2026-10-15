//! 策略平面契约：desired → 校验 → diff／扩大项 → 发布（CAS）→ effective；回滚、bootstrap、
//! 三级吊销与恢复方向；接入方提案永不生效；重开与损坏库。

use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

use studiolo_policy::{
    Denial, DesiredInput, DesiredStatus, MAX_PENDING_PER_INSTANCE, PolicyAgentStore, PolicyError,
    PolicyKind, PolicyStore, ProposalInput, RevocationLevel, Submitter,
};
use tempfile::TempDir;

fn private_dir() -> (TempDir, PathBuf) {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let dir = temp.path().join("policy-data");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    (temp, dir)
}

const NOW: &str = "2026-09-17T22:00:00Z";

fn capability(whitelist: &[&str], bindings: &[(&str, &str)], propose: bool) -> String {
    serde_json::json!({
        "ai_type": "claude",
        "read": {"project_whitelist": whitelist, "deny_by_default": true},
        "write": {"board": false, "content_patch_only": true, "content_direct": false},
        "policy": {"propose_desired": propose, "activate": false},
        "hard_bound_skills": bindings.iter().map(|(when, skill)| serde_json::json!({"when": when, "must_use": skill})).collect::<Vec<_>>(),
    })
    .to_string()
}

fn skill(name: &str, markdown: &str) -> String {
    serde_json::json!({
        "name": name,
        "summary": "React 性能规范",
        "triggers": ["写或改 React / Next.js 代码"],
        "markdown": markdown,
    })
    .to_string()
}

fn desired(kind: PolicyKind, subject: &str, body: Option<&str>) -> DesiredInput {
    DesiredInput {
        kind,
        subject: subject.into(),
        body: body.map(str::to_owned),
    }
}

#[test]
fn board_writes_require_an_explicit_project_subset_and_are_reported_as_expansion() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let mut body: serde_json::Value =
        serde_json::from_str(&capability(&["项目/Studiolo", "项目/Other"], &[], true)).unwrap();
    body["write"]["board"] = true.into();
    body["write"]["board_projects"] = serde_json::json!(["项目/Studiolo"]);
    let record = store
        .submit_desired(
            desired(PolicyKind::CapabilitySet, "claude", Some(&body.to_string())),
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    assert_eq!(record.status, DesiredStatus::Validated);
    assert!(
        record.report.as_ref().unwrap().expansions.iter().any(
            |e| e.path == "write.board_projects" && e.after.as_deref() == Some("项目/Studiolo")
        )
    );
    store
        .activate(
            &record.id,
            &store.effective().unwrap().effective_hash,
            "owner",
            NOW,
        )
        .unwrap();
    let (set, _) = store.capability_set("claude").unwrap().unwrap();
    assert_eq!(set.write.board_projects, vec!["项目/Studiolo"]);
    for projects in [
        serde_json::json!([]),
        serde_json::json!(["项目"]),
        serde_json::json!(["项目/Outside"]),
        serde_json::json!(["项目/Studiolo", "项目/Studiolo"]),
    ] {
        body["write"]["board_projects"] = projects;
        let record = store
            .submit_desired(
                desired(PolicyKind::CapabilitySet, "claude", Some(&body.to_string())),
                Submitter::Owner,
                NOW,
            )
            .unwrap();
        assert_ne!(record.status, DesiredStatus::Validated);
    }
}

#[test]
fn board_only_capability_publishes_without_content_expansion_and_still_rejects_direct() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let mut body: serde_json::Value =
        serde_json::from_str(&capability(&["项目/Studiolo"], &[], false)).unwrap();
    body["ai_type"] = "codex".into();
    body["write"]["board"] = true.into();
    body["write"]["board_projects"] = serde_json::json!(["项目/Studiolo"]);
    body["write"]["content_patch_only"] = false.into();
    let before = store.effective().unwrap().effective_hash;
    let record = store
        .submit_desired(
            desired(PolicyKind::CapabilitySet, "codex", Some(&body.to_string())),
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    assert_eq!(
        record.status,
        DesiredStatus::Validated,
        "{:?}",
        record.report
    );
    let report = record.report.as_ref().unwrap();
    assert!(report.ok && report.problems.is_empty());
    assert_eq!(
        report
            .expansions
            .iter()
            .map(|item| item.path.as_str())
            .collect::<Vec<_>>(),
        [
            "read.project_whitelist",
            "write.board_projects",
            "write.board"
        ]
    );
    assert!(store.capability_set("codex").unwrap().is_none());
    let activation = store.activate(&record.id, &before, "owner", NOW).unwrap();
    assert_eq!(activation.expansions, report.expansions);
    let (set, _) = store.capability_set("codex").unwrap().unwrap();
    assert!(set.write.board);
    assert_eq!(set.write.board_projects, ["项目/Studiolo"]);
    assert!(!set.write.content_patch_only && !set.write.content_direct);

    body["write"]["content_direct"] = true.into();
    let invalid = store
        .submit_desired(
            desired(PolicyKind::CapabilitySet, "codex", Some(&body.to_string())),
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    assert_eq!(invalid.status, DesiredStatus::Rejected);
    assert!(
        invalid
            .report
            .unwrap()
            .problems
            .iter()
            .any(|problem| problem.path == "write.content_direct")
    );
    assert!(matches!(
        store.activate(&invalid.id, &activation.effective_hash, "owner", NOW),
        Err(PolicyError::ValidationFailed)
    ));
    assert_eq!(
        store.effective().unwrap().effective_hash,
        activation.effective_hash
    );
}

#[test]
fn capability_set_goes_through_validate_diff_and_cas_activation() {
    let (_temp, dir) = private_dir();
    assert!(matches!(
        PolicyStore::open_existing(&dir),
        Err(PolicyError::MissingStore)
    ));
    let store = PolicyStore::open(&dir).unwrap();
    let initial = store.effective().unwrap();
    assert_eq!(initial.effective_hash.len(), 64);
    assert!(initial.entries.is_empty());
    assert!(!initial.global_revoked);

    let submitted = store
        .submit_desired(
            desired(
                PolicyKind::CapabilitySet,
                "claude",
                Some(&capability(&["项目/Studiolo", "_规则"], &[], true)),
            ),
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    assert_eq!(submitted.status, DesiredStatus::Validated);
    let report = submitted.report.clone().unwrap();
    assert!(report.ok);
    assert_eq!(report.base_effective_hash, initial.effective_hash);
    // 首次授予读取范围与提案能力都是扩大项，界面必须标红。
    let paths = report
        .expansions
        .iter()
        .map(|item| item.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "read.project_whitelist",
            "read.project_whitelist",
            "write.content_patch_only",
            "policy.propose_desired"
        ]
    );
    assert_eq!(report.diff.format, "json");
    assert!(report.diff.added > 0 && report.diff.removed == 0);

    // effective 未变：提交不产生授权。
    assert_eq!(
        store.effective().unwrap().effective_hash,
        initial.effective_hash
    );
    assert!(store.capability_set("claude").unwrap().is_none());

    // CAS：哈希不匹配即拒绝，版本不变。
    let mismatch = store.activate(&submitted.id, &"0".repeat(64), "owner", NOW);
    assert!(matches!(mismatch, Err(PolicyError::HashMismatch { .. })));
    assert!(store.capability_set("claude").unwrap().is_none());
    assert!(matches!(
        store.activate("01NOPE", &initial.effective_hash, "owner", NOW),
        Err(PolicyError::DesiredNotFound)
    ));

    let activation = store
        .activate(&submitted.id, &initial.effective_hash, "owner", NOW)
        .unwrap();
    assert_eq!(activation.version, 1);
    assert_eq!(activation.kind, PolicyKind::CapabilitySet);
    assert_eq!(activation.previous_effective_hash, initial.effective_hash);
    assert_ne!(activation.effective_hash, initial.effective_hash);
    assert_eq!(activation.expansions.len(), 4);
    assert!(!activation.cleared_type_revocation);

    let (set, meta) = store.capability_set("claude").unwrap().unwrap();
    assert_eq!(set.read.project_whitelist, ["项目/Studiolo", "_规则"]);
    assert_eq!(meta.version, 1);
    assert_eq!(meta.effective_hash, activation.effective_hash);
    let summary = store.effective().unwrap();
    assert_eq!(summary.effective_hash, activation.effective_hash);
    assert_eq!(summary.entries.len(), 1);
    assert_eq!(summary.pending_desired, 0);
    let record = store.desired(&submitted.id).unwrap().unwrap();
    assert_eq!(record.status, DesiredStatus::Activated);
    assert_eq!(record.activated_version, Some(1));
    // 已发布的 desired 不能再次发布。
    assert!(matches!(
        store.activate(&submitted.id, &summary.effective_hash, "owner", NOW),
        Err(PolicyError::DesiredNotActivatable(_))
    ));

    // 收窄：去掉一个前缀、关掉提案 → 无扩大项、两个收窄项。
    let narrowed = store
        .submit_desired(
            desired(
                PolicyKind::CapabilitySet,
                "claude",
                Some(&capability(&["项目/Studiolo"], &[], false)),
            ),
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    let report = narrowed.report.unwrap();
    assert!(report.ok);
    assert!(report.expansions.is_empty());
    assert_eq!(report.narrowings.len(), 2);
    assert_eq!(report.current_version, Some(1));

    // 重开库：effective 与版本都还在。
    drop(store);
    let reopened = PolicyStore::open_existing(&dir).unwrap();
    assert_eq!(
        reopened.effective().unwrap().effective_hash,
        activation.effective_hash
    );
    assert_eq!(reopened.versions(None, None, 10).unwrap().len(), 1);
}

#[test]
fn constant_constraints_cannot_be_relaxed_by_configuration() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let cases: Vec<(serde_json::Value, &str)> = vec![
        (
            serde_json::json!({"ai_type":"claude","read":{"project_whitelist":["项目/Studiolo"],"deny_by_default":false},"write":{"board":false,"content_patch_only":true,"content_direct":false},"policy":{"propose_desired":false,"activate":false}}),
            "read.deny_by_default",
        ),
        (
            serde_json::json!({"ai_type":"claude","read":{"project_whitelist":[],"deny_by_default":true},"write":{"board":false,"content_patch_only":true,"content_direct":true},"policy":{"propose_desired":false,"activate":false}}),
            "write.content_direct",
        ),
        (
            serde_json::json!({"ai_type":"claude","read":{"project_whitelist":[],"deny_by_default":true},"write":{"board":true,"content_patch_only":true,"content_direct":false},"policy":{"propose_desired":false,"activate":false}}),
            "write.board",
        ),
        (
            serde_json::json!({"ai_type":"claude","read":{"project_whitelist":[],"deny_by_default":true},"write":{"board":false,"content_patch_only":true,"content_direct":false},"policy":{"propose_desired":false,"activate":true}}),
            "policy.activate",
        ),
        (
            serde_json::json!({"ai_type":"claude","read":{"project_whitelist":["../School"],"deny_by_default":true},"write":{"board":false,"content_patch_only":true,"content_direct":false},"policy":{"propose_desired":false,"activate":false}}),
            "read.project_whitelist[0]",
        ),
        (
            serde_json::json!({"ai_type":"claude","read":{"project_whitelist":[".git"],"deny_by_default":true},"write":{"board":false,"content_patch_only":true,"content_direct":false},"policy":{"propose_desired":false,"activate":false}}),
            "read.project_whitelist[0]",
        ),
        (
            serde_json::json!({"ai_type":"claude","read":{"project_whitelist":[],"deny_by_default":true},"write":{"board":false,"content_patch_only":true,"content_direct":false},"policy":{"propose_desired":false,"activate":false},"hard_bound_skills":[{"when":"写 React","must_use":"nope"}]}),
            "hard_bound_skills[0].must_use",
        ),
        (
            serde_json::json!({"ai_type":"codex","read":{"project_whitelist":[],"deny_by_default":true},"write":{"board":false,"content_patch_only":true,"content_direct":false},"policy":{"propose_desired":false,"activate":false}}),
            "ai_type",
        ),
    ];
    for (body, path) in cases {
        let record = store
            .submit_desired(
                desired(PolicyKind::CapabilitySet, "claude", Some(&body.to_string())),
                Submitter::Owner,
                NOW,
            )
            .unwrap();
        assert_eq!(record.status, DesiredStatus::Rejected, "{path}");
        let report = record.report.unwrap();
        assert!(!report.ok);
        assert!(
            report.problems.iter().any(|problem| problem.path == path),
            "{path}: {:?}",
            report.problems
        );
        let hash = store.effective().unwrap().effective_hash;
        assert!(matches!(
            store.activate(&record.id, &hash, "owner", NOW),
            Err(PolicyError::ValidationFailed)
        ));
    }
    // 未知字段与非对象 JSON 一律拒绝，且不回显正文。
    for body in [
        r#"{"ai_type":"claude","read":{"project_whitelist":[],"deny_by_default":true},"write":{"board":false,"content_patch_only":true,"content_direct":false},"policy":{"propose_desired":false,"activate":false},"token":"SECRET_CANARY"}"#,
        "[]",
        "not json",
    ] {
        let record = store
            .submit_desired(
                desired(PolicyKind::CapabilitySet, "claude", Some(body)),
                Submitter::Owner,
                NOW,
            )
            .unwrap();
        assert_eq!(record.status, DesiredStatus::Rejected);
        let rendered = format!("{:?}", record.report.unwrap().problems);
        assert!(!rendered.contains("SECRET_CANARY"));
        // 原文原样保留：重新校验得到同样的拒绝，发布只能是 ValidationFailed，绝不退化成撤销。
        assert_eq!(record.body.as_deref(), Some(body));
        let hash = store.effective().unwrap().effective_hash;
        assert!(matches!(
            store.activate(&record.id, &hash, "owner", NOW),
            Err(PolicyError::ValidationFailed)
        ));
    }
    assert!(store.effective().unwrap().entries.is_empty());
}

#[test]
fn hard_bound_skills_cannot_be_retired_and_unbinding_is_an_expansion() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let publish = |input: DesiredInput| {
        let record = store.submit_desired(input, Submitter::Owner, NOW).unwrap();
        let hash = store.effective().unwrap().effective_hash;
        store.activate(&record.id, &hash, "owner", NOW).unwrap()
    };
    publish(desired(
        PolicyKind::Skills,
        "vercel-react-best-practices",
        Some(&skill("vercel-react-best-practices", "# 规范\n")),
    ));
    let bound = publish(desired(
        PolicyKind::CapabilitySet,
        "claude",
        Some(&capability(
            &["项目/Studiolo"],
            &[("写或改 React / Next.js 代码", "vercel-react-best-practices")],
            false,
        )),
    ));
    assert_eq!(bound.version, 2);
    // 绑定中的 skill 不能撤销。
    let retire = store
        .submit_desired(
            desired(PolicyKind::Skills, "vercel-react-best-practices", None),
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    assert_eq!(retire.status, DesiredStatus::Rejected);
    assert!(
        retire.report.unwrap().problems[0]
            .message
            .contains("硬绑定")
    );
    // 被硬绑定的 skill 正文变更是需要人看的扩大项。
    let changed = store
        .submit_desired(
            desired(
                PolicyKind::Skills,
                "vercel-react-best-practices",
                Some(&skill("vercel-react-best-practices", "# 放宽了\n")),
            ),
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    assert!(
        changed.report.unwrap().expansions[0]
            .note
            .contains("硬绑定")
    );
    // 解除绑定 = 约束放宽 = 扩大项。
    let unbound = store
        .submit_desired(
            desired(
                PolicyKind::CapabilitySet,
                "claude",
                Some(&capability(&["项目/Studiolo"], &[], false)),
            ),
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    let report = unbound.report.clone().unwrap();
    assert_eq!(report.expansions.len(), 1);
    assert_eq!(report.expansions[0].path, "hard_bound_skills");
    let hash = store.effective().unwrap().effective_hash;
    store.activate(&unbound.id, &hash, "owner", NOW).unwrap();
    // 现在可以撤销 skill；撤销后 effective 变化、版本标 retired。
    let retire = store
        .submit_desired(
            desired(PolicyKind::Skills, "vercel-react-best-practices", None),
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    assert_eq!(retire.status, DesiredStatus::Validated);
    let before = store.effective().unwrap().effective_hash;
    let retired = store.activate(&retire.id, &before, "owner", NOW).unwrap();
    assert!(retired.retired);
    assert_ne!(retired.effective_hash, before);
    assert!(store.skills().unwrap().is_empty());
    let meta = store.versions(Some(PolicyKind::Skills), None, 10).unwrap();
    assert!(meta[0].retired && !meta[1].retired);
    assert!(
        store
            .version(meta[0].version)
            .unwrap()
            .unwrap()
            .body
            .is_none()
    );
    // 撤销一个不存在的 subject 不可发布。
    let nothing = store
        .submit_desired(
            desired(PolicyKind::Rules, "缺.md", None),
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    assert_eq!(nothing.status, DesiredStatus::Rejected);
}

#[test]
fn rules_diff_flags_removed_constraints_and_rollback_creates_a_new_version() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let v1 = store
        .submit_desired(
            desired(
                PolicyKind::Rules,
                "通用工作规则.md",
                Some("# 规则\r\n\r\n## 准则① 永远说中文\r\n\r\n禁止绕过式修复。\r\n"),
            ),
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    let report = v1.report.clone().unwrap();
    assert!(report.ok);
    assert!(report.expansions.is_empty());
    assert_eq!(report.diff.format, "text");
    let hash = store.effective().unwrap().effective_hash;
    let first = store.activate(&v1.id, &hash, "owner", NOW).unwrap();
    let stored = store.version(first.version).unwrap().unwrap();
    // CRLF 已统一为 LF，入库正文就是 sha256 的字节。
    assert_eq!(
        stored.body.as_deref(),
        Some("# 规则\n\n## 准则① 永远说中文\n\n禁止绕过式修复。\n")
    );
    let v2 = store
        .submit_desired(
            desired(
                PolicyKind::Rules,
                "通用工作规则.md",
                Some("# 规则\n\n## 准则① 永远说中文\n\n可以随便修。\n"),
            ),
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    let report = v2.report.clone().unwrap();
    assert_eq!(report.expansions.len(), 1);
    assert_eq!(report.expansions[0].path, "old:5");
    assert!(report.expansions[0].note.contains("约束性语句"));
    assert_eq!(report.diff.removed, 1);
    assert_eq!(report.diff.added, 1);
    let hash = store.effective().unwrap().effective_hash;
    let second = store.activate(&v2.id, &hash, "owner", NOW).unwrap();
    assert_eq!(second.version, 2);
    // 回滚到 v1：新建 v3，正文等于 v1，历史不变。
    assert!(matches!(
        store.rollback(1, &hash, "owner", NOW),
        Err(PolicyError::HashMismatch { .. })
    ));
    let current = store.effective().unwrap().effective_hash;
    let rolled = store.rollback(1, &current, "owner", NOW).unwrap();
    assert_eq!(rolled.version, 3);
    assert_eq!(rolled.rollback_of, Some(1));
    assert_eq!(rolled.body_sha256, first.body_sha256);
    assert_eq!(store.rules().unwrap()[0].body, stored.body);
    assert!(matches!(
        store.rollback(99, &rolled.effective_hash, "owner", NOW),
        Err(PolicyError::VersionNotFound)
    ));
    let versions = store
        .versions(Some(PolicyKind::Rules), Some("通用工作规则.md"), 10)
        .unwrap();
    assert_eq!(
        versions.iter().map(|meta| meta.version).collect::<Vec<_>>(),
        [3, 2, 1]
    );
    // 规则 subject 校验在入库之前：路径分量、隐藏名、非 .md 与空名都不落库。
    for subject in ["a/b.md", ".hidden.md", "rules.txt", ""] {
        assert!(
            matches!(
                store.submit_desired(
                    desired(PolicyKind::Rules, subject, Some("# x\n")),
                    Submitter::Owner,
                    NOW,
                ),
                Err(PolicyError::InvalidSubject)
            ),
            "{subject}"
        );
    }
    assert_eq!(store.desired_list(None, 100).unwrap().len(), 2);
}

#[test]
fn three_level_revocation_is_immediate_and_restore_only_goes_through_activation() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let record = store
        .submit_desired(
            desired(
                PolicyKind::CapabilitySet,
                "claude",
                Some(&capability(&["项目/Studiolo"], &[], true)),
            ),
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    let hash = store.effective().unwrap().effective_hash;
    store.activate(&record.id, &hash, "owner", NOW).unwrap();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    let ok = agent.authorization("claude", Some("01INSTANCE")).unwrap();
    assert!(ok.denial.is_none());
    assert_eq!(
        ok.grant.as_ref().unwrap().set.read.project_whitelist,
        ["项目/Studiolo"]
    );
    // 未登记类型零权限。
    let none = agent.authorization("grok", Some("01OTHER")).unwrap();
    assert_eq!(none.denial, Some(Denial::NoCapabilitySet));
    assert!(none.grant.is_none());

    assert!(
        store
            .revoke(
                RevocationLevel::Instance,
                "01INSTANCE",
                "owner",
                "泄露",
                NOW
            )
            .unwrap()
    );
    assert!(
        !store
            .revoke(
                RevocationLevel::Instance,
                "01INSTANCE",
                "owner",
                "重复",
                NOW
            )
            .unwrap()
    );
    assert_eq!(
        agent
            .authorization("claude", Some("01INSTANCE"))
            .unwrap()
            .denial,
        Some(Denial::InstanceRevoked)
    );
    assert!(
        agent
            .authorization("claude", Some("01SIBLING"))
            .unwrap()
            .denial
            .is_none()
    );

    assert!(
        store
            .revoke(RevocationLevel::Type, "claude", "owner", "全员停", NOW)
            .unwrap()
    );
    assert_eq!(
        agent
            .authorization("claude", Some("01SIBLING"))
            .unwrap()
            .denial,
        Some(Denial::TypeRevoked)
    );
    // 恢复类型 = 重新发布该类型能力集（L3 路径），吊销随之解除。
    let again = store
        .submit_desired(
            desired(
                PolicyKind::CapabilitySet,
                "claude",
                Some(&capability(&["项目/Studiolo"], &[], true)),
            ),
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    let hash = store.effective().unwrap().effective_hash;
    let restored = store.activate(&again.id, &hash, "owner", NOW).unwrap();
    assert!(restored.cleared_type_revocation);
    assert!(
        agent
            .authorization("claude", Some("01SIBLING"))
            .unwrap()
            .denial
            .is_none()
    );
    // 实例吊销不随类型恢复而恢复。
    assert_eq!(
        agent
            .authorization("claude", Some("01INSTANCE"))
            .unwrap()
            .denial,
        Some(Denial::InstanceRevoked)
    );

    assert!(matches!(
        store.release_global("owner", NOW),
        Err(PolicyError::NotRevoked)
    ));
    assert!(
        store
            .revoke(RevocationLevel::Global, "ignored", "owner", "紧急", NOW)
            .unwrap()
    );
    assert!(store.effective().unwrap().global_revoked);
    assert_eq!(
        agent
            .authorization("claude", Some("01SIBLING"))
            .unwrap()
            .denial,
        Some(Denial::GlobalRevoked)
    );
    store.release_global("owner", NOW).unwrap();
    assert!(
        agent
            .authorization("claude", Some("01SIBLING"))
            .unwrap()
            .denial
            .is_none()
    );
    let active = store.revocations(true).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].level, RevocationLevel::Instance);
    assert_eq!(store.revocations(false).unwrap().len(), 3);
    assert!(matches!(
        store.revoke(RevocationLevel::Type, "Not Valid", "owner", "x", NOW),
        Err(PolicyError::InvalidSubject)
    ));
}

#[test]
fn bootstrap_only_writes_the_first_version() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let first = store
        .bootstrap(
            desired(
                PolicyKind::CapabilitySet,
                "claude",
                Some(&capability(&["_规则"], &[], false)),
            ),
            NOW,
        )
        .unwrap();
    assert_eq!(first.version, 1);
    assert_eq!(first.activated_by, "bootstrap");
    assert!(matches!(
        store.bootstrap(
            desired(
                PolicyKind::CapabilitySet,
                "claude",
                Some(&capability(&["项目/Studiolo"], &[], false)),
            ),
            NOW,
        ),
        Err(PolicyError::AlreadyBootstrapped)
    ));
    assert!(matches!(
        store.bootstrap(desired(PolicyKind::CapabilitySet, "grok", Some("{}")), NOW,),
        Err(PolicyError::ValidationFailed)
    ));
    assert!(matches!(
        store.bootstrap(desired(PolicyKind::Rules, "x.md", None), NOW),
        Err(PolicyError::InvalidBody(_))
    ));
    assert_eq!(store.effective().unwrap().entries.len(), 1);
}

#[test]
fn agent_proposals_are_bounded_pending_and_never_change_effective() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let agent = PolicyAgentStore::open_existing(&dir).unwrap();
    let before = store.effective().unwrap().effective_hash;
    let proposal = agent
        .propose(
            "claude",
            "01INSTANCE",
            ProposalInput {
                kind: PolicyKind::CapabilitySet,
                subject: "claude".into(),
                body: capability(&["School"], &[], true),
            },
            NOW,
        )
        .unwrap()
        .record;
    assert_eq!(proposal.status, DesiredStatus::Pending);
    assert_eq!(proposal.submitter_kind, "agent");
    assert_eq!(proposal.agent_instance.as_deref(), Some("01INSTANCE"));
    assert!(proposal.report.as_ref().unwrap().ok);
    assert_eq!(store.effective().unwrap().effective_hash, before);
    assert!(store.capability_set("claude").unwrap().is_none());
    assert_eq!(store.effective().unwrap().pending_desired, 1);
    // 主人复核：仍不生效；发布必须显式。
    let validated = store.validate_desired(&proposal.id, NOW).unwrap();
    assert_eq!(validated.status, DesiredStatus::Validated);
    assert_eq!(store.effective().unwrap().effective_hash, before);
    // 结构无效的提案直接 rejected。
    let bad = agent
        .propose(
            "claude",
            "01INSTANCE",
            ProposalInput {
                kind: PolicyKind::Skills,
                subject: "x".into(),
                body: "{}".into(),
            },
            NOW,
        )
        .unwrap()
        .record;
    assert_eq!(bad.status, DesiredStatus::Rejected);
    // 每实例上限；各份正文不同（同一正文连提只算一份，T-182）。
    for index in 0..MAX_PENDING_PER_INSTANCE {
        let _ = agent.propose(
            "claude",
            "01INSTANCE",
            ProposalInput {
                kind: PolicyKind::Rules,
                subject: "提案.md".into(),
                body: format!("# 提议 {index}\n"),
            },
            NOW,
        );
    }
    let count = store
        .desired_list(Some(DesiredStatus::Pending), 100)
        .unwrap()
        .iter()
        .filter(|record| record.agent_instance.as_deref() == Some("01INSTANCE"))
        .count();
    assert_eq!(count, MAX_PENDING_PER_INSTANCE);
    assert!(matches!(
        agent.propose(
            "claude",
            "01INSTANCE",
            ProposalInput {
                kind: PolicyKind::Rules,
                subject: "提案.md".into(),
                body: "# 再来\n".into(),
            },
            NOW,
        ),
        Err(PolicyError::TooManyProposals)
    ));
    // 撤回后不再计入。
    let withdrawn = store.withdraw_desired(&proposal.id, NOW).unwrap();
    assert_eq!(withdrawn.status, DesiredStatus::Withdrawn);
    assert!(matches!(
        store.withdraw_desired(&proposal.id, NOW),
        Err(PolicyError::DesiredNotActivatable(_))
    ));
    // 发布其中最新的一份提案后，同 subject 其余较早的 pending 被取代（较早的发不出，T-181）。
    let (pending, newer) = store.desired_queue(100).unwrap();
    let chosen = pending
        .iter()
        .find(|record| record.subject == "提案.md" && newer[&record.id].is_empty())
        .unwrap();
    let hash = store.effective().unwrap().effective_hash;
    store.activate(&chosen.id, &hash, "owner", NOW).unwrap();
    let superseded = store
        .desired_list(Some(DesiredStatus::Superseded), 100)
        .unwrap();
    assert!(superseded.iter().all(|record| record.subject == "提案.md"));
    assert!(superseded.len() >= MAX_PENDING_PER_INSTANCE - 1);
    assert_eq!(store.rules().unwrap().len(), 1);
}

#[test]
fn extra_tables_or_wrong_header_make_the_store_corrupt() {
    let (_temp, dir) = private_dir();
    drop(PolicyStore::open(&dir).unwrap());
    {
        let connection = rusqlite::Connection::open(dir.join("policy.sqlite3")).unwrap();
        connection
            .execute_batch("CREATE TABLE sneaky (x INTEGER)")
            .unwrap();
    }
    assert!(matches!(
        PolicyStore::open_existing(&dir),
        Err(PolicyError::CorruptStore)
    ));
    assert!(matches!(
        PolicyAgentStore::open_existing(&dir),
        Err(PolicyError::CorruptStore)
    ));
    let (_temp2, dir2) = private_dir();
    drop(PolicyStore::open(&dir2).unwrap());
    {
        let connection = rusqlite::Connection::open(dir2.join("policy.sqlite3")).unwrap();
        connection.pragma_update(None, "user_version", 7).unwrap();
    }
    assert!(matches!(
        PolicyStore::open_existing(&dir2),
        Err(PolicyError::CorruptStore)
    ));
    // 目录权限不是 0700 即拒绝。
    let (_temp3, dir3) = private_dir();
    fs::set_permissions(&dir3, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(matches!(
        PolicyStore::open(&dir3),
        Err(PolicyError::UnsafeStorage)
    ));
}
