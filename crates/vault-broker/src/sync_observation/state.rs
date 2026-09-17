//! 内核证据只在本 crate 流转；名字索引不会随公开路径集合泄漏。
use super::{SyncReadSelection, paths};
use crate::blob::file::{ContentProof, NodeProof, Stamp};
use crate::{BlobIdentity, Revision, VaultPath};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct Directory {
    pub stamp: Stamp,
    pub names: BTreeSet<String>,
}
pub(crate) enum Candidate {
    Present(NodeProof),
    Missing(Vec<(String, Stamp)>),
}
pub(crate) enum Proof {
    Present(ContentProof),
    Missing(Vec<(String, Stamp)>),
}
pub(crate) struct Survey {
    pub selection: SyncReadSelection,
    pub required: Vec<VaultPath>,
    pub paths: BTreeSet<VaultPath>,
    pub(super) index: paths::Index,
    pub directories: BTreeMap<String, Directory>,
    pub candidates: BTreeMap<VaultPath, Candidate>,
}
#[derive(Default)]
pub(crate) struct Proofs {
    pub directories: BTreeMap<String, Stamp>,
    pub files: BTreeMap<VaultPath, Proof>,
}
pub(crate) struct Prepared {
    pub selection: SyncReadSelection,
    pub candidates: BTreeMap<VaultPath, Candidate>,
    pub proofs: Proofs,
}
pub(crate) struct Fact {
    pub identity: BlobIdentity,
    pub revision: Option<Revision>,
    pub utf8_valid: bool,
}
