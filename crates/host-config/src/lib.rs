//! 主人配置 Module：管理命令与服务器共享安全文件知识，不读取种子或创建存储。

mod config_file;
mod file_operations;
mod human_content;
mod serving;
mod sync_scope;

use serde::Deserialize;
use std::path::{Path, PathBuf};
use studiolo_auth::{AccessConfig, OwnerBinding, SessionConfig, SessionPolicy};

pub mod alerting;
pub mod backup;
pub mod process_safety;
pub mod sync_transport;

pub use file_operations::{ApprovedRenamePair, FileOperationPolicy, FileRenamePolicy};
pub use human_content::HumanContentScope;
pub use serving::{CanonicalServingRoots, RepositoryServingRoots, ServingRoots};
pub use sync_scope::{
    ApprovedSyncUnion, SyncAttachmentType, SyncBinding, SyncContentMetadata, SyncDirectoryDecision,
    SyncFileKind, SyncFilePolicy, SyncObservationSelection, SyncPathEvidence, SyncScope,
    SyncScopeError, SyncTraversalRule,
};

/// 固定脱敏错误，不包含配置正文、路径或底层文件系统诊断。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("非秘密配置或私有路径检查失败")]
pub struct ConfigError;

/// 只能经指定私有文件加载；无公开字段、构造器或反序列化入口。
///
/// 配置中的 seed_ref 只是非秘密标识，本 Module 从不将其解释为路径或读取环境。
/// ```compile_fail
/// let config: studiolo_host_config::Config = serde_json::from_str("{}").unwrap();
/// ```
pub struct Config {
    document: Document,
    file: config_file::Snapshot,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    auth_directory: PathBuf,
    audit_directory: PathBuf,
    #[serde(deserialize_with = "owner_object")]
    owner: Owner,
    idle_seconds: u64,
    absolute_seconds: u64,
    seed_ref: String,
    #[serde(default, deserialize_with = "human_content::decode")]
    human_content: Option<HumanContentScope>,
    #[serde(default, deserialize_with = "serving::decode_repository")]
    project_repository: Option<serving::RepositoryScope>,
    #[serde(default, deserialize_with = "sync_scope::decode")]
    sync_scope: Option<sync_scope::Document>,
    #[serde(default, deserialize_with = "file_operations::decode")]
    file_operations: Option<file_operations::Document>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    issuer: String,
    audience: String,
    subject: String,
    email: String,
}

