//! 新目录恢复的行为、失败与恢复契约测试。
//!
//! 夹具模拟真实流程：旧配对存储 → 恢复工具把字节复制到**新目录** →
//! Broker 做物理重绑定。复制出来的新目录在重绑定完成之前必须始终不可服务。

mod behaviour;
mod drift;
mod failures;

use super::{NewRootRebind, RestoreError, RestoreFault, RestoreReceipt};
use crate::writer::Binding;
use crate::{FsVaultBroker, Mutation, VaultPath, VaultUuid, VaultWriter};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) struct Fixture {
    _temp: tempfile::TempDir,
    old_vault: PathBuf,
    old_data: PathBuf,
    new_vault: PathBuf,
    new_data: PathBuf,
}

impl Fixture {
    /// 建一对真实存储，写入两次内容，再整树复制到新目录。
    fn restored() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("studiolo-t048-restore-")
            .tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap())
            .unwrap();
        let old_vault = temp.path().join("old/vault");
        let old_data = temp.path().join("old/data");
        {
            let mut broker = FsVaultBroker::open(&old_vault, &old_data).unwrap();
            let first = broker
                .mutate(Mutation::create(note(), b"first".to_vec(), "key-one"))
                .unwrap();
            broker
                .mutate(Mutation::replace(
                    note(),
                    b"second".to_vec(),
                    first.revision,
                    "key-two",
                ))
                .unwrap();
        }
        let new_vault = temp.path().join("new/vault");
        let new_data = temp.path().join("new/data");
        copy_tree(&old_vault, &new_vault);
        copy_tree(&old_data, &new_data);
        Self {
            _temp: temp,
            old_vault,
            old_data,
            new_vault,
            new_data,
        }
    }

    fn sealed_binding(&self) -> Binding {
        let bytes = fs::read(self.new_data.join("broker-binding.json")).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn request(&self) -> NewRootRebind<'_> {
        let binding = self.sealed_binding();
        NewRootRebind {
            new_vault_root: &self.new_vault,
            new_data_root: &self.new_data,
            expected_vault_uuid: binding.vault_uuid,
            expected_pair_id: binding.pair_id,
            checkpoint_id: "cp-2026-09-17-0001".to_owned(),
            parent_lineage: None,
            resume_interrupted: false,
        }
    }

    fn rebind(&self) -> Result<RestoreReceipt, RestoreError> {
        super::rebind_new_roots(&self.request())
    }

    fn rebind_with_fault(&self, fault: RestoreFault) -> Result<RestoreReceipt, RestoreError> {
        super::run(&self.request(), fault)
    }

    fn open_new(&self) -> Result<FsVaultBroker, crate::BrokerError> {
        FsVaultBroker::open_existing(&self.new_vault, &self.new_data)
    }
}

pub(super) fn note() -> VaultPath {
    VaultPath::parse("note.md").unwrap()
}

pub(super) fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let file_type = entry.file_type().unwrap();
        let target = to.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&entry.path(), &target);
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// 读取任意一处 binding 的逻辑配对标识。
pub(super) fn pair_of(path: &Path) -> VaultUuid {
    let bytes = fs::read(path).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value
        .get("pair_id")
        .and_then(serde_json::Value::as_str)
        .unwrap()
        .parse()
        .unwrap()
}
