use super::*;
use serde_json::{Value, json};

const A: &str = "项目/Studiolo-Docs";
const B: &str = "项目/HomeLab-Center-Docs";
const SOURCE: &str = "homelab-status";

fn at(secs: u64) -> Duration {
    Duration::from_secs(secs)
}

fn allowed(projects: &[&str]) -> Vec<String> {
    projects.iter().map(|project| (*project).into()).collect()
}

fn key(index: usize) -> String {
    format!("{index:016x}")
}

fn session(index: usize, project: &str) -> Value {
    json!({"kind":"session","key":key(index),"vendor":"claude_code","project":project,
        "state":"running"})
}

fn full(index: usize) -> Value {
    json!({"kind":"session","key":key(index),"vendor":"codex","project":A,"branch":"claude/t-237",
        "state":"waiting","step":2,"total":5,"last_action":"edit","idle_secs":7})
}

fn snapshot(seq: u64, sessions: &[Value]) -> Snapshot {
    Snapshot::parse(json!({"seq":seq,"sessions":sessions})).expect("合法快照")
}

/// 照生产的顺序推一次：核 → 登记新会话 → 落下。
fn push(
    presence: &mut Presence,
    source: &str,
    snapshot: Snapshot,
    projects: &[&str],
    now: Duration,
) -> Result<(usize, Vec<usize>, usize), Refused> {
    let planned = presence.plan(source, snapshot, &allowed(projects), now)?;
    let reply = (
        planned.accepted(),
        planned.rejected.clone(),
        planned.fresh.len(),
    );
    for _ in &planned.fresh {
        presence.note_record(source, now);
    }
    presence.commit(source, "instance", planned, now);
    Ok(reply)
}

fn view(presence: &Presence, now: Duration) -> Vec<Value> {
    presence
        .view(now, None)
        .into_iter()
        .map(|seen| serde_json::to_value(seen).unwrap())
        .collect()
}

#[test]
fn a_snapshot_replaces_the_whole_previous_one_and_is_one_row_per_session() {
    let mut presence = Presence::default();
    push(
        &mut presence,
        SOURCE,
        snapshot(1, &[full(1), session(2, B)]),
        &[A, B],
        at(0),
    )
    .unwrap();
    assert_eq!(view(&presence, at(0)).len(), 2);
    // 同一编号再推是盖掉，不是多一条；快照里没了的当场消失。
    let mut changed = full(1);
    changed["state"] = json!("idle");
    push(
        &mut presence,
        SOURCE,
        snapshot(2, &[changed]),
        &[A, B],
        at(30),
    )
    .unwrap();
    let rows = view(&presence, at(30));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["state"], "idle");
    // 空快照就是清空。
    push(&mut presence, SOURCE, snapshot(3, &[]), &[A, B], at(60)).unwrap();
    assert!(view(&presence, at(60)).is_empty());
}

#[test]
fn rows_carry_only_the_agreed_fields_and_never_the_key() {
    let mut presence = Presence::default();
    push(
        &mut presence,
        SOURCE,
        snapshot(1, &[full(1), session(2, B)]),
        &[A, B],
        at(100),
    )
    .unwrap();
    let rows = view(&presence, at(110));
    let mut names = rows[0]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    names.sort();
    // 没带的不出现，不给 null、不给「未知」。
    assert_eq!(
        rows[0],
        json!({"source":SOURCE,"vendor":"claude_code","project":B,"state":"running","age_secs":10})
    );
    assert_eq!(names, ["age_secs", "project", "source", "state", "vendor"]);
    assert_eq!(
        rows[1],
        json!({"source":SOURCE,"vendor":"codex","project":A,"branch":"claude/t-237",
            "state":"waiting","step":2,"total":5,"last_action":"edit","idle_secs":17,"age_secs":10})
    );
    let text = serde_json::to_string(&rows).unwrap();
    assert!(!text.contains(&key(1)) && !text.contains(&key(2)), "{text}");
    // 只看一个项目。
    assert_eq!(presence.view(at(110), Some(A)).len(), 1);
    assert!(presence.view(at(110), Some("项目/Other")).is_empty());
}

