//! 服务根的固定身份；复核不把路径检查冒充管理员隔离或完全消除路径替换竞态。

use crate::{Config, ConfigError, config_file};
use std::path::{Path, PathBuf};

mod canonical;
pub use canonical::CanonicalServingRoots;
mod repository;
pub use repository::RepositoryServingRoots;
pub(crate) use repository::{RepositoryScope, decode_repository};

struct Root {
    path: PathBuf,
    identity: (u64, u64),
    private: bool,
}

/// 配置文件与四个根的封闭快照；不能公开构造或修改。
///
/// 身份不含目录mtime：合法SQLite侧文件创建不等于根被替换。依赖可信稳定父目录，
/// 不能识别管理员一致克隆或让检查与后续任意路径操作获得系统级原子性。
pub struct ServingRoots {
    file: config_file::Snapshot,
    roots: [Root; 4],
}

impl ServingRoots {
    pub(crate) fn new(config: &Config, vault: &Path, assets: &Path) -> Result<Self, ConfigError> {
        config.file.revalidate()?;
        let snapshot = Self {
            file: config.file.clone(),
            roots: [
                Root::new(config.auth_directory(), true)?,
                Root::new(config.audit_directory(), true)?,
                Root::new(vault, false)?,
                Root::new(assets, false)?,
            ],
        };
        snapshot.revalidate()?;
        Ok(snapshot)
    }

    /// 供只读Vault实际打开后的根句柄比较，不能作为后续请求的授权凭据。
    #[must_use]
    pub fn vault_identity(&self) -> (u64, u64) {
        self.roots[2].identity
    }

    /// 供静态资源实际打开后的根句柄比较。
    #[must_use]
    pub fn assets_identity(&self) -> (u64, u64) {
        self.roots[3].identity
    }

    /// 监听前重验配置字节/身份、四根身份/权限/无链接及全部分离关系。
    pub fn revalidate(&self) -> Result<(), ConfigError> {
        self.file.revalidate()?;
        for root in &self.roots {
            root.check()?;
        }
        for (index, root) in self.roots.iter().enumerate() {
            for other in &self.roots[index + 1..] {
                config_file::disjoint(&root.path, &other.path)?;
            }
        }
        for public in &self.roots[2..] {
            config_file::outside(&self.file.path, public.identity)?;
        }
        // 关系检查也是路径读取，结束后再次确认固定根和配置对象仍未变化。
        for root in &self.roots {
            root.check()?;
        }
        self.file.revalidate()
    }
}

impl Root {
    fn new(path: &Path, private: bool) -> Result<Self, ConfigError> {
        let identity = if private {
            config_file::private_directory(path)?
        } else {
            config_file::directory(path)?
        };
        Ok(Self {
            path: path.to_path_buf(),
            identity,
            private,
        })
    }

    fn check(&self) -> Result<(), ConfigError> {
        let current = if self.private {
            config_file::private_directory(&self.path)?
        } else {
            config_file::directory(&self.path)?
        };
        if current != self.identity {
            return Err(ConfigError);
        }
        Ok(())
    }
}
