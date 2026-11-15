//! 生产者配置、到期判断，以及错误 peer 不能进入已批准作业。
use std::{
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};
use studiolo_local_server::backup::producer::{
    BackupSchedule, ProducerConfig, ProducerError, ProducerListener,
};

#[test]
fn fixed_config_rejects_missing_file_same_uid_and_secret_fields() {
    assert!(matches!(
        ProducerConfig::load(),
        Err(ProducerError::Configuration)
    ));
    let uid = rustix::process::geteuid().as_raw();
    let ok = format!(
        r#"{{"schema":1,"socket_path":"/run/studiolo/backup-producer.sock","worker_uid":{},"interval_seconds":600}}"#,
        uid.wrapping_add(1).max(1)
    );
    assert!(ProducerConfig::parse(ok.as_bytes(), uid).is_ok());
    assert!(ProducerConfig::parse(ok.as_bytes(), 0).is_err() || uid != 0);
    let same = format!(
        r#"{{"schema":1,"socket_path":"/run/studiolo/backup-producer.sock","worker_uid":{uid},"interval_seconds":600}}"#
    );
    assert!(ProducerConfig::parse(same.as_bytes(), uid).is_err());
    let secret = format!(
        r#"{{"schema":1,"socket_path":"/run/studiolo/backup-producer.sock","worker_uid":{},"interval_seconds":600,"password":"no"}}"#,
        uid.wrapping_add(1).max(1)
    );
    assert!(ProducerConfig::parse(secret.as_bytes(), uid).is_err());
    let relative = format!(
        r#"{{"schema":1,"socket_path":"relative.sock","worker_uid":{},"interval_seconds":600}}"#,
        uid.wrapping_add(1).max(1)
    );
    assert!(ProducerConfig::parse(relative.as_bytes(), uid).is_err());
}

#[test]
fn schedule_is_due_until_a_job_starts_and_not_before_the_interval() {
    let start = Instant::now();
    let mut schedule = BackupSchedule::new(Duration::from_secs(60));
    assert!(schedule.due(start));
    schedule.mark_started(start);
    assert!(!schedule.due(start + Duration::from_secs(59)));
    assert!(schedule.due(start + Duration::from_secs(60)));
}

#[test]
fn listener_rejects_a_peer_that_is_not_the_configured_worker() {
    let socket = std::path::PathBuf::from(format!("/tmp/studiolo-bp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let uid = rustix::process::geteuid().as_raw();
    let config = ProducerConfig::parse(
        format!(
            r#"{{"schema":1,"socket_path":"{}","worker_uid":{},"interval_seconds":60}}"#,
            socket.display(),
            uid.wrapping_add(1).max(1)
        )
        .as_bytes(),
        uid,
    )
    .unwrap();
    let listener = ProducerListener::bind(&config).unwrap();
    let _client = UnixStream::connect(&socket).unwrap();
    assert!(matches!(
        listener.accept_worker(Instant::now() + Duration::from_secs(2)),
        Err(ProducerError::Peer)
    ));
    drop(listener);
    let _ = std::fs::remove_file(&socket);
}
