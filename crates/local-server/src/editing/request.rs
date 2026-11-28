//! 严格 HTTP DTO；客户端不能指定主体、内部 precondition 或绕过 stale。

use super::MAX_BYTES;
use serde::Deserialize;
use studiolo_core::{Revision, VaultPath, VaultUuid};

pub(crate) struct Replace {
    pub vault: VaultUuid,
    pub path: VaultPath,
    pub expected: Revision,
    pub content: String,
    pub source: Option<Revision>,
    pub key: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Body {
    Replace {
        vault_uuid: VaultUuid,
        path: VaultPath,
        expected_revision: String,
        content: String,
        idempotency_key: String,
    },
    Restore {
        vault_uuid: VaultUuid,
        path: VaultPath,
        expected_revision: String,
        source_revision: String,
        idempotency_key: String,
    },
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Replace, ()> {
    if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
        return Err(());
    }
    let body: Body = serde_json::from_slice(bytes).map_err(|_| ())?;
    let (vault, path, expected, content, source, key) = match body {
        Body::Replace {
            vault_uuid,
            path,
            expected_revision,
            content,
            idempotency_key,
        } => (
            vault_uuid,
            path,
            revision(&expected_revision)?,
            content,
            None,
            idempotency_key,
        ),
        Body::Restore {
            vault_uuid,
            path,
            expected_revision,
            source_revision,
            idempotency_key,
        } => (
            vault_uuid,
            path,
            revision(&expected_revision)?,
            String::new(),
            Some(revision(&source_revision)?),
            idempotency_key,
        ),
    };
    if content.len() > MAX_BYTES
        || content.contains('\0')
        || !path.as_str().to_lowercase().ends_with(".md")
        || !(16..=128).contains(&key.len())
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(());
    }
    Ok(Replace {
        vault,
        path,
        expected,
        content,
        source,
        key,
    })
}

pub(crate) fn revision(value: &str) -> Result<Revision, ()> {
    studiolo_core::from_lower_hex(value)
        .map(Revision::from_bytes)
        .ok_or(())
}
