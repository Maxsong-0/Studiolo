//! 打开文件数（BUG-045）：封存的冻结窗口要同时开着两棵树里的每个文件（留句柄定格），
//! 能开多少由本进程的软上限定（[`super::capture`] 的 `Budget`，取一半）。和它放在一处的是启动时的那一步：
//! 把软上限抬到硬上限。只抬不降；抬完还不够（文件比上限的一半多）由定格自己失败关闭，不半拍。
//! 这里只调大打开文件数，不收紧秘密驻留面（那一份实现只在 `studiolo-process-safety`）。
use crate::BrokerError;
use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

/// 把本进程打开文件数的软上限抬到硬上限，返回抬完之后的软上限（无限时是 `None`）。
/// 人侧配了备份生产者时在启动时调一次；抬不上去就不开始服务。
///
/// # 错误
///
/// 内核拒绝新的软上限时返回 [`BrokerError::Io`]。
pub fn raise_open_file_limit() -> Result<Option<u64>, BrokerError> {
    let limit = getrlimit(Resource::Nofile);
    let Some(raised) = raised(&limit) else {
        return Ok(limit.current);
    };
    let current = raised.current;
    setrlimit(Resource::Nofile, raised)
        .map_err(|e| BrokerError::io("抬高打开文件数软上限", e.into()))?;
    Ok(current)
}

/// 软上限低于有限的硬上限时抬到硬上限；已经是无限、已经等于硬上限，或硬上限是无限（无法给出确数）时不动。
/// Linux 上打开文件数的硬上限不会是无限：内核拒绝设成大于 `fs.nr_open` 的值（`prlimit` 返回 EPERM），
/// 无限比它大。所以「软有限、硬无限」只在别的系统上出现，那里不运行人侧；真出现了照旧不抬，
/// 定格按软上限的一半失败关闭、日志写「超过固定上限」。
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
