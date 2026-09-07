//! 构建期完整性检查；不进行文件系统、网络或进程调用。

use std::collections::{BTreeMap, BTreeSet};

use studiolo_core::VaultPath;
use studiolo_vault_index::MarkdownDocument;

use crate::{TreeError, TreeInput, TreeKind};

pub(crate) fn validate(
    entries: &[TreeInput],
    documents: &[MarkdownDocument],
) -> Result<(), TreeError> {
    let mut nodes = BTreeMap::new();
    let mut collision_keys = BTreeSet::new();
    let mut markdown_count = 0;
    for entry in entries {
        let path = entry.path.as_str();
        if !visible_path(path)
            || nodes.insert(path, entry).is_some()
            || !collision_keys.insert(entry.path.collision_key())
        {
            return Err(TreeError::InvalidInput);
        }
        let valid_kind = match entry.kind {
            TreeKind::Directory => entry.size_bytes.is_none(),
            TreeKind::Markdown => {
                markdown_count += 1;
                entry.size_bytes.is_some() && markdown_extension(path)
            }
            TreeKind::Attachment => entry.size_bytes.is_some() && !markdown_extension(path),
        };
        if !valid_kind {
            return Err(TreeError::InvalidInput);
        }
    }
    for entry in entries {
        let parent = parent(entry.path.as_str());
        if !parent.is_empty()
            && !nodes
                .get(parent)
                .is_some_and(|node| node.kind == TreeKind::Directory)
        {
            return Err(TreeError::InvalidInput);
        }
    }
    let mut document_paths = BTreeSet::new();
    for document in documents {
        let path = document.path().as_str();
        let length =
            u64::try_from(document.contents().len()).map_err(|_| TreeError::InvalidInput)?;
        if !document_paths.insert(path)
            || !nodes.get(path).is_some_and(|node| {
                node.kind == TreeKind::Markdown && node.size_bytes == Some(length)
            })
        {
            return Err(TreeError::InvalidInput);
        }
    }
    if document_paths.len() != markdown_count {
        return Err(TreeError::InvalidInput);
    }
    Ok(())
}

pub(crate) fn visible_path(path: &str) -> bool {
    VaultPath::parse(path).is_ok() && !path.split('/').any(|part| part.starts_with('.'))
}

pub(crate) fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn markdown_extension(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|name| {
        name.rsplit_once('.')
            .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("md"))
    })
}
