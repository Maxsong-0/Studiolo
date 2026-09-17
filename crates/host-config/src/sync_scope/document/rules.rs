//! 每条规则必须是对象；不接受序列冒充结构或省略种类/字节限制。

use super::object;
use crate::sync_scope::{SyncAttachmentType, SyncFilePolicy};
use serde::{Deserialize, Deserializer};
use studiolo_core::VaultPath;

#[derive(Clone)]
pub(crate) struct FolderRule {
    pub(in crate::sync_scope) prefix: VaultPath,
    pub(in crate::sync_scope) policy: SyncFilePolicy,
}

#[derive(Clone)]
pub(crate) struct FileRule {
    pub(in crate::sync_scope) path: VaultPath,
    pub(in crate::sync_scope) policy: SyncFilePolicy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Folder {
    prefix: VaultPath,
    markdown: bool,
    attachment_types: Vec<SyncAttachmentType>,
    max_bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    path: VaultPath,
    markdown: bool,
    attachment_types: Vec<SyncAttachmentType>,
    max_bytes: u64,
}

impl<'de> Deserialize<'de> for FolderRule {
    fn deserialize<D: Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        let rule: Folder = object(input)?;
        Ok(Self {
            prefix: rule.prefix,
            policy: SyncFilePolicy {
                markdown: rule.markdown,
                attachment_types: rule.attachment_types,
                max_bytes: rule.max_bytes,
            },
        })
    }
}

impl<'de> Deserialize<'de> for FileRule {
    fn deserialize<D: Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        let rule: File = object(input)?;
        Ok(Self {
            path: rule.path,
            policy: SyncFilePolicy {
                markdown: rule.markdown,
                attachment_types: rule.attachment_types,
                max_bytes: rule.max_bytes,
            },
        })
    }
}
