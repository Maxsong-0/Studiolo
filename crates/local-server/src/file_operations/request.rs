//! HTTP 只表达原路径条件和重试键；主体、scope、预算及 MustNotExist 均由 owner 固定。
use serde::Deserialize;
use studiolo_core::{Revision, VaultPath, VaultUuid};

pub(crate) struct Input {
    pub vault: VaultUuid,
    pub path: VaultPath,
    pub kind: Kind,
    pub key: String,
}
pub(crate) enum Kind {
    Delete(Revision),
    Restore(String),
    Rename { expected: Revision, to: VaultPath },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Body {
    Rename {
        vault_uuid: VaultUuid,
        from: VaultPath,
        to: VaultPath,
        expected_revision: String,
        idempotency_key: String,
    },
    Delete {
        vault_uuid: VaultUuid,
        path: VaultPath,
        expected_revision: String,
        idempotency_key: String,
    },
    RestoreTombstone {
        vault_uuid: VaultUuid,
        path: VaultPath,
        tombstone_id: String,
        idempotency_key: String,
    },
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Input, ()> {
    if bytes.len() > super::JSON_LIMIT
        || bytes.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{')
    {
        return Err(());
    }
    let (vault, path, kind, key) = match serde_json::from_slice(bytes).map_err(|_| ())? {
        Body::Rename {
            vault_uuid,
            from,
            to,
            expected_revision,
            idempotency_key,
        } => {
            if to.as_str().len() > 4096 || from == to {
                return Err(());
            }
            (
                vault_uuid,
                from,
                Kind::Rename {
                    expected: crate::editing::revision(&expected_revision)?,
                    to,
                },
                idempotency_key,
            )
        }
        Body::Delete {
            vault_uuid,
            path,
            expected_revision,
            idempotency_key,
        } => (
            vault_uuid,
            path,
            Kind::Delete(crate::editing::revision(&expected_revision)?),
            idempotency_key,
        ),
        Body::RestoreTombstone {
            vault_uuid,
            path,
            tombstone_id,
            idempotency_key,
        } => {
            if !hex(&tombstone_id) {
                return Err(());
            }
            (
                vault_uuid,
                path,
                Kind::Restore(tombstone_id),
                idempotency_key,
            )
        }
    };
    if path.as_str().len() > 4096
        || !(16..=128).contains(&key.len())
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(());
    }
    Ok(Input {
        vault,
        path,
        kind,
        key,
    })
}

fn hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PageQuery {
    pub vault_uuid: VaultUuid,
    #[serde(default = "default_limit")]
    pub limit: usize,
    pub cursor: Option<String>,
}
fn default_limit() -> usize {
    32
}
impl PageQuery {
    pub fn valid(&self) -> bool {
        (1..=64).contains(&self.limit)
            && self.cursor.as_ref().is_none_or(|value| {
                !value.is_empty()
                    && value.len() <= 100
                    && value.is_ascii()
                    && !value.chars().any(char::is_control)
            })
    }
}
