//! 上线前只读核查脚本（`scripts/check-backup-preflight.py`）对着真实 Broker 的数据跑：
//! 脚本说「会挡封存」的，真封存（冻结、核对、导出、可信扫描）一定拒绝；脚本说干净的，真封存一定通过。
use super::{artifact_request, broker};
use std::{
    fs,
    os::unix::fs::{PermissionsExt as _, symlink},
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
use studiolo_core::{AttemptId, TaskId};
use studiolo_vault_broker::{
    ArtifactStager, FsVaultBroker, VaultReader, VaultWriter, read_trusted_export,
};

const SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scripts/check-backup-preflight.py"
);

/// 跑一次脚本：退出码，和发现的类别（去重排序）。
fn preflight(vault: &Path, data: &Path) -> (i32, Vec<String>) {
    let output = Command::new("python3")
        .args(["-I", "-B", SCRIPT, "--json", "--vault"])
        .arg(vault)
        .arg("--data")
        .arg(data)
        .output()
        .expect("python3 可用");
    let code = output.status.code().expect("脚本正常退出");
    assert!(
        code == 0 || code == 1,
        "脚本没做完：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut kinds: Vec<String> = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| finding["kind"].as_str().unwrap().to_owned())
        .collect();
    kinds.sort();
    kinds.dedup();
    (code, kinds)
}

/// 真封存一次（导出到新的私有空目录再做可信扫描），返回还回来的 Broker 和是否通过。
fn sealing(broker: FsVaultBroker) -> (FsVaultBroker, bool) {
    let target = tempfile::tempdir().unwrap();
    for name in ["canonical", "writer-data"] {
        let path = target.path().join(name);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let uuid = broker.vault_uuid();
    let mut frozen = broker.freeze_checkpoint(Instant::now() + Duration::from_secs(60));
    let passed = frozen.verify_audit(|_, _, _| Ok(())).is_ok()
        && frozen
            .export(
                &target.path().join("canonical"),
                &target.path().join("writer-data"),
            )
            .is_ok()
        && read_trusted_export(target.path(), uuid).is_ok();
    (frozen.thaw(), passed)
}

fn plant(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
fn script_and_real_sealing_agree_on_current_data_and_every_planted_legacy_shape() {
    let (vault, data, mut broker) = broker();
    let request = artifact_request();
    let receipt = broker.commit_attempt_artifacts(request.clone()).unwrap();
    let reference = broker.stage_artifact_commit(&request).unwrap();
    plant(
        &vault.path().join("notes/plain.md"),
        "用户自己的笔记".as_bytes(),
    );
    let (v, d) = (vault.path(), data.path());

    // 当前版本写出来的清单、暂存和用户文件：两边都通过。
    assert_eq!(preflight(v, d), (0, vec![]));
    let (returned, passed) = sealing(broker);
    broker = returned;
    assert!(passed, "当前版本的数据真封存应该通过");

    let manifest = v.join(receipt.manifest_path.as_str());
    let attempt = manifest.parent().unwrap().to_path_buf();
    let original = fs::read(&manifest).unwrap();
    let mut legacy_manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
    legacy_manifest["format_version"] = 1.into();
    legacy_manifest
        .as_object_mut()
        .unwrap()
        .remove("provenance");
    let staged = d.join(format!("artifact-candidate-{}.json", reference.id));
    let mut legacy_staged: serde_json::Value =
        serde_json::from_slice(&fs::read(&staged).unwrap()).unwrap();
    legacy_staged.as_object_mut().unwrap().remove("provenance");
    let legacy_staged_path = d.join(format!("artifact-candidate-{}.json", "e".repeat(64)));
    let stray_attempt = v.join(format!(
        "sources/attempts/{}/{}",
        TaskId::new(),
        AttemptId::new()
    ));

    type Step<'a> = Box<dyn Fn() + 'a>;
    let cases: Vec<(&str, &str, Step, Step)> = vec![
        (
            "第 1 版清单、没有原批准",
            "old_manifest",
            Box::new(|| {
                fs::write(&manifest, serde_json::to_vec(&legacy_manifest).unwrap()).unwrap()
            }),
            Box::new(|| fs::write(&manifest, &original).unwrap()),
        ),
        (
            "尝试目录里清单没列的文件",
            "unlisted_attempt_file",
            Box::new(|| plant(&attempt.join("extra.md"), b"extra")),
            Box::new(|| fs::remove_file(attempt.join("extra.md")).unwrap()),
        ),
        (
            "没有清单的尝试目录",
            "missing_manifest",
            Box::new(|| plant(&stray_attempt.join("result.md"), b"old")),
            Box::new(|| fs::remove_dir_all(stray_attempt.parent().unwrap()).unwrap()),
        ),
        (
            "升级前留下的暂存正文",
            "staged_legacy",
            Box::new(|| {
                plant(
                    &legacy_staged_path,
                    &serde_json::to_vec(&legacy_staged).unwrap(),
                )
            }),
            Box::new(|| fs::remove_file(&legacy_staged_path).unwrap()),
        ),
        (
            "用户目录里的原子写残留",
            "residue",
            Box::new(|| plant(&v.join("notes/.plain.md.studiolo-tmp-1-1"), b"half")),
            Box::new(|| fs::remove_file(v.join("notes/.plain.md.studiolo-tmp-1-1")).unwrap()),
        ),
        (
            "符号链接",
            "special_node",
            Box::new(|| symlink("plain.md", v.join("notes/link.md")).unwrap()),
            Box::new(|| fs::remove_file(v.join("notes/link.md")).unwrap()),
        ),
        (
            "硬链接",
            "hard_link",
            Box::new(|| fs::hard_link(v.join("notes/plain.md"), v.join("notes/twin.md")).unwrap()),
            Box::new(|| fs::remove_file(v.join("notes/twin.md")).unwrap()),
        ),
        (
            "导出扫描认不了的路径",
            "bad_name",
            Box::new(|| plant(&v.join("notes/a%2e.md"), b"name")),
            Box::new(|| fs::remove_file(v.join("notes/a%2e.md")).unwrap()),
        ),
    ];
    for (label, kind, add, remove) in &cases {
        add();
        assert_eq!(preflight(v, d), (1, vec![(*kind).to_owned()]), "{label}");
        let (returned, passed) = sealing(broker);
        broker = returned;
        assert!(!passed, "{label}：脚本说会挡，真封存却通过了");
        remove();
    }
    assert_eq!(preflight(v, d), (0, vec![]));
    let (returned, passed) = sealing(broker);
    assert!(passed, "旧数据都拿走以后真封存应该恢复通过");

    // 工作区里形状对得上的残留：脚本说会挡、重开会清；真重开一次以后两边都恢复。
    plant(&v.join(".workbench/.vault.json.studiolo-tmp-7-7"), b"half");
    assert_eq!(preflight(v, d), (1, vec!["residue".to_owned()]));
    let (returned, passed) = sealing(returned);
    assert!(!passed);
    drop(returned);
    let reopened = FsVaultBroker::open_existing(v, d).unwrap();
    assert_eq!(preflight(v, d), (0, vec![]));
    assert!(sealing(reopened).1, "清扫挪走以后真封存应该通过");
}
