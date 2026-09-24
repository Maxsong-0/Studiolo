//! 纯事实解析的契约：unit 名、systemd 输出、mountinfo／passwd、slice 与前缀、操作名、限额。
//! 七项隔离判定在 `facts_isolation_contract.rs`。

use std::collections::BTreeMap;
use std::path::Path;
use studiolo_sync_launcher::facts::{
    self, dedicated_identity, group_name, hex16, mount_device, operation, parse_hex16, parse_show,
    passwd_name, property_path_acceptable, reap, slice_cgroup_path, slice_from_prefix,
    systemd_version, unit_name, within_prefix,
};
use studiolo_sync_launcher::protocol::{LaunchFrame, LimitsFrame, Verdict};
use studiolo_sync_transport::owned_run::{RunBudget, RunId, RunOperation, SyncMode};

const MOUNTINFO: &str = "48 1 252:0 / / rw,relatime shared:1 - ext4 /dev/mapper/x rw\n\
120 48 7:3 / /var/lib/studiolo-sync rw,relatime shared:2 - ext4 /dev/loop3 rw\n\
121 48 7:4 / /mnt/with\\040space rw - ext4 /dev/loop4 rw\n\
122 48 7:5 / /var/lib/studiolo-sync rw - ext4 /dev/loop5 rw\n";
const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\n\
studiolo-sync:x:999:983:Studiolo:/var/lib/studiolo-sync:/usr/sbin/nologin\n\
maxsong:x:1000:1000:maxsong:/home/maxsong:/bin/bash\n";
const GROUP: &str = "root:x:0:\nsudo:x:27:maxsong\nstudiolo-sync:x:983:\nmaxsong:x:1000:\n";

