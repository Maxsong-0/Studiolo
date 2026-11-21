//! 人侧带备份生产者照生产的装配起来、做完一次真实封存、再停下，登录、审计、策略三个库目录里始终只有
//! 激活快照脚本（`snapshot_worker.py`）认的名字。
//!
//! BUG-040：程序在审计库目录里新建了白名单外的文件，激活在停服之后才被拦下。人侧 r17 起启用备份生产者，
//! 以后每次激活之前都可能已经封存过；这里把「启动和封存都不在三个库目录里留下白名单外的文件」钉成测试。
use super::{
    Fixture, backup_config::approved, checkpoint::policy, snapshot_allowlist::Allowlist,
    test_support, writer::writer_options,
};
use crate::options::FactorOptions;
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::{MetadataExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use studiolo_backup::worker::audit::{
    AdmissionFact, BackupQueueRequest, QueueAnswer, QueueCommand,
};
use studiolo_local_server::backup::{BackupProducer, producer::TARGET};

/// 每一步等待的上限；封存自己的冻结预算是 30 秒。
const WAIT: Duration = Duration::from_secs(60);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backup_startup_and_seal_leave_only_snapshot_allowlisted_names_in_db_dirs() {
    let f = Fixture::new(true);
    let (mut options, _) = writer_options(&f);
    // 照生产人侧装配：因子入口、写者、策略库、接入方身份和黑板。Bridge 在测试里没有受控 launcher，装不上。
    policy(&f, &mut options, true, true);
    options.factor = Some(FactorOptions {
        socket: directory(&f, "proxy", 0o700).join("http.sock"),
        replacement_seed_ref: None,
    });
    options.backup_config = Some(backup_config(&f));
    let directories = Directories::new(&f, options.policy_data.as_deref().unwrap());

    let prepared = super::super::prepare_composed(
        options,
        |_| async { Ok(test_support::verifier().await) },
        || {
            (
                Some("3132333435363738393031323334353637383930".into()),
                None,
            )
        },
    )
    .await
    .unwrap();
    let producer = Arc::clone(
        &prepared
            .backup
            .as_ref()
            .expect("给了备份配置就必须装上生产者")
            .producer,
    );
    let workbench = Arc::clone(prepared.workbench.as_ref().unwrap());
    let assembled = directories.check("装配完成、还没监听");
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let mut server = tokio::spawn(prepared.serve_until(async move {
        let _ = ready_tx.send(());
        let _ = stop_rx.await;
    }));
    // 服务在跑的这一段只收集结果，不 panic：先把服务收掉，再判对错。
    let running = async {
        assembled?;
        match tokio::time::timeout(WAIT, ready_rx).await {
            Ok(Ok(())) => {}
            _ => return Err("服务没在时限内进入关闭等待".to_owned()),
        }
        directories.check("开始服务、封存之前")?;
        seal_once(&producer).await?;
        directories.check("封存完成之后")
    }
    .await;
    let _ = stop_tx.send(());
    let completed = tokio::time::timeout(WAIT, &mut server).await;
    if completed.is_err() {
        server.abort();
        let _ = server.await;
        workbench.stop();
        let _ = tokio::time::timeout(WAIT, workbench.shutdown()).await;
    }
    running.unwrap_or_else(|reason| panic!("{reason}"));
    completed.expect("服务没在时限内停下").unwrap().unwrap();
    directories
        .check("停服之后")
        .unwrap_or_else(|reason| panic!("{reason}"));
    drop(workbench);
    f.unbound();
}

/// 三个库目录，各配上 `snapshot_worker.py` 里的库文件名。
struct Directories {
    allowlist: Allowlist,
    entries: Vec<(PathBuf, String)>,
}

impl Directories {
    fn new(f: &Fixture, policy_data: &Path) -> Self {
        let allowlist = Allowlist::load();
        // 夹具里的目录名和生产上的不同（auth 对 studiolo-human-auth 等），白名单只看库文件名，不影响。
        let entries: Vec<(PathBuf, String)> = allowlist
            .databases
            .iter()
            .map(|database| {
                let directory = match database.as_str() {
                    "auth.sqlite3" => f.root.join("auth"),
                    "audit.sqlite3" => f.root.join("audit"),
                    "policy.sqlite3" => policy_data.to_owned(),
                    other => {
                        panic!("snapshot_worker.py 多了一个库 {other}，这条测试还没有对应的目录")
                    }
                };
                (directory, database.clone())
            })
            .collect();
        assert_eq!(entries.len(), 3, "snapshot_worker.py 管的库不再是这三个");
        Self { allowlist, entries }
    }

    /// 列一次三个目录：库文件本身在（证明目录没找错），别的名字都在白名单里，准入锁是空文件。
    fn check(&self, moment: &str) -> Result<(), String> {
        for (directory, database) in &self.entries {
            let allowed = self.allowlist.names(database);
            let mut seen = BTreeSet::new();
            for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let size = entry.metadata().map_err(|error| error.to_string())?.len();
                if name == self.allowlist.gate() && size != 0 {
                    return Err(format!("{moment}：{database} 的目录里准入锁不是空文件"));
                }
                seen.insert(name);
            }
            if !seen.contains(database) {
                return Err(format!("{moment}：目录里没有 {database}，目录找错了"));
            }
            let stray: Vec<&String> = seen.difference(&allowed).collect();
            if !stray.is_empty() {
                return Err(format!(
                    "{moment}：{database} 的目录里出现激活快照脚本不认的名字 {stray:?}（只认 {allowed:?}）"
                ));
            }
        }
        Ok(())
    }
}