fn owner_object<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Owner, D::Error> {
    struct Object;
    impl<'de> serde::de::Visitor<'de> for Object {
        type Value = Owner;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("主人配置对象")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<Owner, M::Error> {
            Owner::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    // 只接受对象，避免serde把字段齐全的数组当作struct；仍保留重复字段检查。
    deserializer.deserialize_map(Object)
}

impl Config {
    /// 加载原 JSON v1；检查16KiB、0600/0700、当前UID、链接及读取期间对象稳定性。
    ///
    /// 不创建目录、数据库或文件，不修复权限；依赖可信且稳定的父目录。
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let file = config_file::Snapshot::read(path)?;
        if file.bytes().iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
            return Err(ConfigError);
        }
        let document: Document = serde_json::from_slice(file.bytes()).map_err(|_| ConfigError)?;
        let config = Self { document, file };
        config.validate()?;
        Ok(config)
    }

    /// 已批准的私有 Auth 目录；数据库打开策略仍由调用者显式选择。
    #[must_use]
    pub fn auth_directory(&self) -> &Path {
        &self.document.auth_directory
    }

    /// 已批准的私有 Audit 目录。
    #[must_use]
    pub fn audit_directory(&self) -> &Path {
        &self.document.audit_directory
    }

    /// 可信管理命令及显式因子启动用于匹配外部注入的种子；本Module不读取秘密。
    #[must_use]
    pub fn seed_ref(&self) -> &str {
        &self.document.seed_ref
    }

    /// 由同一主人和显式TTL派生持久会话配置，不授予身份或改变磁盘绑定。
    pub fn sessions(&self) -> Result<SessionConfig, ConfigError> {
        let owner = &self.document.owner;
        let owner = OwnerBinding::new(
            owner.issuer.clone(),
            owner.audience.clone(),
            owner.subject.clone(),
            owner.email.clone(),
        )
        .map_err(|_| ConfigError)?;
        let policy = SessionPolicy::new(self.document.idle_seconds, self.document.absolute_seconds)
            .map_err(|_| ConfigError)?;
        Ok(SessionConfig::new(owner, policy))
    }

    /// 从同一主人字段派生真实Access配置；取钥地址与身份不能被独立覆盖。
    pub fn access(&self, public_origin: &str) -> Result<AccessConfig, ConfigError> {
        let owner = &self.document.owner;
        AccessConfig::new(
            public_origin.to_owned(),
            owner.issuer.clone(),
            owner.audience.clone(),
            owner.email.clone(),
        )
        .map_err(|_| ConfigError)
    }

    /// 固定并检查四个服务根；根之间两两不相同、不嵌套，配置文件不可被公开根包含。
    pub fn serving_roots(&self, vault: &Path, assets: &Path) -> Result<ServingRoots, ConfigError> {
        ServingRoots::new(self, vault, assets)
    }

    /// 固定已批准的 canonical、资源和私有写者数据根；不打开或初始化 Broker。
    pub fn canonical_serving_roots(
        &self,
        vault: &Path,
        assets: &Path,
        writer_data: &Path,
    ) -> Result<CanonicalServingRoots, ConfigError> {
        CanonicalServingRoots::new(self, vault, assets, writer_data)
    }

    /// 私有配置显式批准的单仓库；缺省不读取仓库路径。只批准目录和可选 Cargo。
    /// 环境种子没有可推断的文件来源；这里只隔离真实配置目录及明确的服务根。
    pub fn repository_serving_roots(
        &self,
        vault: &Path,
        assets: &Path,
        writer_data: Option<&Path>,
        ingress_socket: Option<&Path>,
    ) -> Result<Option<RepositoryServingRoots>, ConfigError> {
        self.document
            .project_repository
            .as_ref()
            .map(|scope| {
                RepositoryServingRoots::new(self, scope, vault, assets, writer_data, ingress_socket)
            })
            .transpose()
    }

    /// 私有配置显式批准双向整文件范围；缺省关闭，不授予文件系统根或写者能力。
    pub fn sync_scope(&self) -> Result<Option<SyncScope>, ConfigError> {
        self.document
            .sync_scope
            .as_ref()
            .map(|document| SyncScope::new(self, document))
            .transpose()
    }

    /// 人侧删除/最近删除恢复的显式批准；缺省不启用。
    pub fn file_operation_policy(&self) -> Result<Option<FileOperationPolicy>, ConfigError> {
        self.document
            .file_operations
            .as_ref()
            .map(|document| FileOperationPolicy::new(self, document))
            .transpose()
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let document = &self.document;
        if document.version != 1
            || !absolute_normal(self.auth_directory())
            || !absolute_normal(self.audit_directory())
            || self.auth_directory().starts_with(self.audit_directory())
            || self.audit_directory().starts_with(self.auth_directory())
            || document.seed_ref.is_empty()
            || document.seed_ref.len() > 64
            || !document
                .seed_ref
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(ConfigError);
        }
        self.sessions()?;
        if let Some(scope) = &document.human_content {
            scope.validate()?;
        }
        if let Some(scope) = &document.project_repository {
            scope.validate()?;
        }
        if let Some(scope) = &document.sync_scope {
            scope.validate(document.human_content.as_ref())?;
        }
        if let Some(policy) = &document.file_operations {
            policy.validate(
                document.human_content.as_ref(),
                document.sync_scope.as_ref(),
            )?;
        }
        config_file::private_directory(self.auth_directory())?;
        config_file::private_directory(self.audit_directory())?;
        config_file::disjoint(self.auth_directory(), self.audit_directory())
    }
}

fn absolute_normal(path: &Path) -> bool {
    path.to_str().is_some_and(|value| {
        value.starts_with('/')
            && !value.chars().any(char::is_control)
            && (value == "/"
                || value[1..]
                    .split('/')
                    .all(|part| !part.is_empty() && part != "." && part != ".."))
    })
}