#[test]
fn absent_and_null_optional_fields_mean_the_same() {
    let mut value = session(1, A);
    for field in ["branch", "step", "total", "last_action", "idle_secs"] {
        value[field] = Value::Null;
    }
    let with_null = snapshot(1, &[value]);
    let without = snapshot(1, &[session(1, A)]);
    assert_eq!(with_null.sessions, without.sessions);
}

#[test]
fn any_bad_field_refuses_the_whole_snapshot() {
    let good = || vec![session(1, A), full(2)];
    assert!(Snapshot::parse(json!({"seq":1,"sessions":good()})).is_some());
    let changes: Vec<(&str, Value)> = vec![
        ("kind", json!("card")),
        ("kind", Value::Null),
        ("key", json!("short")),
        ("key", json!("ABCDEF0123456789")),
        ("key", json!("g123456789abcdef")),
        ("key", json!("0".repeat(65))),
        ("vendor", json!("unknown")),
        ("vendor", json!("grok")),
        ("project", json!("项目")),
        ("project", json!("项目/A/B")),
        ("project", json!("_规则/A")),
        ("project", json!("../项目/A")),
        ("state", json!("unknown")),
        ("state", json!("done")),
        ("state", Value::Null),
        ("branch", json!("")),
        ("branch", json!("a b")),
        ("branch", json!("a\nb")),
        ("branch", json!("a\u{202e}b")),
        ("branch", json!("a\u{200b}b")),
        ("branch", json!("a\u{600}b")),
        ("branch", json!("a\u{206f}b")),
        ("branch", json!("a\u{e0041}b")),
        ("branch", json!("a\u{e007f}b")),
        ("branch", json!("a\u{1d173}b")),
        ("branch", json!("a..b")),
        ("branch", json!("-a")),
        ("branch", json!("/a")),
        ("branch", json!("a/")),
        ("branch", json!("a//b")),
        ("branch", json!("a/.b")),
        ("branch", json!("a.lock")),
        ("branch", json!("a~1")),
        ("branch", json!("a@{1}")),
        ("branch", json!("x".repeat(121))),
        ("branch", json!("sk-abcdefghijkl0123")),
        ("branch", json!("feat/ghp_abcdefghijkl0123")),
        ("branch", json!("eyJhbGciOiJIUzI1NiJ9")),
        ("branch", json!("0123456789abcdef0123456789abcdef")),
        ("step", json!(1000)),
        ("step", json!(-1)),
        ("step", json!(1.5)),
        ("step", json!("1")),
        ("total", json!(1000)),
        ("total", json!(1)),
        ("last_action", json!("unknown")),
        ("last_action", json!("Bash")),
        ("idle_secs", json!(86_401)),
        ("idle_secs", json!("unknown")),
        ("title", json!("会话标题")),
        ("command", json!("ls")),
    ];
    for (field, value) in changes {
        let mut sessions = good();
        sessions[1][field] = value.clone();
        assert!(
            Snapshot::parse(json!({"seq":1,"sessions":sessions})).is_none(),
            "{field} = {value}"
        );
    }
    for required in ["kind", "key", "vendor", "project", "state"] {
        let mut sessions = good();
        sessions[0].as_object_mut().unwrap().remove(required);
        assert!(
            Snapshot::parse(json!({"seq":1,"sessions":sessions})).is_none(),
            "缺 {required}"
        );
    }
    // `total` 只能跟着 `step` 来。
    let mut alone = session(1, A);
    alone["total"] = json!(3);
    assert!(Snapshot::parse(json!({"seq":1,"sessions":[alone]})).is_none());
    // 没带 `total` 时 `step` 自己也封顶。
    for (step, accepted) in [(999, true), (1000, false)] {
        let mut row = session(1, A);
        row["step"] = json!(step);
        let parsed = Snapshot::parse(json!({"seq":1,"sessions":[row]}));
        assert_eq!(parsed.is_some(), accepted, "{step}");
    }
    // 编号重复、条数超上限、`seq` 超出精确整数、顶层多字段。
    assert!(Snapshot::parse(json!({"seq":1,"sessions":[session(1, A), session(1, B)]})).is_none());
    let many = (0..=MAX_PER_SOURCE)
        .map(|i| session(i, A))
        .collect::<Vec<_>>();
    assert!(Snapshot::parse(json!({"seq":1,"sessions":many[1..].to_vec()})).is_some());
    assert!(Snapshot::parse(json!({"seq":1,"sessions":many})).is_none());
    assert!(Snapshot::parse(json!({"seq":MAX_SEQ,"sessions":[]})).is_some());
    assert!(Snapshot::parse(json!({"seq":MAX_SEQ + 1,"sessions":[]})).is_none());
    assert!(Snapshot::parse(json!({"seq":1,"sessions":[],"host":"x"})).is_none());
    assert!(Snapshot::parse(json!({"sessions":[]})).is_none());
}

