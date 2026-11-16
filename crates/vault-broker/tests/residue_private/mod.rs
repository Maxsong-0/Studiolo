//! 开库清扫：崩溃残留挪进隔离区、正式文件不变；别处、差一点的名字和不像 Broker 写下的节点原地不动；
//! 预算用完就停；封存导出跳过隔离区，开库之后才出现的残留照旧让导出扫描整批拒绝。
use super::*;
use crate::{
    BrokerError, Mutation, MutationActor, MutationKind, MutationMetadata, VaultPath, VaultReader,
    read_trusted_export,
};
use std::{
    fs,
    os::unix::fs::{PermissionsExt as _, symlink},
    path::{Path, PathBuf},
};

struct Fixture {
    temp: tempfile::TempDir,
    vault: PathBuf,
    data: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        Self {
            vault: temp.path().join("vault"),
            data: temp.path().join("data"),
            temp,
        }
    }
    fn open(&self) -> FsVaultBroker {
        FsVaultBroker::open(&self.vault, &self.data).unwrap()
    }
    fn reopen(&self) -> FsVaultBroker {
        FsVaultBroker::open_existing(&self.vault, &self.data).unwrap()
    }
    fn vault_quarantine(&self) -> PathBuf {
        self.vault.join(".workbench/residue-quarantine")
    }
    fn data_quarantine(&self) -> PathBuf {
        self.data.join("residue-quarantine")
    }
    /// 封存目标要求的私有空目录。
    fn private(&self, relative: &str) -> PathBuf {
        let path = self.temp.path().join(relative);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
}

