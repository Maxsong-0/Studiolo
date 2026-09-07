//! 只收紧当前短命登记进程的限制，不改变父进程、服务或系统配置。

use crate::AdminError;
use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

pub(crate) fn disable_core_dumps() -> Result<(), AdminError> {
    let disabled = Rlimit {
        current: Some(0),
        maximum: Some(0),
    };
    setrlimit(Resource::Core, disabled).map_err(|_| AdminError::Process)?;
    let actual = getrlimit(Resource::Core);
    if actual.current != Some(0) || actual.maximum != Some(0) {
        return Err(AdminError::Process);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_limit_is_closed_before_secret_handling() {
        // 只影响此独立CLI测试子进程，不修改cargo/父shell；重复收紧必须幂等。
        disable_core_dumps().unwrap();
        disable_core_dumps().unwrap();
        let limit = getrlimit(Resource::Core);
        assert_eq!(limit.current, Some(0));
        assert_eq!(limit.maximum, Some(0));
    }
}
