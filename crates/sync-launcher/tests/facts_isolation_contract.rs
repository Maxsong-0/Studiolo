//! 七项隔离判定的契约：每一项都只来自 `systemctl show` 回读、垫片报告与 launcher 自己的 stat，
//! 不凭「已请求」填真；任一输入偏离就是 false。

use std::collections::BTreeMap;
use std::path::PathBuf;
use studiolo_sync_launcher::facts::{IsolationInputs, isolation};
use studiolo_sync_launcher::protocol::{PayloadReport, PayloadReportKind};

fn show(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn good_show() -> BTreeMap<String, String> {
    show(&[
        ("PrivateMounts", "yes"),
        ("ProtectSystem", "strict"),
        ("ProtectHome", "yes"),
        ("InaccessiblePaths", "/etc/studiolo /var/lib/studiolo-owner"),
        ("PrivatePIDs", "yes"),
        ("User", "studiolo-sync"),
        ("Group", "983"),
        ("SupplementaryGroups", ""),
        ("CapabilityBoundingSet", ""),
        ("ProtectControlGroups", "yes"),
        ("RestrictAddressFamilies", "AF_INET AF_INET6"),
    ])
}

fn report() -> PayloadReport {
    PayloadReport {
        kind: PayloadReportKind::PayloadReport,
        mirror_identity: (7, 1),
        config_identity: (7, 2),
        node_identity: (8, 3),
        cli_identity: (8, 4),
        uid: 999,
        gid: 983,
    }
}

fn judge(show: &BTreeMap<String, String>, report: &PayloadReport) -> [bool; 7] {
    let inaccessible = [
        PathBuf::from("/var/lib/studiolo-owner"),
        PathBuf::from("/etc/studiolo"),
    ];
    let frame = isolation(&IsolationInputs {
        show,
        report,
        own: [(7, 1), (7, 2), (8, 3), (8, 4)],
        payload_uid: 999,
        payload_gid: 983,
        payload_user: Some("studiolo-sync"),
        payload_group: Some("studiolo-sync"),
        inaccessible: &inaccessible,
        loop_quota: true,
    });
    [
        frame.mount_namespace,
        frame.pid_namespace,
        frame.dedicated_identity,
        frame.dropped_capabilities,
        frame.payload_cannot_write_cgroup,
        frame.final_view_identity_checked,
        frame.disk_quota_enforced,
    ]
}

#[test]
fn 七项隔离全部来自回读与报告() {
    assert_eq!(judge(&good_show(), &report()), [true; 7]);
    let mut show = good_show();
    show.insert("ProtectHome".into(), "read-only".into());
    assert!(!judge(&show, &report())[0], "ProtectHome 不是 yes");
    let mut show = good_show();
    show.insert("InaccessiblePaths".into(), "/etc/studiolo".into());
    assert!(!judge(&show, &report())[0], "不可达路径少了一条");
    let mut show = good_show();
    show.insert("PrivatePIDs".into(), "no".into());
    assert!(!judge(&show, &report())[1]);
    let mut show = good_show();
    show.insert("SupplementaryGroups".into(), "docker".into());
    assert!(!judge(&show, &report())[2], "有补充组");
    let mut show = good_show();
    show.insert("User".into(), "nobody".into());
    assert!(!judge(&show, &report())[2], "身份不是专用身份");
    let mut show = good_show();
    show.insert("User".into(), "999".into());
    assert!(judge(&show, &report())[2], "数字形式的回读也认");
    let mut report_uid = report();
    report_uid.uid = 1000;
    assert!(!judge(&good_show(), &report_uid)[2], "垫片实测 uid 不符");
    let mut show = good_show();
    show.insert("CapabilityBoundingSet".into(), "cap_chown".into());
    assert!(!judge(&show, &report())[3]);
    let mut show = good_show();
    show.insert(
        "RestrictAddressFamilies".into(),
        "AF_UNIX AF_INET AF_INET6".into(),
    );
    assert!(!judge(&show, &report())[4], "放开了 AF_UNIX");
    let mut show = good_show();
    show.insert("RestrictAddressFamilies".into(), "~".into());
    assert!(!judge(&show, &report())[4], "没设置");
    let mut report_cli = report();
    report_cli.cli_identity = (8, 5);
    assert!(
        !judge(&good_show(), &report_cli)[5],
        "最终视图里的 cli 不是同一对象"
    );
}