#[test]
fn ordinary_branch_names_are_not_mistaken_for_secrets() {
    for name in [
        "main",
        "claude/t-237-connector",
        "task-237-something-long-enough",
        "desk-lamp-with-many-words",
        "修复/黑板-状态",
        "dependabot/npm_and_yarn/web/vite-7.1.2",
        "release/2026.11.24",
    ] {
        assert!(branch_name(name), "{name}");
    }
}

#[test]
fn everything_disappears_after_three_hundred_seconds_without_a_new_snapshot() {
    let mut presence = Presence::default();
    push(&mut presence, SOURCE, snapshot(1, &[full(1)]), &[A], at(10)).unwrap();
    assert_eq!(view(&presence, at(309)).len(), 1);
    assert!(view(&presence, at(310)).is_empty());
    presence.sweep(at(309));
    assert_eq!(presence.sources().len(), 1);
    presence.sweep(at(310));
    assert!(presence.sources().is_empty());
    // 过期之后 `seq` 重新算：客户端换了计数起点也能接上。
    assert!(
        push(
            &mut presence,
            SOURCE,
            snapshot(1, &[full(1)]),
            &[A],
            at(311)
        )
        .is_ok()
    );
}

#[test]
fn an_old_or_repeated_sequence_is_refused_and_leaves_the_current_state() {
    let mut presence = Presence::default();
    push(&mut presence, SOURCE, snapshot(5, &[full(1)]), &[A], at(0)).unwrap();
    for seq in [0, 4, 5] {
        assert_eq!(
            push(&mut presence, SOURCE, snapshot(seq, &[]), &[A], at(1)),
            Err(Refused::Stale)
        );
    }
    assert_eq!(view(&presence, at(1)).len(), 1);
    // 空快照也占着 `seq`：晚到的旧快照不能把清掉的带回来。
    push(&mut presence, SOURCE, snapshot(6, &[]), &[A], at(2)).unwrap();
    assert_eq!(
        push(&mut presence, SOURCE, snapshot(5, &[full(1)]), &[A], at(3)),
        Err(Refused::Stale)
    );
    // 各来源各算各的。
    assert!(push(&mut presence, "other", snapshot(1, &[]), &[A], at(3)).is_ok());
}

#[test]
fn sessions_outside_the_roster_are_rejected_one_by_one() {
    let mut presence = Presence::default();
    let reply = push(
        &mut presence,
        SOURCE,
        snapshot(1, &[session(1, B), full(2), session(3, "项目/Other")]),
        &[A],
        at(0),
    )
    .unwrap();
    assert_eq!(reply, (1, vec![0, 2], 1));
    assert_eq!(view(&presence, at(0)).len(), 1);
}

#[test]
fn losing_the_grant_drops_the_rows_at_once_and_they_do_not_come_back() {
    let mut presence = Presence::default();
    push(
        &mut presence,
        SOURCE,
        snapshot(1, &[full(1), session(2, B)]),
        &[A, B],
        at(0),
    )
    .unwrap();
    presence.restrict(SOURCE, Some(&allowed(&[A])));
    assert_eq!(view(&presence, at(1)).len(), 1);
    presence.restrict(SOURCE, Some(&allowed(&[A, B])));
    assert_eq!(view(&presence, at(1)).len(), 1, "再放开也不许带回收回过的");
    presence.restrict(SOURCE, None);
    assert!(presence.sources().is_empty());
    assert!(view(&presence, at(1)).is_empty());
}

