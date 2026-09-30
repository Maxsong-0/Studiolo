//! 对外部已验完整性、独占私有目录中的恢复副本执行重绑定与打开验收。
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    process::ExitCode,
};
use studiolo_core::VaultUuid;
use studiolo_vault_broker::{
    FsVaultBroker, MaintenanceCoverage, NewRootRebind, PhysicalId, rebind_new_roots,
};
type Result<T> = std::result::Result<T, &'static str>;
const HELP: &str =
    "--vault <abs> --writer-data <abs> --vault-uuid <uuid> --pair-id <uuid> --checkpoint <id>";
const KEYS: [&str; 5] = [
    "--vault",
    "--writer-data",
    "--vault-uuid",
    "--pair-id",
    "--checkpoint",
];
#[derive(Clone)]
struct Options {
    roots: [PathBuf; 2],
    ids: [VaultUuid; 2],
    checkpoint: String,
}
fn absolute(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("E_PATH");
    }
    Ok(path.components().collect())
}
fn parse(args: Vec<OsString>) -> Result<Option<Options>> {
    if args == [OsString::from("--help")] {
        return Ok(None);
    }
    if args.len() != 10 {
        return Err("E_ARGUMENTS");
    }
    let mut fields = BTreeMap::new();
    for pair in args.chunks_exact(2) {
        let key = pair[0].to_str().ok_or("E_ARGUMENTS")?;
        if !KEYS.contains(&key) || fields.insert(key, &pair[1]).is_some() {
            return Err("E_ARGUMENTS");
        }
    }
    let text = |key: &str| fields[key].to_str().ok_or("E_ARGUMENTS");
    let checkpoint = text(KEYS[4])?.to_owned();
    if checkpoint.trim().is_empty()
        || checkpoint.len() > 128
        || checkpoint.chars().any(char::is_control)
    {
        return Err("E_ARGUMENTS");
    }
    Ok(Some(Options {
        roots: [
            absolute(Path::new(fields[KEYS[0]]))?,
            absolute(Path::new(fields[KEYS[1]]))?,
        ],
        ids: [
            text(KEYS[2])?.parse().map_err(|_| "E_ARGUMENTS")?,
            text(KEYS[3])?.parse().map_err(|_| "E_ARGUMENTS")?,
        ],
        checkpoint,
    }))
}
fn directory(path: &Path, private: bool) -> Result<PhysicalId> {
    let mut current = PathBuf::new();
    for component in absolute(path)?.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).map_err(|_| "E_PATH")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("E_PATH");
        }
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| "E_PATH")?;
    if private
        && (metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o7777 != 0o700)
    {
        return Err("E_PRIVATE_ROOT");
    }
    Ok(PhysicalId {
        dev: metadata.dev(),
        ino: metadata.ino(),
    })
}
/// 这里只取防原根误用所需字段；完整封存格式仍由 Broker 核验。
#[derive(Deserialize)]
struct Sealed {
    format_version: u32,
    vault_uuid: VaultUuid,
    pair_id: VaultUuid,
    canonical: PhysicalId,
    data: PhysicalId,
}
fn binding(path: &Path) -> Result<Vec<u8>> {
    use rustix::fs::{Mode, OFlags, open};
    directory(path.parent().ok_or("E_BINDING")?, false)?;
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    let file = fs::File::from(open(path, flags, Mode::empty()).map_err(|_| "E_BINDING")?);
    let metadata = file.metadata().map_err(|_| "E_BINDING")?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() > 65536 {
        return Err("E_BINDING");
    }
    let mut bytes = Vec::new();
    file.take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "E_BINDING")?;
    if bytes.len() > 65536 {
        return Err("E_BINDING");
    }
    Ok(bytes)
}
fn probe(options: &Options) -> Result<Value> {
    let [vault, writer] = &options.roots;
    if vault.starts_with(writer) || writer.starts_with(vault) {
        return Err("E_ROOT_OVERLAP");
    }
    let roots = [directory(vault, true)?, directory(writer, true)?];
    let left = binding(&vault.join(".workbench/broker-binding.json"))?;
    if left != binding(&writer.join("broker-binding.json"))? {
        return Err("E_BINDING");
    }
    let sealed: Sealed = serde_json::from_slice(&left).map_err(|_| "E_BINDING")?;
    if sealed.format_version != 1 || [sealed.vault_uuid, sealed.pair_id] != options.ids {
        return Err("E_IDENTITY");
    }
    let originals = [sealed.canonical, sealed.data];
    if roots[0] == roots[1] || roots.iter().any(|id| originals.contains(id)) {
        return Err("E_ORIGINAL_ROOT");
    }
    if roots != [directory(vault, true)?, directory(writer, true)?] {
        return Err("E_ROOT_CHANGED");
    }
    // 不初始化、不手改 JSON、不接手失败的恢复；API 返回后内部租约已释放。
    let receipt = rebind_new_roots(&NewRootRebind {
        new_vault_root: vault,
        new_data_root: writer,
        expected_vault_uuid: options.ids[0],
        expected_pair_id: options.ids[1],
        checkpoint_id: options.checkpoint.clone(),
        parent_lineage: None,
        resume_interrupted: false,
    })
    .map_err(|_| "E_REBIND")?;
    if !receipt.coverage.covers_required_bindings() || !receipt.lineage.is_completed() {
        return Err("E_COVERAGE");
    }
    let broker = FsVaultBroker::open_existing(vault, writer).map_err(|_| "E_OPEN")?;
    let status = broker.write_status().map_err(|_| "E_STATUS")?;
    if status.unresolved || status.read_blocked {
        return Err("E_PENDING");
    }
    Ok(json!({"status":"ok", "snapshot_id":options.checkpoint,
        "vault_uuid":receipt.vault_uuid, "pair_id":receipt.pair_id,
        "committed_through":status.committed_through, "unresolved":false,
        "coverage":{"canonical_binding":receipt.coverage.canonical_binding,
            "data_binding":receipt.coverage.data_binding, "history_meta_v1":receipt.coverage.history_meta_v1,
            "maintenance_v1":match receipt.coverage.maintenance_v1 {
                MaintenanceCoverage::Rebound => "rebound", MaintenanceCoverage::NotEnabled => "not_enabled"}}}))
}
fn main() -> ExitCode {
    let result = parse(std::env::args_os().skip(1).collect())
        .and_then(|options| match options {
            Some(options) => probe(&options),
            None => Ok(json!({"status":"help", "arguments":HELP})),
        })
        .and_then(|value| {
            let mut output = std::io::stdout().lock();
            serde_json::to_writer(&mut output, &value).map_err(|_| "E_OUTPUT")?;
            writeln!(output).map_err(|_| "E_OUTPUT")
        });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(stage) => {
            eprintln!("{stage}");
            ExitCode::FAILURE
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use studiolo_core::VaultPath;
    use studiolo_vault_broker::{Mutation, VaultWriter};
    fn copy(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        fs::set_permissions(to, fs::Permissions::from_mode(0o700)).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    fn fixture() -> (tempfile::TempDir, Options, Options) {
        let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let roots = ["old-vault", "old-writer"].map(|s| temp.path().join(s));
        for root in &roots {
            fs::create_dir(root).unwrap();
            fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut broker = FsVaultBroker::initialize_empty(&roots[0], &roots[1]).unwrap();
        let note = VaultPath::parse("note.md").unwrap();
        let mutation = Mutation::create(note, b"fixture".to_vec(), "fixture-key");
        broker.mutate(mutation).unwrap();
        drop(broker);
        let bytes = binding(&roots[1].join("broker-binding.json")).unwrap();
        let sealed: Sealed = serde_json::from_slice(&bytes).unwrap();
        let old = Options {
            roots,
            ids: [sealed.vault_uuid, sealed.pair_id],
            checkpoint: "cp-fixture".into(),
        };
        let new = Options {
            roots: ["new-vault", "new-writer"].map(|s| temp.path().join(s)),
            ..old.clone()
        };
        for (from, to) in old.roots.iter().zip(&new.roots) {
            copy(from, to);
        }
        (temp, old, new)
    }
    #[test]
    fn original_or_mixed_roots_do_not_change_bindings() {
        let (_temp, old, new) = fixture();
        let paths = [
            old.roots[0].join(".workbench/broker-binding.json"),
            old.roots[1].join("broker-binding.json"),
        ];
        let before = paths.each_ref().map(|p| fs::read(p).unwrap());
        assert_eq!(probe(&old).unwrap_err(), "E_ORIGINAL_ROOT");
        for index in 0..2 {
            let mut mixed = new.clone();
            mixed.roots[index] = old.roots[index].clone();
            assert_eq!(probe(&mixed).unwrap_err(), "E_ORIGINAL_ROOT");
        }
        assert_eq!(before, paths.each_ref().map(|p| fs::read(p).unwrap()));
    }
    #[test]
    fn fresh_copy_rebinds_and_opens_with_original_watermark() {
        let (_temp, _old, new) = fixture();
        assert!(FsVaultBroker::open_existing(&new.roots[0], &new.roots[1]).is_err());
        assert_eq!(probe(&new).unwrap()["committed_through"], 1);
        let broker = FsVaultBroker::open_existing(&new.roots[0], &new.roots[1]).unwrap();
        assert!(!broker.write_status().unwrap().unresolved);
    }
    #[test]
    fn wrong_ids_and_symlink_ancestors_fail_before_rebinding() {
        let (temp, _old, new) = fixture();
        for index in 0..2 {
            let mut wrong = new.clone();
            wrong.ids[index] = VaultUuid::new();
            assert_eq!(probe(&wrong).unwrap_err(), "E_IDENTITY");
        }
        let alias = temp.path().join("alias");
        symlink(temp.path(), &alias).unwrap();
        let mut linked = new.clone();
        linked.roots[0] = alias.join("new-vault");
        assert_eq!(probe(&linked).unwrap_err(), "E_PATH");
        assert!(probe(&new).is_ok());
    }
    #[test]
    fn arguments_are_closed_and_absolute() {
        let id = "00000000-0000-0000-0000-000000000001";
        let values = ["/vault", "/writer", id, id, "cp-test"];
        let base: Vec<_> = KEYS
            .into_iter()
            .zip(values)
            .flat_map(|(key, value)| [OsString::from(key), OsString::from(value)])
            .collect();
        assert!(parse(base.clone()).unwrap().is_some());
        for (index, bad) in [
            (0, "--unknown"),
            (2, "--vault"),
            (1, "relative"),
            (1, "/tmp/../vault"),
            (5, "bad-uuid"),
            (9, ""),
        ] {
            let mut args = base.clone();
            args[index] = bad.into();
            assert!(parse(args).is_err());
        }
        assert!(parse(base[..8].to_vec()).is_err());
        assert!(parse([base, vec!["--help".into()]].concat()).is_err());
        assert!(parse(vec!["--help".into()]).unwrap().is_none());
    }
}
