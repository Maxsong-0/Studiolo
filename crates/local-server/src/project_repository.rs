//! 显式绑定的只读仓库能力；默认仅目录，固定清单须另行授权，不接受宿主路径查询。

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use axum::{Extension, Router};
use serde::Serialize;
use studiolo_core::VaultPath;
use studiolo_vault_broker::{BrokerError, ReadOnlyVault};
use studiolo_workbench::DirectoryObservation;

use crate::LocalError;

const MAX_OBSERVED_DIRECTORIES: usize = 1024;
const MAX_OBSERVED_PATH_BYTES: usize = 256 * 1024;

/// 启动者明确指定的一个项目及源码根；默认只允许目录枚举。
pub struct ProjectRepository {
    project: String,
    root: ReadOnlyVault,
    root_path: PathBuf,
    cargo_dependencies: bool,
}

impl ProjectRepository {
    /// 打开一个既有绝对仓库根；不读取源码、初始化或自动查找其他仓库。
    pub fn open(project: &str, root: &Path) -> Result<Self, LocalError> {
        let path = VaultPath::parse(project).map_err(|_| LocalError::Configuration)?;
        let components: Vec<_> = path.as_str().split('/').collect();
        if !root.is_absolute()
            || components.len() != 2
            || components[0] != "项目"
            || components[1].starts_with('.')
            || path.is_protected()
        {
            return Err(LocalError::Configuration);
        }
        Ok(Self {
            project: project.to_owned(),
            root: ReadOnlyVault::open(root)?,
            root_path: root.to_owned(),
            cargo_dependencies: false,
        })
    }

    /// 显式启用固定 Cargo 清单读取；不在启用时读取文件或扩大目录观测范围。
    #[must_use]
    pub fn with_cargo_dependencies(mut self) -> Self {
        self.cargo_dependencies = true;
        self
    }

    pub(crate) fn observe_cargo_dependencies(
        &self,
    ) -> crate::project_cargo::CargoDependencyObservation {
        if !self.cargo_dependencies {
            return crate::project_cargo::CargoDependencyObservation::disabled();
        }
        crate::project_cargo::observe(&self.root, || self.revalidate())
    }

    /// 监听前和请求前复核根绑定；替换后的路径不能获得原绑定的读取权限。
    pub fn revalidate(&self) -> Result<(), LocalError> {
        let current = ReadOnlyVault::open(&self.root_path)?;
        if current.root_identity()? != self.root.root_identity()? {
            return Err(LocalError::Unavailable);
        }
        Ok(())
    }

    pub(crate) fn matches(&self, project: &str) -> bool {
        self.project == project
    }

    pub(crate) fn observe(&self, parents: &[String]) -> RepositoryCheck {
        let checked = (|| {
            self.revalidate().ok()?;
            let first = self.capture(parents)?;
            let second = self.capture(parents)?;
            self.revalidate().ok()?;
            // 两轮相同只证明此次有界观测稳定，不承诺跨根原子性或后续不会变更。
            if first != second {
                return None;
            }
            let observed_at = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .ok()?
                .as_secs()
                .to_string();
            Some((second, observed_at))
        })();
        match checked {
            Some((observations, observed_at)) => RepositoryCheck {
                status: "ready",
                observed_at: Some(observed_at),
                observations,
            },
            None => RepositoryCheck::unavailable(),
        }
    }

    fn capture(&self, parents: &[String]) -> Option<Vec<DirectoryObservation>> {
        if parents.is_empty() || parents.len() > 64 {
            return None;
        }
        let mut count = 0;
        let mut bytes = 0;
        let mut observations = Vec::new();
        for parent in parents {
            let path = if parent.is_empty() {
                None
            } else {
                Some(VaultPath::parse(parent).ok()?)
            };
            let (status, children) = match self.root.list_directories(path.as_ref(), 8192) {
                Ok(children) => (
                    "ready",
                    children.iter().map(ToString::to_string).collect::<Vec<_>>(),
                ),
                Err(BrokerError::NotFound(missing)) if missing_parent(path.as_ref(), &missing) => {
                    ("missing", Vec::new())
                }
                Err(_) => return None,
            };
            count += children.len();
            bytes += children.iter().map(String::len).sum::<usize>();
            if count > MAX_OBSERVED_DIRECTORIES || bytes > MAX_OBSERVED_PATH_BYTES {
                return None;
            }
            observations.push(DirectoryObservation {
                path: parent.clone(),
                status: status.to_owned(),
                children,
            });
        }
        Some(observations)
    }
}

fn missing_parent(requested: Option<&VaultPath>, missing: &VaultPath) -> bool {
    requested.is_some_and(|requested| {
        requested == missing || requested.as_str().starts_with(&format!("{missing}/"))
    })
}

/// 将已打开的仓库能力加到只读工作台；既有路由认证/来源门禁仍逐请求执行。
///
/// 可信装配方负责根隔离；生产CLI首片仅允许旧本机/Access只读模式使用此能力。
pub fn with_project_repository(app: Router, repository: Arc<ProjectRepository>) -> Router {
    app.layer(Extension(repository))
}

#[derive(Serialize)]
pub(crate) struct RepositoryCheck {
    pub status: &'static str,
    pub observed_at: Option<String>,
    #[serde(skip)]
    pub observations: Vec<DirectoryObservation>,
}

impl RepositoryCheck {
    pub(crate) fn unbound() -> Self {
        Self {
            status: "unbound",
            observed_at: None,
            observations: Vec::new(),
        }
    }

    pub(crate) fn unavailable() -> Self {
        Self {
            status: "unavailable",
            observed_at: None,
            observations: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::missing_parent;
    use studiolo_core::VaultPath;

    #[test]
    fn a_child_disappearing_during_listing_is_not_a_missing_parent() {
        let parent = VaultPath::parse("src").unwrap();
        let child = VaultPath::parse("src/model").unwrap();
        let sibling = VaultPath::parse("src-other").unwrap();
        assert!(!missing_parent(None, &parent));
        assert!(!missing_parent(Some(&parent), &child));
        assert!(!missing_parent(Some(&sibling), &parent));
        assert!(missing_parent(Some(&child), &parent));
        assert!(missing_parent(Some(&child), &child));
    }
}
