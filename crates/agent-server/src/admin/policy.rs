//! 策略平面的可信本地子命令：初始化私有库、首版 bootstrap、状态查看。
//!
//! bootstrap 是 M4 唯一不经 L3 One-time Code 的写入路径，因此限制得很死：
//! 只在真实前台控制终端运行（与 `type-issue` 同一道门禁）、只允许为从未有过版本的 subject
//! 写 version 1（改已有版本一律拒绝）、结果进权威审计账本。后续一切变更走人侧 L3 发布。

use std::path::Path;

use studiolo_audit_ledger::{AuditLedger, SqliteAuditLedger};
use studiolo_policy::{DesiredInput, MAX_BODY_BYTES, PolicyKind, PolicyStore, audit};

use super::terminal::ControllingTerminal;
use crate::state::now;

pub(super) fn init(policy: &Path) -> Result<(), String> {
    PolicyStore::open(policy).map_err(|error| format!("初始化策略私有库失败：{error}"))?;
    println!("已初始化：policy.sqlite3（effective 为空：任何类型都是零权限，直到发布其能力集）");
    Ok(())
}

pub(super) fn bootstrap(
    policy: &Path,
    data: &Path,
    kind: &str,
    subject: &str,
    file: &Path,
) -> Result<(), String> {
    // 门禁在读任何输入之前：没有控制终端就什么都不做。
    let terminal = ControllingTerminal::open().map_err(|error| {
        format!(
            "拒绝 bootstrap：{error}。\n本次未读取文件、未打开策略库、未写入任何版本。\
             请在交互终端直接运行 `studiolo-agent admin policy-bootstrap`。"
        )
    })?;
    let kind = PolicyKind::parse(kind).map_err(|error| error.to_string())?;
    // 唯一的文件读取：主人显式指定的正文文件，有界，不跟随目录遍历。
    let bytes = std::fs::read(file).map_err(|_| "无法读取正文文件（不回显路径）".to_owned())?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(format!("正文超过上限 {MAX_BODY_BYTES} 字节"));
    }
    let body = String::from_utf8(bytes).map_err(|_| "正文不是 UTF-8".to_owned())?;
    // 账本先于策略库：账本不可用就不发布，避免留下无审计的版本。
    let ledger = SqliteAuditLedger::open_existing(&data.join("audit"))
        .map_err(|_| "审计账本不可用或未初始化；请先运行 admin init".to_owned())?;
    let store = PolicyStore::open_existing(policy).map_err(|error| error.to_string())?;
    let input = DesiredInput {
        kind,
        subject: subject.to_owned(),
        body: Some(body),
    };
    // 先校验再发布：校验不过就把逐条问题打到标准输出（不含正文），不写任何版本。
    let report = store.preview(&input).map_err(|error| error.to_string())?;
    if !report.ok {
        let problems = report
            .problems
            .iter()
            .map(|problem| format!("  - {}：{}", problem.path, problem.message))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("校验未通过，未写入任何版本：\n{problems}"));
    }
    let activation = store
        .bootstrap(input, &now())
        .map_err(|error| error.to_string())?;
    ledger
        .append(audit::bootstrapped(&activation).map_err(|_| "审计事件构造失败".to_owned())?)
        .map_err(|_| {
            format!(
                "版本 {} 已发布但审计追加失败：请立刻修复账本并手工补记（effective {}）",
                activation.version, activation.effective_hash
            )
        })?;
    terminal
        .show(&format!(
            "已 bootstrap：{} {} → 版本 {}，effective {}\n",
            activation.kind, activation.subject, activation.version, activation.effective_hash
        ))
        .map_err(|error| format!("版本已发布并已审计，但控制终端回显失败：{error}"))?;
    Ok(())
}

pub(super) fn status(policy: &Path) -> Result<(), String> {
    let store = PolicyStore::open_existing(policy).map_err(|error| error.to_string())?;
    let summary = store.effective().map_err(|error| error.to_string())?;
    println!("effective\t{}", summary.effective_hash);
    println!(
        "global_revoked\t{}\tpending_desired\t{}",
        summary.global_revoked, summary.pending_desired
    );
    for entry in &summary.entries {
        println!(
            "{}\t{}\tv{}\t{}\t{}",
            entry.kind,
            entry.subject,
            entry.version,
            &entry.body_sha256[..12],
            entry.activated_at
        );
    }
    for revocation in store.revocations(true).map_err(|error| error.to_string())? {
        println!(
            "revoked\t{}\t{}\t{}\t{}",
            revocation.level.as_str(),
            revocation.target,
            revocation.revoked_at,
            revocation.reason
        );
    }
    Ok(())
}
