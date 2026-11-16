//! 导出目录的来源事实。扫描发生在 Broker，目录组装发生在备份层，两边都不按文件名补完整范围。
use crate::{ArtifactProvenance, Revision, VaultPath};

/// owner 导出里一个文件的固定类别。尝试正文必须另带原批准。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustedExportClass {
    /// 清单第 2 版里的尝试正文或清单本身，以及它们留在历史里的旧版本。
    SourceMaterial,
    /// 已登记逻辑路径的完整范围 L0；来源资料的旧版本不在这一类。
    FullScopeL0,
    /// Control 数据库副本。
    ControlDatabase,
    /// 权威审计副本。
    AuditLedger,
    /// 其余由 owner 声明的必含内容。
    Canonical,
}

/// 一条已经分类的导出对象。`provenance` 为空时不能当成完整来源资料。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedExportFact {
    /// 相对导出根的路径。
    pub export_path: VaultPath,
    /// owner 或原清单给出的类别。
    pub class: TrustedExportClass,
    /// 原批准。只有来源资料（尝试正文、清单和它们的历史版本）才有，别的类别带着它会被整批拒绝。
    pub provenance: Option<ArtifactProvenance>,
    /// 按导出路径和实际字节计算的 revision。
    pub revision: Revision,
}