#[test]
fn only_sessions_seen_for_the_first_time_are_recorded_grouped_by_project_and_vendor() {
    let mut presence = Presence::default();
    let first = snapshot(1, &[session(1, A), session(2, A), full(3), session(4, B)]);
    let planned = presence
        .plan(SOURCE, first, &allowed(&[A, B]), at(0))
        .unwrap();
    assert_eq!(
        planned.fresh,
        [
            Fresh {
                project: B.into(),
                vendor: Vendor::ClaudeCode,
                count: 1
            },
            Fresh {
                project: A.into(),
                vendor: Vendor::ClaudeCode,
                count: 2
            },
            Fresh {
                project: A.into(),
                vendor: Vendor::Codex,
                count: 1
            },
        ]
    );
    presence.commit(SOURCE, "instance", planned, at(0));
    // 一小时、每 30 秒一次、5 个会话不断变化：只有第一次出现的那次要登记。
    let mut records = 0;
    for tick in 1..=120u64 {
        let sessions = (1..=5)
            .map(|index| {
                let mut row = session(index, A);
                row["step"] = json!(tick);
                row
            })
            .collect::<Vec<_>>();
        records += push(
            &mut presence,
            SOURCE,
            snapshot(tick + 1, &sessions),
            &[A, B],
            at(tick * 30),
        )
        .unwrap()
        .2;
    }
    assert!(records <= 10, "{records}");
    assert_eq!(records, 1);
}

#[test]
fn a_source_that_keeps_inventing_sessions_is_throttled_but_old_ones_still_update() {
    let mut presence = Presence::default();
    let mut last = 0;
    for index in 0..RECORDS_PER_HOUR {
        last = index;
        push(
            &mut presence,
            SOURCE,
            snapshot(index as u64 + 1, &[session(index, A)]),
            &[A],
            at(index as u64),
        )
        .unwrap();
    }
    assert_eq!(
        push(
            &mut presence,
            SOURCE,
            snapshot(1000, &[session(5000, A)]),
            &[A],
            at(100)
        ),
        Err(Refused::Throttled)
    );
    assert!(
        push(
            &mut presence,
            SOURCE,
            snapshot(1001, &[session(last, A)]),
            &[A],
            at(101)
        )
        .is_ok()
    );
    // 过一小时名额回来。
    assert!(
        push(
            &mut presence,
            SOURCE,
            snapshot(1002, &[session(5000, A)]),
            &[A],
            at(3700)
        )
        .is_ok()
    );
}

#[test]
fn the_total_across_sources_is_bounded() {
    let mut presence = Presence::default();
    let rows = (0..MAX_PER_SOURCE)
        .map(|i| session(i, A))
        .collect::<Vec<_>>();
    for index in 0..MAX_TOTAL / MAX_PER_SOURCE {
        push(
            &mut presence,
            &format!("source-{index}"),
            snapshot(1, &rows),
            &[A],
            at(0),
        )
        .unwrap();
    }
    assert_eq!(
        push(
            &mut presence,
            "one-more",
            snapshot(1, &[session(1, A)]),
            &[A],
            at(0)
        ),
        Err(Refused::Full)
    );
    // 自己那份是替换，不算超。
    assert!(push(&mut presence, "source-0", snapshot(2, &rows), &[A], at(1)).is_ok());
}

#[test]
fn the_same_denial_is_noted_once_a_minute() {
    let mut presence = Presence::default();
    assert!(presence.denial_due(SOURCE, "项目不在名单", at(0)));
    assert!(!presence.denial_due(SOURCE, "项目不在名单", at(59)));
    assert!(presence.denial_due(SOURCE, "别的原因", at(59)));
    assert!(presence.denial_due("other", "项目不在名单", at(59)));
    assert!(presence.denial_due(SOURCE, "项目不在名单", at(60)));
}
