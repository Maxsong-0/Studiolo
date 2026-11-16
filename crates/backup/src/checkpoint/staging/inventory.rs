//! 只从真实复验过的视图及三个原元数据文件构造，供远端逐项核对。
use super::{
    Result, StagingError, check_deadline,
    tree::{FileProof, TreeProof},
};
use std::time::Instant;
use studiolo_core::VaultPath;

/// 经实际封存复验的完整传输清单；没有公开构造器或反序列化入口。
#[derive(PartialEq, Eq)]
pub struct SealedInventory {
    tree: TreeProof,
}
/// 清单中一个已经核验过的普通文件；引用不会包含文件正文。
pub struct SealedFile<'a> {
    path: &'a VaultPath,
    proof: &'a FileProof,
}
impl SealedInventory {
    pub(super) fn from_data(
        data: TreeProof,
        metadata: [(&str, String, u64); 3],
        deadline: Instant,
    ) -> Result<Self> {
        let mut tree = TreeProof::default();
        tree.directories.insert("data".into());
        for directory in data.directories {
            check_deadline(deadline)?;
            tree.directories.insert(format!("data/{directory}"));
        }
        for (path, proof) in data.files {
            check_deadline(deadline)?;
            tree.files.insert(
                VaultPath::parse(format!("data/{}", path.as_str()))
                    .map_err(|_| StagingError::Manifest)?,
                proof,
            );
        }
        for (name, sha256, bytes) in metadata {
            check_deadline(deadline)?;
            let path = VaultPath::parse(name).map_err(|_| StagingError::Manifest)?;
            if tree
                .files
                .insert(path, FileProof { sha256, bytes })
                .is_some()
            {
                return Err(StagingError::Manifest);
            }
        }
        Ok(Self { tree })
    }
    /// 全部相对目录，含 data 根；不含快照中的宿主机祖先目录。
    pub fn directories(&self) -> impl Iterator<Item = &str> {
        self.tree.directories.iter().map(String::as_str)
    }
    /// 全部相对普通文件；包含 seal.json、manifest.json 和 inclusion.json 的原字节摘要。
    pub fn files(&self) -> impl Iterator<Item = SealedFile<'_>> {
        self.tree
            .files
            .iter()
            .map(|(path, proof)| SealedFile { path, proof })
    }
}
impl SealedFile<'_> {
    /// 已核验的相对路径。
    #[must_use]
    pub fn path(&self) -> &VaultPath {
        self.path
    }
    /// 原封存实际字节数。
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.proof.bytes
    }
    /// 原封存实际 SHA-256。
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.proof.sha256
    }
}
