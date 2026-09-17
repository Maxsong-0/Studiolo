//! Rename 的双路径持久事实；旧 v5 记录没有这些字段。
use super::{
    FileOperationKind, FileOperationPhase, FilePresence, observation::Observation, record::Active,
};
use crate::{BrokerError, VaultPath, blob::file::ContentProof};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Progress {
    pub before: Observation,
    pub history: Option<ContentProof>,
    pub target_revision: Option<crate::Revision>,
    pub executed: Option<Observation>,
    pub source_check: Option<Observation>,
    pub target_effect_at_ms: Option<i64>,
    pub install_id: String,
    pub detach_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TerminalProgress {
    pub before: FilePresence,
    pub phase: FileOperationPhase,
    pub target_revision: Option<crate::Revision>,
    pub target_effect_at_ms: Option<i64>,
}
pub(crate) fn get(active: &Active) -> Result<(&VaultPath, &Progress), BrokerError> {
    match (&active.frozen.kind, &active.rename) {
        (FileOperationKind::Rename { to, .. }, Some(r)) => Ok((to, r)),
        _ => Err(BrokerError::CorruptWriterState),
    }
}
pub(crate) fn step(id: &str, role: &str) -> String {
    crate::sha256(format!("file-rename-step-v1:{id}:{role}").as_bytes())
}
pub(crate) fn paths(from: &VaultPath, to: &VaultPath) -> Result<(), BrokerError> {
    super::frozen::validate_path(from)?;
    super::frozen::validate_path(to)?;
    let extension = |p: &VaultPath| {
        p.as_str()
            .rsplit('/')
            .next()
            .and_then(|n| n.rsplit_once('.'))
            .map(|(_, e)| e.to_lowercase())
    };
    if extension(from) != extension(to) {
        return Err(BrokerError::InvalidArtifact(
            "不允许跨扩展名类别重命名".into(),
        ));
    }
    let a = from.as_str().to_lowercase();
    let b = to.as_str().to_lowercase();
    if a == b || a.starts_with(&(b.clone() + "/")) || b.starts_with(&(a + "/")) {
        return Err(BrokerError::InvalidArtifact(
            "重命名路径相同、别名或前缀交叠".into(),
        ));
    }
    Ok(())
}
