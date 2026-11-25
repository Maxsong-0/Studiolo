//! 连接器入口守卫（`connector_entry`）的反面：在真源码上改错一处，每一种都得判红。
use crate::connector_entry::{
    AGENT, GRANT, HUMAN, PRESENCE, agent_problems, grant_problems, human_problems,
    other_agent_sources, presence_problems, source,
};

/// 在真源码上改错一处；改不动就是用例过时了。
fn changed(path: &str, before: &str, after: &str) -> String {
    let text = source(path);
    assert_eq!(text.matches(before).count(), 1, "{path}：{before}");
    text.replacen(before, after, 1)
}

#[test]
fn every_way_out_of_the_agent_boundary_is_caught() {
    for (file, before, after) in [
        (
            0,
            "state::{AppState, now},",
            "state::{AppState, now}, freshness,",
        ),
        (
            0,
            "use limits::Limits;",
            "use limits::Limits;\nuse crate::server::grant;",
        ),
        (
            2,
            "/// 每个答复都带的头",
            "use crate::state::AppState;\n/// 每个答复都带的头",
        ),
        (
            0,
            "audit::{self, Actor};",
            "{audit::{self, Actor}, ReadOnlyVault};",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let _ = &self.app.allow;\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let whole = &self.app; let _ = &whole.allow;\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let whole = self.app.clone(); let _ = &whole.allow;\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "use limits::Limits;",
            "use limits::Limits;\nuse super::server::grant;",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let AppState { allow, .. } = &*self.app;\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "self.recheck(caller, Entry::Board, &instance, &granted)?;",
            "let _ = self.recheck(caller, Entry::Board, &instance, &granted);",
        ),
        (
            0,
            "self.recheck(caller, Entry::Task, &instance, &granted)?;",
            "// self.recheck(caller, Entry::Task, &instance, &granted)?;",
        ),
        (
            0,
            "self.recheck(caller, Entry::Sessions, &instance, &granted)?;",
            "#[cfg(test)]\n        self.recheck(caller, Entry::Board, &instance, &granted)?;",
        ),
        (
            0,
            "self.record(caller, &instance, &granted, name, Entry::Board)?;",
            "let _ = self.record(caller, &instance, &granted, name, Entry::Board);",
        ),
        (
            0,
            "self.record(caller, &instance, &granted, project, Entry::Task)?;",
            "",
        ),
        // 多一个守卫看不见的子模块。
        (0, "mod reply;", "mod reply;\nmod escape;"),
        (
            1,
            "#[derive(Default)]\npub(super) struct Limits {",
            "mod escape;\n#[derive(Default)]\npub(super) struct Limits {",
        ),
        // 拆开自己的字段拿走整个 `AppState`。
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let Connect { app: whole, .. } = self; let _ = &whole.allow;\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let _ = elsewhere(&self.app, 1);\n        let Some(policy) = &self.app.policy else {",
        ),
        // 不先收完正文、不排名额、不取操作准入就裁。
        (
            0,
            "let Ok(_slot) = state.authority.acquire().await else {\n        return UNAVAILABLE.into_response();\n    };",
            "",
        ),
        (
            0,
            "let Some(_operation) = state.app.begin_operation().await else {\n        return UNAVAILABLE.into_response();\n    };",
            "let _ = state.app.begin_operation().await;",
        ),
        (
            0,
            "Entry::Sessions => match read(body).await {\n            Ok(body) => body,\n            Err(refusal) => return refusal.into_response(),\n        },\n    };",
            "Entry::Sessions => Bytes::new(),\n    };",
        ),
        // 从句柄上调名单外的方法、写全路径、带进别的文件。
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "self.app.store.issue_type(1);\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let store = &self.app.store;\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let board = self.app.board.as_ref(); board.call(1);\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let policy = self.app.policy.clone();\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "std::fs::read(a);\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "tokio::fs::read(a);\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "crate::freshness::now();\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "studiolo_supply::ReadOnlyVault::open(a);\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "audit::instance_revoked(a);\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "AppState::open(a);\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "include!(\"escape.rs\");\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "fn f(whole: &AppState) {}\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "        client\n            .connect(&json!({",
            "        let _ = client.call(1);\n        client\n            .connect(&json!({",
        ),
        (
            0,
            "        client\n            .connect(&json!({",
            "        client\n            .call(&json!({",
        ),
        (
            0,
            "let authorization = policy\n",
            "let _ = policy.publish(1);\n        let authorization = policy\n",
        ),
        (
            0,
            "let authorization = policy\n            .authorization(",
            "let authorization = policy\n            .publish(",
        ),
        (0, "mod limits;", "#[path = \"../escape.rs\"]\nmod limits;"),
        (
            1,
            "const PER_MINUTE: usize = 12;",
            "include!(\"escape.rs\");\nconst PER_MINUTE: usize = 12;",
        ),
        (
            2,
            "use axum::{\n    Json,",
            "use axum::{\n    extract::State,\n    Json,",
        ),
        // 没认出的拒绝不限频。
        (
            0,
            "            state.deny_anonymous(refused);\n",
            "            let _ = state.app.audit(every(refused));\n            state.deny_anonymous(refused);\n",
        ),
    ] {
        let mut sources = AGENT.map(source);
        sources[file] = changed(AGENT[file], before, after);
        assert!(
            !agent_problems(&sources).is_empty(),
            "改成「{after}」没判红"
        );
    }
}

