//! 目标必须预先批准且为空；以逐层 nofollow 和物理祖先检查排除源、目标互相包含。
use crate::{
    BrokerError, FsVaultBroker,
    writer::{self, DirectoryPin, Identity},
};
use cap_std::fs::{Dir, MetadataExt as _};
use std::path::{Path, PathBuf};

pub(super) struct Targets {
    pub vault: Dir,
    pub data: Dir,
    pins: [DirectoryPin; 2],
}
impl Targets {
    pub fn new(broker: &FsVaultBroker, vault: &Path, data: &Path) -> Result<Self, BrokerError> {
        let (vault_path, vault_dir, vault_pin) = empty(vault)?;
        let (data_path, data_dir, data_pin) = empty(data)?;
        let (source_vault, source_data) = broker.writer.checkpoint_paths();
        let roots: [(&Path, Identity); 4] = [
            (&vault_path, Identity::directory(&vault_dir)?),
            (&data_path, Identity::directory(&data_dir)?),
            (source_vault, Identity::directory(&broker.root)?),
            (source_data, Identity::directory(&broker.data)?),
        ];
        for (index, (left, left_id)) in roots.iter().enumerate() {
            for (right, right_id) in &roots[index + 1..] {
                if left.starts_with(right)
                    || right.starts_with(left)
                    || writer::has_ancestor(left, *right_id)?
                    || writer::has_ancestor(right, *left_id)?
                {
                    return Err(BrokerError::WriterBindingMismatch);
                }
            }
        }
        Ok(Self {
            vault: vault_dir,
            data: data_dir,
            pins: [vault_pin, data_pin],
        })
    }
    pub fn check(&self) -> Result<(), BrokerError> {
        for (pin, dir) in self.pins.iter().zip([&self.vault, &self.data]) {
            pin.check()?;
            private(dir)?;
        }
        Ok(())
    }
}
fn private(dir: &Dir) -> Result<(), BrokerError> {
    let metadata = dir
        .dir_metadata()
        .map_err(|e| BrokerError::io("核对封存目标", e))?;
    if !metadata.is_dir()
        || metadata.mode() & 0o7777 != 0o700
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(BrokerError::WriterBindingMismatch);
    }
    Ok(())
}
fn empty(path: &Path) -> Result<(PathBuf, Dir, DirectoryPin), BrokerError> {
    let path = writer::absolute(path)?;
    let dir = writer::walk(&path, false)?;
    private(&dir)?;
    if dir
        .entries()
        .map_err(|e| BrokerError::io("核对空封存目录", e))?
        .next()
        .transpose()
        .map_err(|e| BrokerError::io("核对空封存节点", e))?
        .is_some()
    {
        return Err(BrokerError::WriterBindingMismatch);
    }
    let pin = DirectoryPin::new(path.clone(), &dir)?;
    Ok((path, dir, pin))
}
