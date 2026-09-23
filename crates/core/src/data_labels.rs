//! 源材料的敏感分级、备份与桌面同步范围、模型外发规则，以及经校验、不可拆分的整组标签 `DataLabels`。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::DataLabelError;

/// 源内容的敏感分级。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    /// 公开数据。
    Public,
    /// 内部数据。
    Internal,
    /// 机密数据。
    Confidential,
    /// 受限数据，适用最严格的默认拒绝规则。
    Restricted,
}

/// 源材料中可进入持久备份的范围。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupScope {
    /// 可备份经校验的完整对象。
    Full,
    /// 只可备份脱敏摘要与清单（manifest）。
    SummaryOnly,
    /// 只可备份不含正文的墓碑记录（tombstone）。
    None,
}

/// 源材料中可同步到桌面端的范围。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopSync {
    /// 不同步任何内容。
    Deny,
    /// 只同步脱敏摘要。
    SummaryOnly,
}

/// 源材料能否发送给模型。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelEgress {
    /// 禁止向模型外发。
    Deny,
    /// 只允许经批准的本地模型读取。
    LocalOnly,
    /// 只允许列明的提供方读取。
    AllowProviders(BTreeSet<String>),
}

/// 经校验、由对象继承的分级、留存与外发标签。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DataLabels {
    classification: Classification,
    backup_scope: BackupScope,
    desktop_sync: DesktopSync,
    model_egress: ModelEgress,
}

impl<'de> Deserialize<'de> for DataLabels {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            classification: Classification,
            backup_scope: BackupScope,
            desktop_sync: DesktopSync,
            model_egress: ModelEgress,
        }
        let fields = Fields::deserialize(deserializer)?;
        Self::new(
            fields.classification,
            fields.backup_scope,
            fields.desktop_sync,
            fields.model_egress,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl DataLabels {
    /// 校验并构造一组不可拆分的标签。
    pub fn new(
        classification: Classification,
        backup_scope: BackupScope,
        desktop_sync: DesktopSync,
        model_egress: ModelEgress,
    ) -> Result<Self, DataLabelError> {
        if backup_scope == BackupScope::None && desktop_sync != DesktopSync::Deny {
            return Err(DataLabelError::DesktopOutlivesBackup);
        }
        if matches!(&model_egress, ModelEgress::AllowProviders(providers) if providers.is_empty()) {
            return Err(DataLabelError::EmptyProviderList);
        }
        if classification == Classification::Restricted
            && matches!(model_egress, ModelEgress::AllowProviders(_))
        {
            return Err(DataLabelError::RestrictedProviderEgress);
        }
        Ok(Self {
            classification,
            backup_scope,
            desktop_sync,
            model_egress,
        })
    }

    /// 返回敏感分级。
    #[must_use]
    pub fn classification(&self) -> Classification {
        self.classification
    }

    /// 返回备份范围。
    #[must_use]
    pub fn backup_scope(&self) -> BackupScope {
        self.backup_scope
    }

    /// 返回桌面同步范围。
    #[must_use]
    pub fn desktop_sync(&self) -> DesktopSync {
        self.desktop_sync
    }

    /// 返回模型外发规则。
    #[must_use]
    pub fn model_egress(&self) -> &ModelEgress {
        &self.model_egress
    }
}
