//! 测试夹具显式授予的完整来源。生产路径不得调用，也不能当成缺字段时的默认值。
use studiolo_core::{
    ArtifactProvenance, BackupScope, Classification, DataLabels, DesktopSync, ModelEgress,
    SourcePolicyRef,
};

/// 显式授予的完整来源。调用点必须写出来，缺字段的旧记录不能走这里补默认值。
pub fn granted_provenance() -> ArtifactProvenance {
    ArtifactProvenance::new(
        SourcePolicyRef::new("fixture-source", "a".repeat(64)).unwrap(),
        DataLabels::new(
            Classification::Restricted,
            BackupScope::Full,
            DesktopSync::Deny,
            ModelEgress::Deny,
        )
        .unwrap(),
        4_000_000_000,
        "b".repeat(64),
    )
    .unwrap()
}