fn show(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn frame(operation: &str, mode: Option<SyncMode>) -> LaunchFrame {
    LaunchFrame {
        run_id: RunId::from_bytes([0xab; 16]),
        sequence: 7,
        operation: operation.to_owned(),
        mode,
        nonce: [1u8; 32],
        argv_digest: [2u8; 32],
        budget: RunBudget {
            deadline_seconds: 1,
            stop_grace_seconds: 1,
            stdout_bytes: 1,
            stderr_bytes: 1,
            config_json_bytes: 1,
            private_state_bytes: 1,
            log_bytes: 1,
            memory_bytes: 1,
            pids_max: 1,
            cpu_max_percent: 1,
        },
        payload_uid: 999,
        payload_gid: 983,
    }
}

#[test]
fn unit名与十六进制() {
    assert_eq!(
        unit_name(7, &RunId::from_bytes([0xab; 16])),
        "studiolo-sync-payload-7-abababab.service"
    );
    let bytes = [
        0x1e, 0xfd, 0x69, 0x7b, 0xb2, 0xc1, 0x47, 0xab, 0x9c, 0x79, 0x19, 0xc8, 0xf9, 0x5a, 0x24,
        0x93,
    ];
    assert_eq!(
        parse_hex16("1efd697b-b2c1-47ab-9c79-19c8f95a2493\n"),
        Some(bytes)
    );
    assert_eq!(parse_hex16(&hex16(&bytes)), Some(bytes));
    assert_eq!(parse_hex16("1efd697b"), None);
    assert_eq!(parse_hex16("zz".repeat(16).as_str()), None);
}

#[test]
fn systemd版本行() {
    assert_eq!(
        systemd_version("systemd 259 (259.5-0ubuntu3.4)\n+PAM +AUDIT\n"),
        Some(259)
    );
    assert_eq!(systemd_version("systemctl 259"), None);
    assert_eq!(systemd_version(""), None);
}

#[test]
fn mountinfo取最上层挂载并解转义() {
    assert_eq!(
        mount_device(MOUNTINFO, Path::new("/var/lib/studiolo-sync")),
        Some("7:5".to_owned()),
        "同一挂载点两条记录取最后一条（最上层）"
    );
    assert_eq!(
        mount_device(MOUNTINFO, Path::new("/mnt/with space")),
        Some("7:4".to_owned())
    );
    assert_eq!(
        mount_device(MOUNTINFO, Path::new("/")),
        Some("252:0".to_owned())
    );
    assert_eq!(mount_device(MOUNTINFO, Path::new("/nope")), None);
}

#[test]
fn 专用身份要求主组相符且无补充组() {
    assert_eq!(
        passwd_name(PASSWD, 999, 983),
        Some("studiolo-sync".to_owned())
    );
    assert_eq!(passwd_name(PASSWD, 999, 1000), None, "主组不符");
    assert_eq!(group_name(GROUP, 983), Some("studiolo-sync".to_owned()));
    assert!(dedicated_identity(PASSWD, GROUP, 999, 983));
    assert!(
        !dedicated_identity(PASSWD, GROUP, 1000, 1000),
        "maxsong 在 sudo 组里"
    );
    assert!(!dedicated_identity(PASSWD, GROUP, 998, 983), "用户不存在");
    let extra = format!("{GROUP}docker:x:107:studiolo-sync\n");
    assert!(!dedicated_identity(PASSWD, &extra, 999, 983), "多了补充组");
    assert!(
        !dedicated_identity(PASSWD, "root:x:0:\n", 999, 983),
        "组不存在"
    );
}

#[test]
fn slice与前缀() {
    assert_eq!(
        slice_cgroup_path("system.slice"),
        Some("system.slice".to_owned())
    );
    assert_eq!(
        slice_cgroup_path("studiolo-sync.slice"),
        Some("studiolo.slice/studiolo-sync.slice".to_owned()),
        "连字符表示层级"
    );
    assert_eq!(
        slice_cgroup_path("studiolo-launcher-test.slice"),
        Some("studiolo.slice/studiolo-launcher.slice/studiolo-launcher-test.slice".to_owned())
    );
    for bad in ["payload", ".slice", "-a.slice", "a-.slice", "a--b.slice"] {
        assert_eq!(slice_cgroup_path(bad), None, "{bad}");
    }
    assert_eq!(
        slice_from_prefix(Path::new(
            "/sys/fs/cgroup/studiolo.slice/studiolo-sync.slice"
        )),
        Some("studiolo-sync.slice".to_owned())
    );
    assert_eq!(
        slice_from_prefix(Path::new(
            "/sys/fs/cgroup/user.slice/user-1000.slice/user@1000.service/studiolo.slice/studiolo-sync.slice"
        )),
        Some("studiolo-sync.slice".to_owned()),
        "用户 manager 的根以 .service 结尾"
    );
    assert_eq!(
        slice_from_prefix(Path::new("/sys/fs/cgroup/studiolo-sync.slice")),
        None,
        "少了 studiolo.slice 这一层"
    );
    assert_eq!(
        slice_from_prefix(Path::new("/sys/fs/cgroup/a.slice/b.slice")),
        None,
        "b.slice 不住在 a.slice 下"
    );
    assert_eq!(slice_from_prefix(Path::new("/sys/fs/cgroup/payload")), None);
    let prefix = Path::new("/sys/fs/cgroup/studiolo.slice/studiolo-sync.slice");
    assert!(within_prefix(
        "/studiolo.slice/studiolo-sync.slice/studiolo-sync-payload-7-abababab.service",
        prefix
    ));
    assert!(!within_prefix("/system.slice/x.service", prefix));
    assert!(
        !within_prefix("/studiolo.slice/studiolo-sync.slice", prefix),
        "前缀本身不算在内"
    );
    assert!(!within_prefix(
        "/studiolo.slice/studiolo-sync.slice/../x",
        prefix
    ));
    assert!(property_path_acceptable(Path::new(
        "/var/lib/studiolo-sync/mirror"
    )));
    assert!(!property_path_acceptable(Path::new("/var/lib/with space")));
}

#[test]
fn 操作名闭集() {
    assert_eq!(
        operation(&frame("inspect_config", None)),
        Ok(RunOperation::InspectConfig)
    );
    assert_eq!(
        operation(&frame("run_once", None)),
        Ok(RunOperation::RunOnce)
    );
    assert_eq!(
        operation(&frame("set_mode", Some(SyncMode::Bidirectional))),
        Ok(RunOperation::SetMode(SyncMode::Bidirectional))
    );
    assert_eq!(operation(&frame("set_mode", None)), Err(Verdict::Protocol));
    assert_eq!(
        operation(&frame("run_once", Some(SyncMode::MirrorRemote))),
        Err(Verdict::Protocol)
    );
    assert_eq!(operation(&frame("sync", None)), Err(Verdict::Protocol));
}

#[test]
fn show解析与等待结果() {
    let parsed =
        parse_show("ActiveState=failed\nExecMainCode=1\nExecMainStatus=7\nBroken\nControlGroup=\n");
    assert_eq!(facts::value(&parsed, "ActiveState"), "failed");
    assert_eq!(facts::value(&parsed, "ControlGroup"), "");
    assert_eq!(facts::value(&parsed, "Missing"), "");
    assert_eq!(facts::number::<i32>(&parsed, "ExecMainStatus"), Some(7));
    assert_eq!(reap(&parsed), (true, Some(7), None));
    assert_eq!(
        reap(&show(&[("ExecMainCode", "2"), ("ExecMainStatus", "15")])),
        (true, None, Some(15))
    );
    assert_eq!(
        reap(&show(&[("ExecMainCode", "3"), ("ExecMainStatus", "6")])),
        (true, None, Some(6))
    );
    assert_eq!(
        reap(&show(&[("ExecMainCode", "0"), ("ExecMainStatus", "0")])),
        (false, None, None)
    );
    assert_eq!(
        reap(&show(&[("Result", "success")])),
        (false, None, None),
        "没有 ExecMainCode 就不算等到"
    );
}

#[test]
fn 限额解析() {
    assert_eq!(
        facts::limits("268435456\n", "0\n", "64\n", "150000 100000\n"),
        Ok(LimitsFrame {
            memory_max: Some(268_435_456),
            memory_swap_max: Some(0),
            pids_max: Some(64),
            cpu_max: Some((150_000, 100_000)),
        })
    );
    assert_eq!(
        facts::limits("max\n", "max\n", "max\n", "max 100000\n"),
        Ok(LimitsFrame::default())
    );
    assert_eq!(
        facts::limits("lots\n", "0\n", "64\n", "max 100000\n"),
        Err(Verdict::Protocol)
    );
}
