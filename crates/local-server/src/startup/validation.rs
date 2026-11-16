//! 启动参数的交叉约束；纯校验，不读取秘密、不打开存储。
use super::StartupError;
use crate::options::Options;

pub(super) fn options(options: &Options) -> Result<(), StartupError> {
    if options.port == 0
        || options.label.trim().is_empty()
        || options.label.chars().count() > 80
        || options.label.chars().any(char::is_control)
        || (options.session.is_some() && options.access.is_some())
        || (options.canonical && options.session.is_none())
        || (options.factor.is_some() && options.session.is_none())
        || (options.project_repository.is_some() && options.session.is_some())
        || (options.cargo_dependencies && options.project_repository.is_none())
        || (options.rust_call_sites.is_some() && options.project_repository.is_none())
        || (options.writer_data.is_some() && (options.canonical || options.session.is_none()))
        || options
            .writer_data
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
        || options
            .policy_data
            .as_ref()
            .is_some_and(|path| !path.is_absolute() || options.session.is_none())
        || options.agent_data.as_ref().is_some_and(|path| {
            !path.is_absolute()
                || options.policy_data.is_none()
                || options
                    .board
                    .as_ref()
                    .is_some_and(|board| board.agent_data != *path)
        })
        || options.runner_config.as_ref().is_some_and(|path| {
            !path.is_absolute() || options.board.is_none() || options.factor.is_none()
        })
    {
        return Err(StartupError::Configuration);
    }
    Ok(())
}
