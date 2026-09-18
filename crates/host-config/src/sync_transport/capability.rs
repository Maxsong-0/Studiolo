//! 封闭的传输能力：只能由已批准 SyncScope 加显式审批文档取得。
//!
//! 本类型既不启动进程，也不证明受控 Linux 隔离已经存在；它只是「主人已批准这些值」。

use super::document::SyncTransportApproval;
use super::types::{TransportBackend, TransportBudget, TransportRoots, TransportRuntime};
use crate::{ConfigError, SyncBinding, SyncScope, config_file};
use std::path::Path;

/// 固定 link 的传输运行批准；没有公开构造器、公开字段或反序列化入口。
///
/// 借用已批准的 `SyncScope`：每次使用前后都复验同一份私有配置快照，
/// 配置换内容、换对象或范围代次变化都会让本能力立即失效。
///
/// ```compile_fail
/// let approved: studiolo_host_config::sync_transport::ApprovedTransport =
///     serde_json::from_str("{}").unwrap();
/// ```
pub struct ApprovedTransport<'scope> {
    scope: &'scope SyncScope,
    approval: SyncTransportApproval,
    binding: SyncBinding,
    roots: TransportRoots,
}

impl<'scope> ApprovedTransport<'scope> {
    /// 在已批准范围之上追加运行审批。
    ///
    /// `reserved_roots` 是调用方必须传入的其它权威根（canonical、writer-data、
    /// Auth/Audit、Bridge DB、Headless config、外 stage 等）；传输私有根与镜像根
    /// 必须与它们以及彼此两两不重叠。传空切片不代表已证明不重叠，只代表调用方没有申报。
    pub fn approve(
        scope: &'scope SyncScope,
        approval: SyncTransportApproval,
        reserved_roots: &[&Path],
    ) -> Result<Self, ConfigError> {
        scope.revalidate()?;
        let roots = pin_roots(&approval, reserved_roots)?;
        let approved = Self {
            scope,
            binding: scope.binding().clone(),
            approval,
            roots,
        };
        approved.revalidate()?;
        Ok(approved)
    }

    /// 复验私有配置快照、绑定事实与两个根的真实对象身份。
    ///
    /// 成功只代表检查时刻一致；不能证明与后续 IO 的系统级原子性。
    pub fn revalidate(&self) -> Result<(), ConfigError> {
        self.scope.revalidate()?;
        if self.scope.binding() != &self.binding {
            return Err(ConfigError);
        }
        let private = config_file::private_directory(self.approval.private_root())?;
        let mirror = config_file::private_directory(self.approval.mirror_root())?;
        if private != self.roots.private_identity() || mirror != self.roots.mirror_identity() {
            return Err(ConfigError);
        }
        Ok(())
    }

    /// 可公开比较的绑定事实；它是请求绑定，不是认证。
    #[must_use]
    pub fn binding(&self) -> &SyncBinding {
        &self.binding
    }

    /// 传输私有根与镜像根及其对象身份。
    #[must_use]
    pub fn roots(&self) -> &TransportRoots {
        &self.roots
    }

    /// 固定运行时事实。
    #[must_use]
    pub fn runtime(&self) -> &TransportRuntime {
        self.approval.parts().0
    }

    /// 受控 Linux 后端的批准期望。
    #[must_use]
    pub fn backend(&self) -> &TransportBackend {
        self.approval.parts().1
    }

    /// 运行预算。
    #[must_use]
    pub fn budget(&self) -> TransportBudget {
        self.approval.parts().2
    }

    /// 批准配置摘要；进入持久绑定，用于拒绝换配置后的旧运行槽。
    #[must_use]
    pub fn approval_digest(&self) -> [u8; 32] {
        self.approval.digest()
    }
}

fn pin_roots(
    approval: &SyncTransportApproval,
    reserved_roots: &[&Path],
) -> Result<TransportRoots, ConfigError> {
    let private_path = approval.private_root();
    let mirror_path = approval.mirror_root();
    // 私有根必须是 0700、本人所有的真实目录；逐段拒绝链接与非目录。
    let private = config_file::private_directory(private_path)?;
    let mirror = config_file::private_directory(mirror_path)?;
    if private == mirror {
        return Err(ConfigError);
    }
    config_file::disjoint(private_path, mirror_path)?;
    for reserved in reserved_roots {
        config_file::disjoint(private_path, reserved)?;
        config_file::disjoint(mirror_path, reserved)?;
    }
    Ok(TransportRoots::new(
        private_path.to_path_buf(),
        private,
        mirror_path.to_path_buf(),
        mirror,
    ))
}
