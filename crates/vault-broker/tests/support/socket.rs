//! 在夹具根内绑定 Unix socket。能否建成只取决于路径是否短于 `sun_path` 容量：
//! 可判定能建成时必须建成，否则打印跳过原因；两种情况都打印判据输入与实际结果。

use std::{os::unix::net::UnixListener, path::Path};

/// Unix socket 地址 `sun_path` 的字节容量（含结尾 NUL）；路径须严格短于它才能绑定。
#[cfg(target_os = "linux")]
pub const SUN_PATH_CAPACITY: usize = 108;
#[cfg(not(target_os = "linux"))]
pub const SUN_PATH_CAPACITY: usize = 104;

/// 返回已绑定的监听端；路径过长时返回 None，调用方据此选择断言分支。
pub fn bind_fixture(path: &Path) -> Option<UnixListener> {
    let bytes = path.as_os_str().len();
    let result = UnixListener::bind(path);
    eprintln!(
        "socket 夹具·分支自证：路径 {bytes} 字节，sun_path 容量 {SUN_PATH_CAPACITY}，bind = {:?}",
        result.as_ref().map(|_| ())
    );
    if bytes < SUN_PATH_CAPACITY {
        return Some(result.expect("路径短于 sun_path 容量时 socket 夹具必须建成"));
    }
    eprintln!("跳过：临时目录路径 {bytes} 字节已达 sun_path 容量，无法在根内绑定 socket 夹具");
    assert!(
        matches!(&result, Err(error) if error.kind() == std::io::ErrorKind::InvalidInput),
        "{result:?}"
    );
    None
}
