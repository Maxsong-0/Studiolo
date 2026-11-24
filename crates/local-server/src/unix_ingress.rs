//! 同 UID Unix 入口；权限边界不识别具体可执行文件，也不对抗同 UID 或管理员。

mod layout;
pub(crate) mod leftover;
mod peer;

pub(crate) use peer::{TrustedNetworkSource, source};

use crate::CanonicalWorkbench;
use axum::Router;
use std::{future::Future, os::unix::net::UnixListener, path::Path};

/// Unix 入口拒绝配置、变化的文件系统或不可信请求；说明不含主机路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IngressError {
    /// 路径、权限或对象身份不符合入口约定。
    #[error("Unix 入口路径或权限无效")]
    Configuration,
    /// 监听或连接状态不可用。
    #[error("Unix 入口暂不可用")]
    Unavailable,
    /// 未获真实 Unix 同 UID 凭据，或请求来源头并非唯一有效值。
    #[error("请求来源不可验证")]
    UntrustedSource,
}

/// 只读准备结果；不创建目录、socket，也不清除任何已有节点。
pub struct PreparedUnixIngress {
    layout: layout::Layout,
}

impl PreparedUnixIngress {
    /// 要求绝对规范路径、已有同 UID 的 0700 父目录，且与给定根互不嵌套。
    /// 路径最多 103 个 UTF-8 字节，以同时适配 macOS 和 Linux 的 pathname socket。
    pub fn prepare(path: &Path, isolated_roots: &[&Path]) -> Result<Self, IngressError> {
        Ok(Self {
            layout: layout::Layout::prepare(path, isolated_roots, None)?,
        })
    }

    /// 同 [`PreparedUnixIngress::prepare`]，但容得下上一任被强杀后留下、已经没人监听的 socket：
    /// 这里仍然只看不动，删它要等 [`PreparedUnixIngress::bind_over_leftover`] 拿到主库锁的凭据。
    /// `label` 是日志里的叫法，不带路径。
    pub fn prepare_over_leftover(
        path: &Path,
        isolated_roots: &[&Path],
        label: &'static str,
    ) -> Result<Self, IngressError> {
        Ok(Self {
            layout: layout::Layout::prepare(path, isolated_roots, Some(label))?,
        })
    }

    /// 重验完整目录身份、权限及目标不存在（或是容得下的残留）；不自动修复变化。
    pub fn revalidate(&self) -> Result<(), IngressError> {
        self.layout.revalidate()
    }

    /// 先接管判死的残留，再照 [`PreparedUnixIngress::bind`] 建监听；没按容得下残留准备的，有节点照旧拒绝。
    pub fn bind_over_leftover(
        self,
        workbench: &CanonicalWorkbench,
    ) -> Result<BoundUnixIngress, IngressError> {
        self.revalidate()?;
        if let Some(entrance) = self.layout.entrance() {
            entrance
                .take_over(workbench)
                .map_err(|_| IngressError::Configuration)?;
        }
        self.bind()
    }

    /// 建立监听并复核 0600、UID、类型和 inode；不需要 Tokio runtime。已有任何同名节点都拒绝。
    pub fn bind(self) -> Result<BoundUnixIngress, IngressError> {
        self.layout.vacant()?;
        let listener =
            UnixListener::bind(self.layout.path()).map_err(|_| IngressError::Unavailable)?;
        let cleanup = self.layout.claim()?;
        listener
            .set_nonblocking(true)
            .map_err(|_| IngressError::Unavailable)?;
        Ok(BoundUnixIngress { listener, cleanup })
    }
}

/// 监听与本次 socket 的清理责任；销毁时仅清除仍匹配的本次 inode。
pub struct BoundUnixIngress {
    listener: UnixListener,
    cleanup: layout::Cleanup,
}

impl BoundUnixIngress {
    /// 在真实 Unix 连接上安装私有来源凭据。
    pub async fn serve(self, app: Router) -> Result<(), IngressError> {
        self.serve_shutdown(app, std::future::pending()).await
    }

    /// 等待调用者给出的退出信号，再排空连接；取消 future 也保留安全清理。
    pub async fn serve_shutdown<F>(self, app: Router, shutdown: F) -> Result<(), IngressError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let Self { listener, cleanup } = self;
        cleanup.revalidate()?;
        let listener =
            tokio::net::UnixListener::from_std(listener).map_err(|_| IngressError::Unavailable)?;
        let result = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<peer::UnixPeer>(),
        )
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|_| IngressError::Unavailable);
        drop(cleanup);
        result
    }
}

#[cfg(test)]
#[path = "../tests/unix_ingress/source.rs"]
mod source_tests;