#[test]
fn every_way_of_writing_more_on_the_human_side_is_caught() {
    for (before, after) in [
        // 每份快照都登记、逐条状态进账本。
        (
            "for fresh in &planned.fresh {",
            "for fresh in &planned.all {",
        ),
        (
            "presence.commit(&call.agent_type,",
            "let _ = self.record(json!({\"seq\": 1}));\n        presence.commit(&call.agent_type,",
        ),
        (
            "presence.commit(&call.agent_type,",
            "let _ = self.audit.append(event);\n        presence.commit(&call.agent_type,",
        ),
        // 登记不了也照收。
        (
            "self.record_fresh(&call, fresh)\n                    .map_err(|_| Failure::Unavailable)?;",
            "let _ = self.record_fresh(&call, fresh);",
        ),
        // 拒绝不限频。
        (
            "if !presence.denial_due(&call.agent_type, reason, now) {\n            return;\n        }",
            "",
        ),
        // 改工作项。
        (
            "let mut lineages = Lineages::default();",
            "let engine = self.engine.lock();\n            engine.board_cancel(1);\n        let mut lineages = Lineages::default();",
        ),
        (
            ".board_detail(task_id, &grant.hash)",
            ".board_claim(task_id, &grant.hash)",
        ),
        (
            ".board_list_all(&grant.hash)",
            ".board_reconcile(&grant.hash)",
        ),
        // 从 `self`、`plane` 上调 `BoardPlane` 写工作项的现成方法，或换个名字带走再调。
        (
            "let mut lineages = Lineages::default();",
            "let _ = self.execute(request);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "let whole = self;\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "elsewhere(self, 1);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "elsewhere(&self);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "elsewhere(self);\n        let mut lineages = Lineages::default();",
        ),
        (
            "        plane.connect(call)\n",
            "        elsewhere(plane)\n",
        ),
        (
            "let mut lineages = Lineages::default();",
            "BoardPlane::execute(1);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "let _ = Self::execute;\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "super::execute::run(1);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "crate::writer::run(1);\n        let mut lineages = Lineages::default();",
        ),
        (
            "        plane.connect(call)\n",
            "        plane.execute(call)\n",
        ),
        (
            "        plane.connect(call)\n",
            "        let whole = plane;\n        whole.execute(call)\n",
        ),
        (
            "execute::{Failure as BoardFailure, parse, value},",
            "execute::{Failure as BoardFailure, parse, run, value},",
        ),
        (
            "fn board_roster(set: &CapabilitySet)",
            "mod escape;\nfn board_roster(set: &CapabilitySet)",
        ),
        // 同一件事的别的写法：解引用、写全路径、放在别的参数位、起别名、从句柄上调写方法、带进别的文件。
        (
            "let mut lineages = Lineages::default();",
            "(*self).execute(request);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "<Self>::execute(&*self, request);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "elsewhere(&*self, 1);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "elsewhere(1, self);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "let (_, whole) = (0, self); whole.execute(request);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "match self { whole => whole.execute(request) };\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "match self { whole if true => whole.execute(request), _ => {} };\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "let run = |_: u8, whole: &Self| whole.execute(request);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "studiolo_audit_ledger::AuditLedger::append(&*self.audit, event);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "let ledger = &self.audit;\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "let held = self.engine.clone();\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "self.identities.issue_type(1);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "std::fs::write(a, b);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "::std::fs::write(a, b);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "tokio::fs::write(a, b);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "include!(\"escape.rs\");\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "requests::run(1);\n        let mut lineages = Lineages::default();",
        ),
        (
            "        plane.connect(call)\n",
            "        elsewhere(1, plane)\n",
        ),
        (
            "        plane.connect(call)\n",
            "        (*plane).execute(call)\n",
        ),
        (
            "        plane.connect(call)\n",
            "        plane.connect(call);\n        plane.execute(call)\n",
        ),
        (
            "    Extension(plane): Extension<Arc<BoardPlane>>,\n    call:",
            "    Extension(whole): Extension<Arc<BoardPlane>>,\n    call:",
        ),
        (
            "    plane: Option<Extension<Arc<BoardPlane>>>,\n",
            "    whole: Option<Extension<Arc<BoardPlane>>>,\n",
        ),
        (
            "let Some(Extension(plane)) = plane else {",
            "let Some(Extension(whole)) = plane else {",
        ),
        (
            "use crate::editing::Problem;",
            "use crate::editing::{self, Problem};",
        ),
        (
            "use studiolo_policy::CapabilitySet;",
            "use studiolo_policy::{CapabilitySet, PolicyStore};",
        ),
        (
            "use std::{\n    collections::BTreeMap,",
            "use std::{\n    fs,\n    collections::BTreeMap,",
        ),
        // 交付前不再复核。
        (
            "self.confirm(call, board_roster, &grant)?;\n        Ok(json!({",
            "Ok(json!({",
        ),
        (
            "self.confirm(call, board_roster, &grant)?;\n        value(view)",
            "let _ = self.confirm(call, board_roster, &grant);\n        value(view)",
        ),
    ] {
        assert!(
            !human_problems(&changed(HUMAN, before, after)).is_empty(),
            "改成「{after}」没判红"
        );
    }
}

#[test]
fn presence_reaching_outside_is_caught() {
    for addition in [
        "fn later() { let _ = std::time::Instant::now(); }",
        "fn later() { let _ = std::fs::read(\"x\"); }",
        "use std::time::SystemTime;",
        "fn later() { let _ = jiff::Timestamp::now(); }",
        "use crate::editing::Problem;",
        "use super::BoardPlane;",
        "extern crate tokio as harmless;",
        "mod escape;",
        "mod escape { pub fn now() {} }",
        "fn later() { let _ = env!(\"HOME\"); }",
        "fn later() { println![\"x\"]; }",
        "include!(\"escape.rs\");",
        "use studiolo_policy::PolicyStore;",
        "use studiolo_supply::AgentStore;",
        "use studiolo_audit_ledger::SqliteAuditLedger;",
        "fn later() { let _ = studiolo_policy::PolicyAgentStore::open_existing(x); }",
        "fn later() { let _ = ::std::time::Instant::now(); }",
        "type Clock = std::time::Instant;",
        "fn later() { let _ = x.collect::<::std::time::Instant>(); }",
        "fn later() { let _ = <VaultPath>::now(); }",
        "fn later() { unsafe { now() } }",
    ] {
        let text = changed(
            PRESENCE,
            "use studiolo_core::VaultPath;",
            &format!("use studiolo_core::VaultPath;\n{addition}"),
        );
        assert!(
            !presence_problems(&text).is_empty(),
            "加上「{addition}」没判红"
        );
    }
}

#[test]
fn letting_connector_types_into_mcp_is_caught() {
    let others = other_agent_sources();
    for (before, after) in [
        (
            "if grant.set.connector_only() {",
            "if false && grant.set.connector_only() {",
        ),
        (
            "if grant.set.connector_only() {",
            "if !grant.set.connector_only() {",
        ),
        (
            "if grant.set.connector_only() {",
            "if grant.set.write.board {",
        ),
        (
            "        let whitelist = grant",
            "        if grant.set.connector_only() {}\n        let whitelist = grant",
        ),
        // 另开一条交授权的路。
        (
            "        let whitelist = grant",
            "        if early { return Ok(Grant { whitelist: all }); }\n        let whitelist = grant",
        ),
        (
            "        let allowed = intersect_prefixes(",
            "        let early: Grant = all;\n        let allowed = intersect_prefixes(",
        ),
        // 接了策略平面也走兼容模式那一份。
        (
            "let Some(policy) = &self.state.policy else {",
            "let Some(policy) = self.state.policy.as_ref().filter(|_| strict) else {",
        ),
    ] {
        assert!(
            !grant_problems(&changed(GRANT, before, after), &others).is_empty(),
            "改成「{after}」没判红"
        );
    }
    // 别处另判一遍也算错：两处会走偏；别处另建一份授权就绕过了那一判。
    for elsewhere in [
        "fn f() { if grant.set.connector_only() { return; } }",
        "fn f() -> Result<Grant, Error> { Ok(Grant { instance, allowed, policy: None }) }",
    ] {
        let others = [elsewhere.to_owned()];
        assert!(
            !grant_problems(&source(GRANT), &others).is_empty(),
            "{elsewhere}"
        );
    }
}
