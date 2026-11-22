//! 打开文件数：备份封存的冻结窗口要同时开着写者两棵树里的每个文件（BUG-045，留句柄定格），
//! 配了备份生产者就在启动时把本进程的软上限抬到硬上限。只抬不降；抬不上去就不开始服务。
//! 抬完还不够（文件比上限的一半多）由定格自己失败关闭，不半拍（`vault-broker` 的 `capture.rs`）。
use super::StartupError;
use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

/// 返回抬完之后的软上限（无限时是 `None`）。
pub(super) fn raise_open_files() -> Result<Option<u64>, StartupError> {
    let limit = getrlimit(Resource::Nofile);
    let Some(raised) = raised(&limit) else {
        return Ok(limit.current);
    };
    let current = raised.current;
    setrlimit(Resource::Nofile, raised).map_err(|_| StartupError::Process)?;
    Ok(current)
}

/// 软上限低于有限的硬上限时抬到硬上限；已经是无限、已经等于硬上限，或硬上限是无限（无法给出确数）时不动。
fn raised(limit: &Rlimit) -> Option<Rlimit> {
    match (limit.current, limit.maximum) {
        (Some(current), Some(maximum)) if current < maximum => Some(Rlimit {
            current: Some(maximum),
            maximum: Some(maximum),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Rlimit, raised};

    fn limit(current: Option<u64>, maximum: Option<u64>) -> Rlimit {
        Rlimit { current, maximum }
    }

    #[test]
    fn the_soft_open_file_limit_is_raised_to_a_finite_hard_limit_and_never_lowered() {
        // 169 上人侧单元的样子：软 1024、硬 524288。
        let up = raised(&limit(Some(1024), Some(524_288))).unwrap();
        assert_eq!((up.current, up.maximum), (Some(524_288), Some(524_288)));
        // 已经到顶、软上限无限、硬上限无限：都不动。
        assert!(raised(&limit(Some(524_288), Some(524_288))).is_none());
        assert!(raised(&limit(None, None)).is_none());
        assert!(raised(&limit(Some(1024), None)).is_none());
        // 不会出现的软高于硬：也不动，更不往下压。
        assert!(raised(&limit(Some(2048), Some(1024))).is_none());
    }
}