/// 照 worker 的问法安排一次封存、等它做完，再确认真的做成了（原键成了待办）：做失败了也可能什么都不留。
async fn seal_once(producer: &Arc<BackupProducer>) -> Result<(), String> {
    let deadline = Instant::now() + WAIT;
    loop {
        match ask(producer, QueueCommand::Schedule).await? {
            QueueAnswer::Started => break,
            // 监听一起来先回收一次过期暂存，回收线程还在跑时回 outstanding：等它做完再问。
            QueueAnswer::Outstanding if Instant::now() < deadline => {
                settle(producer).await?;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            other => return Err(format!("第一次安排封存应当开始，实际回 {other:?}")),
        }
    }
    settle(producer).await?;
    match ask(producer, QueueCommand::NextPending).await? {
        QueueAnswer::Pending(record) => {
            let fact = AdmissionFact::from_receipt(&record)
                .map_err(|error| format!("待办读不出准入：{error:?}"))?;
            match fact.key().sequence.get() {
                1 => Ok(()),
                other => Err(format!("待办不是这次的原键：序号 {other}")),
            }
        }
        other => Err(format!(
            "封存没有做成：待办应是这次的原键，实际回 {other:?}"
        )),
    }
}

/// 一问一答走生产者的分发入口（socket 那一层要求对端是另一个系统账号，测试里连不上）。
async fn ask(producer: &Arc<BackupProducer>, command: QueueCommand) -> Result<QueueAnswer, String> {
    let producer = Arc::clone(producer);
    tokio::task::spawn_blocking(move || {
        let request = BackupQueueRequest::new(TARGET, command);
        let bytes = request.encode().map_err(|error| format!("{error:?}"))?;
        let reply = producer
            .dispatch(&bytes)
            .map_err(|error| format!("生产者拒答：{error:?}"))?;
        request
            .decode_reply(&reply)
            .map_err(|error| format!("{error:?}"))
    })
    .await
    .map_err(|_| "问生产者的线程没回来".to_owned())?
}

/// 等生产者的后台线程（回收、封存）做完。
async fn settle(producer: &Arc<BackupProducer>) -> Result<(), String> {
    let producer = Arc::clone(producer);
    let waiting = tokio::task::spawn_blocking(move || producer.settle());
    match tokio::time::timeout(WAIT, waiting).await {
        Ok(Ok(())) => Ok(()),
        _ => Err("生产者的后台线程没在时限内做完".to_owned()),
    }
}

/// 照生产写一份备份配置（0600）：读取组用本账号的组；封存父目录和 socket 父目录照暂存层、监听层的要求
/// 建成 0750、属组是读取组。目录名和 `approved` 里写的一致。
fn backup_config(f: &Fixture) -> PathBuf {
    let staging = directory(f, "backup-staging", 0o750);
    let reader_gid = fs::metadata(&staging).unwrap().gid();
    assert_ne!(reader_gid, 0, "读取组不许是 0：这条测试要在非 root 组下跑");
    directory(f, "ephemeral", 0o700);
    let socket = directory(f, "bsock", 0o750).join("p.sock");
    let mut value: serde_json::Value = serde_json::from_slice(&approved(&f.root)).unwrap();
    value["backup_producer"]["reader_gid"] = reader_gid.into();
    value["backup_producer"]["socket"] = serde_json::json!(socket);
    let path = f.root.join("config/backup.json");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    path
}

fn directory(f: &Fixture, name: &str, mode: u32) -> PathBuf {
    let path = f.root.join(name);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    path
}
