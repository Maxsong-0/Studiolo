//! 备份生产者的固定事实：只从 `--backup-config` 指向的私有备份配置（两段）和启动选项里算；
//! 没给这个参数就不启用，给了但文件或条件不齐就拒绝启动。备份两段不放进主人配置：
//! 主人配置整份字节是 Bridge `scope_hash` 的输入，改它会让人侧起不来（T-216）。
//! 这里不建目录、不绑 socket、不碰任何 owner：监听在 `serving.rs` 里和别的入口一起起停。
use super::StartupError;
use crate::options::Options;
use std::{path::PathBuf, sync::Arc};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_backup::checkpoint::manifest::{
    AuditCoverageProof, DeploymentEntry, ManifestInputs, PackageDigest, SecretReference,
    SecretSource,
};
use studiolo_core::checkpoint::FreezeBudget;
use studiolo_host_config::{
    Config,
    backup::{BackupConfig, BackupTargetShape},
};
use studiolo_local_server::{
    backup::{BackupProducer, ProducerPlan},
    checkpoint::CheckpointInputs,
};

/// 从私有备份配置（`--backup-config`）读出来的部分；等写者和各平面装好之后才能配上原实例。
pub(super) struct Approved {
    socket: PathBuf,
    plan: ProducerPlan,
}

/// 装配完成、尚未监听的生产者。
pub(super) struct Prepared {
    pub(super) socket: PathBuf,
    pub(super) producer: Arc<BackupProducer>,
}

/// 给了备份配置才启用，文件里两段必须成对。原生 worker 只接专用的仅追加目标，别的目标形态在这里就拒绝。
pub(super) fn approved(
    config: &Config,
    options: &Options,
) -> Result<Option<Approved>, StartupError> {
    let Some(path) = &options.backup_config else {
        return Ok(None);
    };
    let backup = BackupConfig::load(path, config).map_err(|_| StartupError::Configuration)?;
    let (approval, settings) = (backup.approval(), backup.producer());
    if approval.target_shape() != BackupTargetShape::DedicatedAppendOnlyTarget {
        return Err(StartupError::Configuration);
    }
    let freeze_budget =
        FreezeBudget::new(settings.freeze_budget_millis()).ok_or(StartupError::Configuration)?;
    // 各 owner 正在服务的数据根：暂存层用它们核对封存目录没有落在任何一个里面。
    let mut source_roots = vec![
        options.vault.clone(),
        config.auth_directory().to_owned(),
        config.audit_directory().to_owned(),
    ];
    source_roots.extend(options.writer_data.clone());
    source_roots.extend(options.policy_data.clone());
    source_roots.extend(options.agent_data.clone());
    if let Some(board) = &options.board {
        source_roots.extend([board.data.clone(), board.agent_data.clone()]);
    }
    if source_roots.iter().any(|root| !root.is_absolute()) {
        return Err(StartupError::Configuration);
    }
    let restic = approval.restic();
    let manifest = ManifestInputs {
        application_packages: settings
            .application_packages()
            .iter()
            .map(|package| PackageDigest {
                name: package.name().to_owned(),
                version: package.version().to_owned(),
                digest: package.digest().to_owned(),
            })
            .collect(),
        cli_packages: vec![PackageDigest {
            name: restic.package().to_owned(),
            version: restic.version().to_owned(),
            digest: settings.restic_digest().to_owned(),
        }],
        deployment: settings
            .deployment()
            .iter()
            .map(|entry| DeploymentEntry {
                unit: entry.unit().to_owned(),
                revision: entry.revision().to_owned(),
            })
            .collect(),
        secret_references: approval
            .credentials()
            .iter()
            .map(|reference| SecretReference {
                logical_name: reference.logical_name().to_owned(),
                source: SecretSource::EnvironmentVariable {
                    variable: reference.environment_variable().to_owned(),
                },
            })
            .collect(),
        // 占位：每次封存都由真实核对结果覆盖；万一没被覆盖，这个值本身不成立，清单会被拒绝。
        audit_coverage: AuditCoverageProof {
            acked_events_checked: 0,
            acked_events_matched: 0,
            offset_comparison_only: true,
        },
    };
    Ok(Some(Approved {
        socket: settings.socket().to_owned(),
        plan: ProducerPlan {
            worker_uid: settings.worker_uid(),
            reader_gid: settings.reader_gid(),
            staging_parent: settings.staging_parent().to_owned(),
            ephemeral_root: settings.ephemeral_root().to_owned(),
            source_roots,
            min_interval_millis: approval.schedule().interval_millis(),
            rpo_millis: approval.schedule().rpo_millis(),
            source_retention_millis: settings.source_retention_millis(),
            freeze_budget,
            manifest,
        },
    }))
}

/// 配了生产者就必须有原账本和全部 owner 的原实例；没有写者的只读模式不能做封存。
pub(super) fn prepare(
    approved: Option<Approved>,
    ledger: Option<Arc<SqliteAuditLedger>>,
    inputs: Option<&Arc<CheckpointInputs>>,
) -> Result<Option<Prepared>, StartupError> {
    let Some(Approved { socket, plan }) = approved else {
        return Ok(None);
    };
    let ledger = ledger.ok_or(StartupError::Audit)?;
    let inputs = inputs.ok_or(StartupError::Configuration)?;
    // 封存定格要同时开着写者的每个文件：上限的抬法和定格的额度都归写者（BUG-045）。只写抬完的上限，不写路径。
    let open_files =
        studiolo_vault_broker::raise_open_file_limit().map_err(|_| StartupError::Process)?;
    eprintln!(
        "备份封存可同时打开文件数上限：{}",
        open_files.map_or_else(|| "无限".to_owned(), |limit| limit.to_string())
    );
    Ok(Some(Prepared {
        socket,
        producer: BackupProducer::new(ledger, Arc::clone(inputs), plan),
    }))
}

/// 正在监听的备份 socket；`shutdown` 可以重复调用。
#[cfg(target_os = "linux")]
pub(super) struct Listening(studiolo_local_server::backup::BackupService);

#[cfg(target_os = "linux")]
impl Listening {
    /// 配了就必须起得来：父目录、属组、权限任何一样不符，或者同名节点已在，都算入口错误。
    pub(super) fn start(prepared: Prepared) -> Result<Self, StartupError> {
        studiolo_local_server::backup::BackupService::start(prepared.producer, &prepared.socket)
            .map(Self)
            .map_err(|_| StartupError::Ingress)
    }

    /// 先收监听，再等在途的回收和封存做完；要在停写者之前调用。
    pub(super) async fn shutdown(&self) {
        self.0.shutdown().await;
    }
}

/// 备份 socket 的对端核对只在 Linux 上有；别的平台配了生产者就拒绝启动，不假装在监听。
#[cfg(not(target_os = "linux"))]
pub(super) struct Listening;

#[cfg(not(target_os = "linux"))]
impl Listening {
    pub(super) fn start(_: Prepared) -> Result<Self, StartupError> {
        Err(StartupError::Configuration)
    }

    pub(super) async fn shutdown(&self) {}
}
