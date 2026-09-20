//! 类型凭据与实例登记；摘要复核用于撤销已打开的传输流。
use super::{
    AgentStore, Instance, IssuedToken, SupplyError, TOKEN_PREFIX, unavailable, validate_type_name,
};
use rusqlite::{OptionalExtension as _, params};
use sha2::{Digest as _, Sha256};

impl AgentStore {
    /// 签发（或轮换）一个类型的 token；旧 token 立即失效。明文只返回这一次。
    pub fn issue_type(&self, name: &str, now: &str) -> Result<IssuedToken, SupplyError> {
        validate_type_name(name)?;
        let token = format!("{TOKEN_PREFIX}{}", random_hex_64());
        let digest = sha256_hex(&token);
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO agent_type (name, token_sha256, issued_at, revoked_at)
                 VALUES (?1, ?2, ?3, NULL)
                 ON CONFLICT(name) DO UPDATE SET token_sha256 = excluded.token_sha256,
                     issued_at = excluded.issued_at, revoked_at = NULL",
                params![name, digest, now],
            )
            .map_err(unavailable)?;
        Ok(IssuedToken {
            agent_type: name.to_owned(),
            token,
        })
    }

    /// 吊销类型：即刻拒绝新握手与后续调用。返回是否存在该类型。
    pub fn revoke_type(&self, name: &str, now: &str) -> Result<bool, SupplyError> {
        validate_type_name(name)?;
        let connection = self.lock()?;
        let changed = connection
            .execute(
                "UPDATE agent_type SET revoked_at = ?2 WHERE name = ?1 AND revoked_at IS NULL",
                params![name, now],
            )
            .map_err(unavailable)?;
        Ok(changed == 1)
    }

    /// 已登记的类型及其状态（名字、签发时间、是否吊销）；不返回摘要。
    pub fn types(&self) -> Result<Vec<(String, String, bool)>, SupplyError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT name, issued_at, revoked_at IS NOT NULL FROM agent_type ORDER BY name")
            .map_err(unavailable)?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(unavailable)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(unavailable)
    }

    /// 用明文 token 认证：返回未吊销的类型名；格式不符、未知或已吊销都返回 `None`。
    pub fn authenticate(&self, token: &str) -> Result<Option<String>, SupplyError> {
        if !token.starts_with(TOKEN_PREFIX)
            || token.len() != TOKEN_PREFIX.len() + 64
            || !token[TOKEN_PREFIX.len()..]
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Ok(None);
        }
        let digest = sha256_hex(token);
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT name FROM agent_type WHERE token_sha256 = ?1 AND revoked_at IS NULL",
                params![digest],
                |row| row.get(0),
            )
            .optional()
            .map_err(unavailable)
    }

    /// 类型是否仍有效（用于每次 tool 调用的即刻吊销检查）。
    pub fn type_is_active(&self, name: &str) -> Result<bool, SupplyError> {
        let connection = self.lock()?;
        let active: Option<i64> = connection
            .query_row(
                "SELECT 1 FROM agent_type WHERE name = ?1 AND revoked_at IS NULL",
                params![name],
                |row| row.get(0),
            )
            .optional()
            .map_err(unavailable)?;
        Ok(active.is_some())
    }

    /// 注册一个实例：ULID id，绑定类型与会话摘要；实例无独立权限。
    pub fn register_instance(
        &self,
        agent_type: &str,
        session_id: &str,
        now: &str,
    ) -> Result<Instance, SupplyError> {
        let id = ulid::Ulid::generate().to_string();
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO agent_instance (id, type_name, session_digest, registered_at, last_seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                params![id, agent_type, sha256_hex(session_id), now],
            )
            .map_err(unavailable)?;
        Ok(Instance {
            id,
            agent_type: agent_type.to_owned(),
            registered_at: now.to_owned(),
        })
    }

    /// 已注册实例（新到旧）：实例、最近活动时间；不返回会话摘要。
    pub fn instances(&self, limit: usize) -> Result<Vec<(Instance, String)>, SupplyError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, type_name, registered_at, last_seen_at FROM agent_instance
                 ORDER BY registered_at DESC, id DESC LIMIT ?1",
            )
            .map_err(unavailable)?;
        let rows = statement
            .query_map(params![limit.clamp(1, 10_000) as i64], |row| {
                Ok((
                    Instance {
                        id: row.get(0)?,
                        agent_type: row.get(1)?,
                        registered_at: row.get(2)?,
                    },
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(unavailable)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(unavailable)
    }

    /// 更新实例最近活动时间。
    pub fn touch_instance(&self, instance_id: &str, now: &str) -> Result<(), SupplyError> {
        let connection = self.lock()?;
        connection
            .execute(
                "UPDATE agent_instance SET last_seen_at = ?2 WHERE id = ?1",
                params![instance_id, now],
            )
            .map_err(unavailable)?;
        Ok(())
    }

    /// 摘要仅供服务内部绑定会话；当前 token 已轮换或类型吊销时返回 false。
    pub fn credential_is_current(
        &self,
        agent_type: &str,
        digest: &str,
    ) -> Result<bool, SupplyError> {
        let connection = self.lock()?;
        let found: Option<i64> = connection.query_row(
            "SELECT 1 FROM agent_type WHERE name = ?1 AND token_sha256 = ?2 AND revoked_at IS NULL",
            params![agent_type, digest], |row| row.get(0),
        ).optional().map_err(unavailable)?;
        Ok(found.is_some())
    }
}

fn random_hex_64() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
