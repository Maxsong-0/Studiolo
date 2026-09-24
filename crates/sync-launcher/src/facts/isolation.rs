//! 七项隔离判定：全部由 `systemctl show` 回读、垫片报告与 launcher 自己的 stat 推出，不凭「已请求」填真。

use super::value;
use crate::protocol::{IsolationFrame, PayloadReport};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// transient unit 必须回读为恰好这两族；含 `AF_UNIX` 就够得到 manager／launcher 的 socket。
const ADDRESS_FAMILIES: [&str; 2] = ["AF_INET", "AF_INET6"];

/// 七项隔离判定的全部输入。
pub struct IsolationInputs<'a> {
    /// `systemctl show` 回读。
    pub show: &'a BTreeMap<String, String>,
    /// 垫片在最终 mount 视图内的报告。
    pub report: &'a PayloadReport,
    /// launcher 自己 stat 的四个对象（镜像根、配置根、node、cli）。
    pub own: [(u64, u64); 4],
    /// 配置的专用 UID。
    pub payload_uid: u32,
    /// 配置的专用 GID。
    pub payload_gid: u32,
    /// `/etc/passwd` 解析出的用户名（`User=` 回读可能是名字而不是数字）。
    pub payload_user: Option<&'a str>,
    /// `/etc/group` 解析出的组名。
    pub payload_group: Option<&'a str>,
    /// 配置要求对 payload 不可达的路径。
    pub inaccessible: &'a [PathBuf],
    /// probe 同一判据：镜像根是否落在固定 loop 镜像上。
    pub loop_quota: bool,
}

/// 空白分隔的属性值当集合比较（顺序与重复都不算差异）。
fn words(text: &str) -> BTreeSet<&str> {
    text.split_whitespace().collect()
}

/// 七项隔离：任一项不成立就是 false；owner 据此拒绝放行。
#[must_use]
pub fn isolation(input: &IsolationInputs<'_>) -> IsolationFrame {
    let read = |key: &str| value(input.show, key);
    let expected: BTreeSet<&str> = input
        .inaccessible
        .iter()
        .filter_map(|path| path.to_str())
        .collect();
    let mount_namespace = read("PrivateMounts") == "yes"
        && read("ProtectSystem") == "strict"
        && read("ProtectHome") == "yes"
        && words(read("InaccessiblePaths")) == expected;
    let user_ok = read("User") == input.payload_uid.to_string()
        || input.payload_user.is_some_and(|name| name == read("User"));
    let group_ok = read("Group") == input.payload_gid.to_string()
        || input
            .payload_group
            .is_some_and(|name| name == read("Group"));
    let dedicated_identity = user_ok
        && group_ok
        && read("SupplementaryGroups").trim().is_empty()
        && input.report.uid == input.payload_uid
        && input.report.gid == input.payload_gid;
    let reported = [
        input.report.mirror_identity,
        input.report.config_identity,
        input.report.node_identity,
        input.report.cli_identity,
    ];
    IsolationFrame {
        mount_namespace,
        pid_namespace: read("PrivatePIDs") == "yes",
        dedicated_identity,
        dropped_capabilities: read("CapabilityBoundingSet").trim().is_empty(),
        payload_cannot_write_cgroup: read("ProtectControlGroups") == "yes"
            && words(read("RestrictAddressFamilies")) == BTreeSet::from(ADDRESS_FAMILIES),
        final_view_identity_checked: reported == input.own,
        disk_quota_enforced: input.loop_quota,
    }
}
