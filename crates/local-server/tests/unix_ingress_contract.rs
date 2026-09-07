//! 公开 Unix Interface 的文件系统契约；只使用临时目录和真实节点。

#[path = "unix_ingress/lifecycle.rs"]
mod lifecycle;
#[path = "unix_ingress/paths.rs"]
mod paths;
#[path = "unix_ingress/support.rs"]
mod support;