fn plant(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// 隔离区里唯一的那个批次：一次开库只建一个。
fn only_batch(quarantine: &Path) -> PathBuf {
    let batches: Vec<PathBuf> = fs::read_dir(quarantine)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(batches.len(), 1, "隔离区里应该恰好一个批次：{batches:?}");
    batches.into_iter().next().unwrap()
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

fn far() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

#[test]
fn only_the_exact_atomic_write_shape_counts_as_residue() {
    for name in [
        ".a.studiolo-tmp-1-1",
        ".note.md.studiolo-tmp-4242-18446744073709551615",
        "..hidden.studiolo-tmp-1-2",
        ".a.studiolo-tmp-x.studiolo-tmp-1-2",
    ] {
        assert!(is_exact_temporary_name(name), "{name}");
    }
    for name in [
        "a.studiolo-tmp-1-1",
        "..studiolo-tmp-1-1",
        ".a.studiolo-tmp-1-",
        ".a.studiolo-tmp--1",
        ".a.studiolo-tmp-a-1",
        ".a.studiolo-tmp-1",
        ".a.studiolo-tmp-1-1-2",
        ".a.studiolo-tmp-1-1.bak",
        ".a.studiolo-tmp-1-1 ",
        ".a.studiolo-tmp-1-+1",
        ".a.studiolo-tmp-1-１",
        ".a.studiolo-tmp-123456789012345678901-1",
    ] {
        assert!(!is_exact_temporary_name(name), "{name}");
    }
}

#[test]
fn reopening_moves_crash_residue_into_the_quarantine_and_keeps_the_real_files() {
    let f = Fixture::new();
    let note = VaultPath::parse("note.md").unwrap();
    let mut broker = f.open();
    broker
        .mutate_recorded(
            Mutation::create(note.clone(), "正文".as_bytes().to_vec(), "first"),
            MutationMetadata::new(MutationActor::Obsidian, MutationKind::Replace).unwrap(),
        )
        .unwrap();
    drop(broker);
    // 没有残留时重开一次，作为对照。
    let broker = f.reopen();
    let status = broker.write_status().unwrap();
    drop(broker);
    let bindings = [
        f.vault.join(".workbench/broker-binding.json"),
        f.data.join("broker-binding.json"),
    ];
    let before: Vec<Vec<u8>> = bindings
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect();

    let history = format!("history/.{}.studiolo-tmp-4242-8", "ab".repeat(32));
    // (原位置, 在哪一处的隔离区, 批次里的相对路径)；Vault 侧从 `.workbench/` 算起。
    let residues = [
        (
            f.data.join(".writer-state.json.studiolo-tmp-4242-7"),
            f.data_quarantine(),
            ".writer-state.json.studiolo-tmp-4242-7".to_owned(),
        ),
        (f.data.join(&history), f.data_quarantine(), history.clone()),
        (
            f.vault
                .join(".workbench/.broker-binding.json.studiolo-tmp-4242-9"),
            f.vault_quarantine(),
            ".broker-binding.json.studiolo-tmp-4242-9".to_owned(),
        ),
        (
            f.vault
                .join(".workbench/gateway/task/attempt/call/.manifest.json.studiolo-tmp-4242-10"),
            f.vault_quarantine(),
            "gateway/task/attempt/call/.manifest.json.studiolo-tmp-4242-10".to_owned(),
        ),
    ];
    for (index, (original, _, _)) in residues.iter().enumerate() {
        plant(original, format!("写到一半 {index}").as_bytes());
    }

    let broker = f.reopen();
    assert_eq!(broker.write_status().unwrap(), status, "清扫不改写者状态");
    assert_eq!(broker.read(&note).unwrap().content, "正文".as_bytes());
    for (path, bytes) in bindings.iter().zip(&before) {
        assert_eq!(&fs::read(path).unwrap(), bytes, "正式文件不变：{path:?}");
    }
    let vault_batch = only_batch(&f.vault_quarantine());
    let data_batch = only_batch(&f.data_quarantine());
    for (index, (original, quarantine, relative)) in residues.iter().enumerate() {
        assert!(!original.exists(), "残留应该挪走：{original:?}");
        let batch = if *quarantine == f.vault_quarantine() {
            &vault_batch
        } else {
            &data_batch
        };
        assert_eq!(
            fs::read(batch.join(relative)).unwrap(),
            format!("写到一半 {index}").as_bytes(),
            "隔离区里原样保留：{relative}"
        );
    }
    for directory in [
        f.vault_quarantine(),
        vault_batch.clone(),
        f.data_quarantine(),
        data_batch.clone(),
    ] {
        assert_eq!(mode(&directory), 0o700, "{directory:?}");
    }
    drop(broker);

    // 再开一次：没有新残留就不建新批次，隔离区里的东西原样留着，不会被当成残留再挪。
    let broker = f.reopen();
    assert_eq!(only_batch(&f.vault_quarantine()), vault_batch);
    assert_eq!(only_batch(&f.data_quarantine()), data_batch);
    assert!(data_batch.join(&history).exists());
    assert_eq!(broker.write_status().unwrap(), status);
}

#[test]
fn names_outside_broker_places_near_misses_and_odd_nodes_stay_put() {
    let f = Fixture::new();
    drop(f.open());
    let workbench = f.vault.join(".workbench");
    let stays = [
        // 用户笔记目录和 Vault 根顶层：Broker 不在那里起临时文件。
        f.vault.join("notes/.plain.md.studiolo-tmp-7-3"),
        f.vault.join(".plain.md.studiolo-tmp-7-4"),
        // 差一点的名字。
        workbench.join(".x.studiolo-tmp-1-"),
        workbench.join(".x.studiolo-tmp--1"),
        workbench.join(".x.studiolo-tmp-a-1"),
        workbench.join("x.studiolo-tmp-1-1"),
        workbench.join("..studiolo-tmp-1-1"),
        workbench.join(".x.studiolo-tmp-1-1-2"),
        workbench.join(".x.studiolo-tmp-1-1.bak"),
        f.data.join(".y.studiolo-tmp-1-x"),
        f.data.join(".y.studiolo-tmp-1"),
    ];
    for (index, path) in stays.iter().enumerate() {
        plant(path, format!("留着 {index}").as_bytes());
    }
    // 名字对得上，但不是原子写起的那种节点：目录、链接、有两个名字的文件。
    let directory = workbench.join(".dir.studiolo-tmp-1-1");
    fs::create_dir(&directory).unwrap();
    let link = workbench.join(".link.studiolo-tmp-1-2");
    symlink("/outside", &link).unwrap();
    let linked = workbench.join(".hard.studiolo-tmp-1-3");
    plant(&linked, b"two names");
    fs::hard_link(&linked, workbench.join("hard-twin")).unwrap();

    drop(f.reopen());
    for (index, path) in stays.iter().enumerate() {
        assert_eq!(
            fs::read(path).unwrap(),
            format!("留着 {index}").as_bytes(),
            "{path:?}"
        );
    }
    assert!(directory.is_dir());
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&linked).unwrap(), b"two names");
    assert!(
        !f.vault_quarantine().exists() && !f.data_quarantine().exists(),
        "没有真残留就不建隔离区"
    );
}

