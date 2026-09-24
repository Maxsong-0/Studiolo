//! 封闭的传输能力：只能由已批准 SyncScope 加显式审批文档取得。
//!
//! 本类型既不启动进程，也不证明受控 Linux 隔离已经存在；它只是「主人已批准这些值」。

use super::document::SyncTransportApproval;
use super::types::{TransportBackend, TransportBudget, TransportRoots, TransportRuntime};
use crate::config_file::PinnedDirectory;
use crate::{ConfigError, SyncBinding, SyncScope, config_file};
use std::path::Path;

/// 固定 link 的传输运行批准；没有公开构造器、公开字段或反序列化入口。
///
/// 借用已批准的 `SyncScope`：每次使用前后都复验同一份私有配置快照，
/// 配置换内容、换对象或范围代次变化都会让本能力立即失效。
///
/// 两个根各持有一个打开的目录 FD。只记 `(dev, ino)` 不够：Linux 把目录 rmdir 后
/// 同名 mkdir 会立刻发还同一个 inode 号，取值比较看不出替换；持有 FD 后既能看到
/// 旧 inode 的链接数归零，也让那个 inode 号在 FD 释放前发不出去，两条都指向漂移。
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
    private_pin: PinnedDirectory,
    /// 钉的是 owner 视图（idmapped bind），不是 payload 身份的真实镜像根。
    view_pin: PinnedDirectory,
    plane_pin: PinnedDirectory,
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
        let (roots, private_pin, view_pin, plane_pin) = pin_roots(&approval, reserved_roots)?;
        let approved = Self {
            scope,
            binding: scope.binding().clone(),
            approval,
            roots,
            private_pin,
            view_pin,
            plane_pin,
        };
        approved.revalidate()?;
        Ok(approved)
    }

    /// 复验私有配置快照、绑定事实与两个根的真实对象身份。
    ///
    /// 根身份走持有的 FD 复验：目录被删掉重建时，即使 inode 号原样发还，
    /// 持有的那个旧 inode 的链接数也已经归零，替换藏不住。
    ///
    /// 成功只代表检查时刻一致；不能证明与后续 IO 的系统级原子性。
    pub fn revalidate(&self) -> Result<(), ConfigError> {
        self.scope.revalidate()?;
        if self.scope.binding() != &self.binding {
            return Err(ConfigError);
        }
        self.private_pin.revalidate()?;
        self.view_pin.revalidate()?;
        self.plane_pin.revalidate()?;
        // 对外报告的取值必须始终等于钉住的对象，别让投影和真实钉悄悄分家。
        if self.private_pin.identity() != self.roots.private_identity()
            || self.view_pin.identity() != self.roots.mirror_identity()
            || self.plane_pin.identity() != self.roots.plane_identity()
        {
            return Err(ConfigError);
        }
        Ok(())
    }

    /// 镜像节点设备名。
    #[must_use]
    pub fn device_name(&self) -> &str {
        self.approval.device_name()
    }

    /// 审批的镜像同步模式。
    #[must_use]
    pub fn headless_mode(&self) -> super::TransportSyncMode {
        self.approval.headless_mode()
    }

    /// 审批的排除文件夹（已排序、去重）。
    #[must_use]
    pub fn excluded_folders(&self) -> &[String] {
        self.approval.excluded_folders()
    }

    /// 人工录入的账号配额。
    #[must_use]
    pub fn manual_quota(&self) -> Option<super::ManualQuota> {
        self.approval.manual_quota()
    }

    /// 可公开比较的绑定事实；它是请求绑定，不是认证。
    #[must_use]
    pub fn binding(&self) -> &SyncBinding {
        &self.binding
    }

    /// 借用的已批准整文件范围；驱动平面用它导出观察选择与批准完整联合路径。
    ///
    /// 只是借用回去，不新增任何根、写者或反序列化入口；范围自身每次使用前后仍复验配置。
    #[must_use]
    pub fn scope(&self) -> &'scope SyncScope {
        self.scope
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

type PinnedRoots = (
    TransportRoots,
    PinnedDirectory,
    PinnedDirectory,
    PinnedDirectory,
);

fn pin_roots(
    approval: &SyncTransportApproval,
    reserved_roots: &[&Path],
) -> Result<PinnedRoots, ConfigError> {
    let private_path = approval.private_root();
    let view_path = approval.mirror_view_root();
    let plane_path = approval.plane_root();
    let mirror_path = approval.mirror_root();
    // 私有根、视图根、平面根都必须是 0700、本人所有的真实目录；逐段拒绝链接与非目录。
    // 核验通过后立刻持有它的 FD：整条能力的生命期内都不松手，替换才藏不住。
    // 真实镜像根属 payload 身份，owner 无权遍历，这里只核对它不与任何自有根同名。
    let private = PinnedDirectory::pin(private_path)?;
    let view = PinnedDirectory::pin(view_path)?;
    let plane = PinnedDirectory::pin(plane_path)?;
    let identities = [private.identity(), view.identity(), plane.identity()];
    if (1..identities.len()).any(|i| identities[..i].contains(&identities[i])) {
        return Err(ConfigError);
    }
    let owned = [private_path, view_path, plane_path];
    for (i, left) in owned.iter().enumerate() {
        for right in &owned[i + 1..] {
            config_file::disjoint(left, right)?;
        }
        if mirror_path.starts_with(left) || left.starts_with(mirror_path) {
            return Err(ConfigError);
        }
        for reserved in reserved_roots {
            config_file::disjoint(left, reserved)?;
        }
    }
    let roots = TransportRoots::new(
        private_path.to_path_buf(),
        private.identity(),
        mirror_path.to_path_buf(),
        view.identity(),
        view_path.to_path_buf(),
        plane_path.to_path_buf(),
        plane.identity(),
    );
    Ok((roots, private, view, plane))
}
