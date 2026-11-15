//! 常驻备份 worker。没有配置就退出，不创建默认配置，也不从参数接受仓库或密钥。
use std::{
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};
use studiolo_backup::checkpoint::staging::reclaim_staging;
use studiolo_backup_worker::{
    AppendRunner, ApprovedJob, ProcessError, ResticCredentials, WorkerConfig, read_queue,
};
use studiolo_process_safety::harden;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => reside(),
        Some("reclaim") => reclaim(),
        Some(_) => ExitCode::from(2),
    }
}

fn reside() -> ExitCode {
    let config = match WorkerConfig::load(Instant::now() + Duration::from_secs(30)) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("备份 worker 配置不可用：{error}");
            return ExitCode::from(78);
        }
    };
    let queue = std::path::Path::new("/var/lib/studiolo-backup-worker/queue");
    loop {
        match read_queue(queue) {
            Ok(keys) if keys.is_empty() => thread::sleep(Duration::from_secs(30)),
            Ok(keys) => {
                if let Err(error) = run_keys(&config, &keys) {
                    eprintln!("备份作业未完成：{error}");
                    return ExitCode::from(1);
                }
            }
            Err(error) => {
                eprintln!("备份队列不可用：{error}");
                return ExitCode::from(1);
            }
        }
    }
}

fn run_keys(
    config: &WorkerConfig,
    keys: &[studiolo_backup::worker::state::JobKey],
) -> Result<(), ProcessError> {
    let residency = harden().map_err(|_| ProcessError::Configuration)?;
    for key in keys {
        let deadline = Instant::now() + Duration::from_secs(7200);
        let credentials = ResticCredentials::load(&residency, config, deadline)?;
        let job = ApprovedJob::connect(config, key.clone(), deadline)?;
        let runner = AppendRunner::new(&job, credentials)?;
        runner.run_approved_backup()?;
        let file = std::path::Path::new("/var/lib/studiolo-backup-worker/queue")
            .join(format!("{}.json", key.checkpoint_id.as_str()));
        std::fs::remove_file(file).map_err(|_| ProcessError::Io)?;
    }
    Ok(())
}

fn reclaim() -> ExitCode {
    let config = match WorkerConfig::load(Instant::now() + Duration::from_secs(30)) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("备份回收配置不可用：{error}");
            return ExitCode::from(78);
        }
    };
    match reclaim_once(&config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("备份回收未完成：{error}");
            ExitCode::from(1)
        }
    }
}

fn reclaim_once(config: &WorkerConfig) -> Result<(), ProcessError> {
    reclaim_staging(
        config.sealed_parent(),
        Some(config.reader_gid()),
        Instant::now() + Duration::from_secs(30),
    )
    .map(|_| ())
    .map_err(|_| ProcessError::Io)
}
