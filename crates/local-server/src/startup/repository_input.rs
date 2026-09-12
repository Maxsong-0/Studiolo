//! 先验证源码白名单，再打开显式仓库根；非法装配不触发根/配置读取。

use std::sync::Arc;

use studiolo_local_server::ProjectRepository;
use studiolo_workbench::validate_rust_source_paths;

use super::StartupError;
use crate::options::Options;

pub(super) fn open(options: &Options) -> Result<Option<Arc<ProjectRepository>>, StartupError> {
    let sources = options
        .rust_call_sites
        .as_ref()
        .map(|paths| validate_rust_source_paths(paths))
        .transpose()
        .map_err(|_| StartupError::Configuration)?;
    let Some((project, root)) = &options.project_repository else {
        return Ok(None);
    };
    let mut repository =
        ProjectRepository::open(project, root).map_err(|_| StartupError::ReadRoots)?;
    if options.cargo_dependencies {
        repository = repository.with_cargo_dependencies();
    }
    if let Some(sources) = sources {
        repository = repository
            .with_rust_call_sites(sources)
            .map_err(|_| StartupError::Configuration)?;
    }
    Ok(Some(Arc::new(repository)))
}
