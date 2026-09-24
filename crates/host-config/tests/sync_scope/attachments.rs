//! 附件闭集扩到 txt／json（ADR-025 D12），以及 `_规则/` 自 2026-09-23 起可由目录规则批准；
//! 保留文档名、AI 专属目录与隐藏路径在规则目录之下照样拒绝。

use super::fixture::*;
use serde_json::{Value, json};
use std::slice;
use studiolo_host_config::{
    SyncAttachmentType as Attachment, SyncFileKind as Kind, SyncScopeError as Error,
};

fn rules_folder(types: &[&str], max_bytes: u64) -> Value {
    json!({"prefix":"_规则","markdown":true,"attachment_types":types,"max_bytes":max_bytes})
}

#[test]
fn txt_and_json_join_the_closed_attachment_set_but_exe_stays_out() {
    let f = Fixture::new();
    let mut value = f.value.clone();
    value["sync_scope"]["whole_folders"] =
        json!([folder("笔记"), rules_folder(&["txt", "json"], 1_048_576)]);
    f.write_value(&value);
    assert!(f.load().is_ok());
    for bad in ["exe", "TXT", "text", "sh"] {
        value["sync_scope"]["whole_folders"][1]["attachment_types"] = json!([bad]);
        f.write_value(&value);
        assert!(f.load().is_err(), "{bad}");
    }
}

#[test]
fn txt_attachments_pass_only_under_a_rule_that_approved_txt() {
    let mut f = Fixture::new();
    f.value["sync_scope"]["whole_folders"] =
        json!([folder("笔记"), rules_folder(&["txt"], 1_048_576)]);
    f.write();
    let scope = f.scope();
    let approved = present("_规则/归档/x.txt", Kind::Attachment(Attachment::Txt), 12);
    let union = scope
        .approve_union(
            scope.binding(),
            &[],
            slice::from_ref(&approved),
            slice::from_ref(&approved),
        )
        .unwrap();
    assert_eq!(
        union.paths()[&path("_规则/归档/x.txt")].attachment_types(),
        &[Attachment::Txt]
    );
    // 类型是显式批准的：笔记目录没批 txt，规则目录没批 json。
    for unapproved in [
        present("笔记/a.txt", Kind::Attachment(Attachment::Txt), 12),
        present("_规则/b.json", Kind::Attachment(Attachment::Json), 12),
    ] {
        assert!(
            matches!(
                scope.approve_union(
                    scope.binding(),
                    &[],
                    slice::from_ref(&unapproved),
                    slice::from_ref(&unapproved)
                ),
                Err(Error::ScopeChangeRequired)
            ),
            "{}",
            unapproved.path
        );
    }
}

#[test]
fn rules_directory_is_approvable_by_a_folder_rule_while_reserved_names_stay_excluded() {
    let mut f = Fixture::new();
    f.value["sync_scope"]["whole_folders"] =
        json!([folder("笔记"), rules_folder(&["txt"], 1_048_576)]);
    f.write();
    let scope = f.scope();
    for approved in [
        markdown("_规则/通用工作规则.md"),
        present("_规则/归档/x.txt", Kind::Attachment(Attachment::Txt), 12),
    ] {
        let union = scope
            .approve_union(
                scope.binding(),
                &[],
                slice::from_ref(&approved),
                slice::from_ref(&approved),
            )
            .unwrap();
        assert!(
            union.paths().contains_key(&approved.path),
            "{}",
            approved.path
        );
    }
    for reserved in ["_规则/CLAUDE.md", "_规则/skills/a.md", "_规则/.private/x"] {
        let item = markdown(reserved);
        assert!(
            matches!(
                scope.approve_union(
                    scope.binding(),
                    &[],
                    slice::from_ref(&item),
                    slice::from_ref(&item)
                ),
                Err(Error::ScopeChangeRequired)
            ),
            "{reserved}"
        );
    }
}

#[test]
fn max_bytes_reports_the_largest_limit_across_folder_and_file_rules() {
    let mut f = Fixture::new();
    assert_eq!(f.scope().max_bytes(), 1_048_576);
    f.value["sync_scope"]["whole_folders"] =
        json!([folder("笔记"), rules_folder(&["txt"], 4_194_304)]);
    f.write();
    assert_eq!(f.scope().max_bytes(), 4_194_304);
    f.value["sync_scope"]["whole_folders"] = json!([folder("笔记")]);
    f.value["sync_scope"]["whole_files"] = json!([{
        "path":"附件/手册","markdown":false,"attachment_types":["pdf"],"max_bytes":2_097_152
    }]);
    f.write();
    assert_eq!(f.scope().max_bytes(), 2_097_152);
}