#[test]
fn an_exhausted_budget_stops_and_leaves_the_rest_for_the_next_open() {
    let f = Fixture::new();
    let broker = f.open();
    let residue = f
        .data
        .join(format!("history/.{}.studiolo-tmp-9-1", "cd".repeat(32)));
    plant(&residue, b"half");
    // 时间用完、节点用完：一个都不挪，也不建隔离区。
    sweep_within(&broker, Instant::now(), u64::MAX);
    sweep_within(&broker, far(), 0);
    sweep_within(&broker, far(), 1);
    assert!(residue.exists());
    assert!(!f.data_quarantine().exists());

    // 比深度上限还深的不进去，浅处的照常挪。
    let mut deep = f.data.join("deep");
    for _ in 0..=MAX_DEPTH {
        deep.push("d");
    }
    let buried = deep.join(".x.studiolo-tmp-9-2");
    plant(&buried, b"too deep");
    sweep_within(&broker, far(), u64::MAX);
    assert!(!residue.exists());
    assert!(buried.exists());
    let batch = only_batch(&f.data_quarantine());
    assert_eq!(
        fs::read(batch.join("history").join(residue.file_name().unwrap())).unwrap(),
        b"half"
    );
}

/// 冻结、核对、导出到 `<名字>/canonical` 和 `<名字>/writer-data`，再交给可信导出扫描。
fn export(
    f: &Fixture,
    broker: FsVaultBroker,
    name: &str,
) -> (FsVaultBroker, PathBuf, Result<usize, BrokerError>) {
    let root = f.private(name);
    let vault = f.private(&format!("{name}/canonical"));
    let data = f.private(&format!("{name}/writer-data"));
    let uuid = broker.vault_uuid();
    let mut frozen = broker.freeze_checkpoint(far());
    frozen.verify_audit(|_, _, _| Ok(())).unwrap();
    frozen.export(&vault, &data).unwrap();
    let facts = read_trusted_export(&root, uuid).map(|facts| facts.len());
    (frozen.thaw(), root, facts)
}

#[test]
fn checkpoint_export_skips_the_quarantine_and_still_refuses_fresh_residue() {
    let f = Fixture::new();
    drop(f.open());
    plant(
        &f.data.join(".writer-state.json.studiolo-tmp-4242-1"),
        b"half",
    );
    plant(
        &f.vault.join(".workbench/.vault.json.studiolo-tmp-4242-2"),
        b"half",
    );
    let broker = f.reopen();
    assert!(f.vault_quarantine().is_dir() && f.data_quarantine().is_dir());

    let (broker, root, facts) = export(&f, broker, "export");
    assert!(facts.unwrap() > 0, "隔离区不进导出，导出扫描不再整批拒绝");
    assert!(
        !root
            .join("canonical/.workbench/residue-quarantine")
            .exists()
    );
    assert!(!root.join("writer-data/residue-quarantine").exists());
    assert!(root.join("canonical/.workbench/vault.json").is_file());
    assert!(root.join("writer-data/writer-state.json").is_file());

    // 开库之后才出现的残留不会被挪，导出照旧带上它，扫描照旧整批拒绝（失败关闭）。
    plant(
        &f.data.join(".writer-state.json.studiolo-tmp-4242-3"),
        b"half",
    );
    let (broker, root, facts) = export(&f, broker, "later");
    assert!(
        root.join("writer-data/.writer-state.json.studiolo-tmp-4242-3")
            .is_file()
    );
    assert!(matches!(facts, Err(BrokerError::InvalidArtifact(_))));
    drop(broker);
}

/// 隔离区目录已经在、却不是本用户私有的（组或其他人有权限）：不往里挪，残留原地不动、照旧挡封存，等人看过
/// （复审 A3-F6）；收成本用户私有之后照常复用它。
#[test]
fn an_existing_quarantine_is_reused_only_when_it_is_private() {
    let f = Fixture::new();
    drop(f.open());
    let name = ".broker-binding.json.studiolo-tmp-4242-11";
    let residue = f.vault.join(".workbench").join(name);
    fs::create_dir(f.vault_quarantine()).unwrap();
    for loose in [0o750, 0o705, 0o777] {
        fs::set_permissions(f.vault_quarantine(), fs::Permissions::from_mode(loose)).unwrap();
        plant(&residue, b"source text");
        drop(f.reopen());
        assert_eq!(
            fs::read(&residue).unwrap(),
            b"source text",
            "{loose:o} 的隔离区不往里挪"
        );
        assert_eq!(fs::read_dir(f.vault_quarantine()).unwrap().count(), 0);
    }

    fs::set_permissions(f.vault_quarantine(), fs::Permissions::from_mode(0o700)).unwrap();
    drop(f.reopen());
    assert!(!residue.exists(), "私有的隔离区照常复用");
    let batch = only_batch(&f.vault_quarantine());
    assert_eq!(fs::read(batch.join(name)).unwrap(), b"source text");
    assert_eq!(mode(&batch), 0o700);
}
