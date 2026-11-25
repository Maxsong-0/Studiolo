//! 写者旧记录的自动打包（BUG-051）：只在不封存的那一轮、用量到一半、有过一次成功封存时打；打完照常封存。
use super::{producer::Stage, *};
use crate::{
    backup::producer::TARGET, coordinator::archive::CachedUsage, health::RecordUsageProbe as _,
};
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};
use studiolo_backup::worker::{
    audit::{AdmissionFact, QueueAnswer, QueueCommand},
    job::BACKUP_AUDIT_ACTION,
};
use studiolo_core::VaultPath;
use studiolo_vault_broker::{
    FsVaultBroker, Mutation, MutationActor, MutationKind, MutationMetadata, RecordUsage,
    RecordUsageItem,
};

const DAY: i64 = 24 * 60 * 60 * 1000;

impl Stage {
    /// 最满的一处摆成用了这么多（上限 4096），跑一轮只回收不封存的，交回这一轮打的行。到 2048 是一半，到 3277 是八成。
    async fn archive_round(&self, used: u64) -> Vec<String> {
        // 先让在途的重扫走完，免得它把下面摆的用量量回去。
        self.assembled.workbench.refresh().await.unwrap();
        self.pretend(used);
        self.round()
    }
    /// 刚量过，最满的一处用了这么多。
    fn pretend(&self, used: u64) {
        let empty = RecordUsageItem { used: 0, limit: 1 };
        *self.assembled.workbench.core.usage.lock().unwrap() = CachedUsage::at(
            RecordUsage {
                descriptors: RecordUsageItem { used, limit: 4_096 },
                descriptor_bytes: empty,
                replays: empty,
                replay_bytes: empty,
                file_operations: empty,
                file_operation_bytes: empty,
                ..RecordUsage::default()
            },
            Instant::now(),
        );
    }
    /// 现在就跑一轮，用量不动。
    fn round(&self) -> Vec<String> {
        self.producer.seal_lines.lock().unwrap().clear();
        self.producer.reclaim();
        self.producer.settle();
        self.producer.seal_lines.lock().unwrap().clone()
    }
    fn data(&self) -> std::path::PathBuf {
        self.assembled.fixture.root.join("data")
    }
    /// 另写这几篇，回执全落账。
    fn written(&self, range: std::ops::Range<usize>) {
        let mut owner = self.unflushed(range);
        flush_content(owner.as_mut().unwrap(), &self.assembled.human, 0);
    }
    /// 另写这几篇，回执还没落账。
    fn unflushed(
        &self,
        range: std::ops::Range<usize>,
    ) -> std::sync::MutexGuard<'_, Option<FsVaultBroker>> {
        let mut owner = self.assembled.workbench.core.owner.lock().unwrap();
        let broker = owner.as_mut().unwrap();
        for index in range {
            broker
                .mutate_recorded(
                    Mutation::create(
                        VaultPath::parse(format!("more-{index}.md")).unwrap(),
                        format!("正文 {index}").into_bytes(),
                        format!("more-{index}"),
                    ),
                    MutationMetadata::new(MutationActor::Obsidian, MutationKind::Replace).unwrap(),
                )
                .unwrap();
        }
        owner
    }
    /// 健康页现在读到的用量。
    fn usage(&self) -> RecordUsage {
        self.assembled.workbench.record_usage().unwrap()
    }
    /// 封存一次；`verified` 时由 worker 写下验过的成功结果。
    fn sealed(&self, now_ms: i64, verified: bool) {
        assert_eq!(self.schedule_at(now_ms), QueueAnswer::Started);
        self.producer.settle();
        let QueueAnswer::Pending(record) = self.ask(QueueCommand::NextPending) else {
            panic!("封存完成后必须有待办：{:?}", self.last_failure());
        };
        let key = AdmissionFact::from_receipt(&record).unwrap().key().clone();
        assert_eq!(
            AdmissionFact::from_receipt(&record).unwrap().target(),
            TARGET
        );
        let body = serde_json::json!({"result":"succeeded","objects_complete":true,
            "checksum_verified":verified,"paired_checkpoint_reference":true,
            "job_sequence":key.sequence.get(),"checkpoint_id":key.checkpoint_id});
        self.assembled
            .human
            .append(AuditEvent::new(
                AuditEventId::parse(format!(
                    "backup.snapshot:{}:{}",
                    key.sequence,
                    key.checkpoint_id.as_str()
                ))
                .unwrap(),
                AuditPhase::Outcome,
                BACKUP_AUDIT_ACTION,
                SanitizedPayload::try_from(body).unwrap(),
            ))
            .unwrap();
        // 让队列读到这条结果。
        assert_eq!(self.ask(QueueCommand::NextPending), QueueAnswer::Idle);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_records_are_packed_only_after_a_successful_seal_and_the_next_seal_still_works() {
    let stage = Stage::new(true).await;
    let now = super::steps::now_millis();
    *stage.producer.archive_clock.lock().unwrap() = Some(now + 8 * DAY);
    let archive = stage.data().join("archive-v1");
    stage.written(0..39);
    // 还没有一次成功封存：用量到了也不打。
    assert!(stage.archive_round(2_048).await.is_empty());
    stage.sealed(now, false);
    assert!(
        stage.archive_round(2_048).await.is_empty(),
        "没验过不算成功"
    );
    stage.sealed(now + DAY, true);
    // 用量没到一半：不打，写者状态的格式不会自己变。
    assert!(stage.archive_round(2_047).await.is_empty());
    assert!(!archive.exists());
    let lines = stage.archive_round(2_048).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("写者旧记录打包：40 条，"), "{lines:?}");
    assert_eq!(fs::read_dir(&archive).unwrap().count(), 1);
    assert_eq!(
        fs::read_dir(stage.data().join("history/meta-v1/requests"))
            .unwrap()
            .count(),
        0
    );
    // 打完量过一次，健康页读到的是打包之后的用量。
    assert_eq!(
        (stage.usage().packs, stage.usage().descriptors.used),
        (40, 0)
    );
    // 打了包的照常封存、逐条核回执；业务照常读写。
    stage.sealed(now + 2 * DAY, true);
    stage.assembled.writable(true);
    // 再来一轮没有可打的：不打行、不多出段。
    assert!(stage.archive_round(2_048).await.is_empty());
    assert_eq!(fs::read_dir(&archive).unwrap().count(), 1);
    // 看完的一轮之后从头看：后来的够一段照样打。
    stage.written(39..79);
    stage.sealed(now + 3 * DAY, true);
    let lines = stage.archive_round(2_048).await;
    assert!(lines[0].starts_with("写者旧记录打包：40 条，"), "{lines:?}");
    // 有没投递完的审计：不是故障，不打行，下一轮再来。
    drop(stage.unflushed(79..80));
    stage.pretend(2_048);
    assert!(stage.round().is_empty());
    assert_eq!(fs::read_dir(&archive).unwrap().count(), 2);
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 凑不够半段：到一半用量不打，到八成才放小段。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_small_segment_waits_until_the_store_is_four_fifths_full() {
    let stage = Stage::new(true).await;
    let now = super::steps::now_millis();
    *stage.producer.archive_clock.lock().unwrap() = Some(now + 8 * DAY);
    stage.written(0..7);
    stage.sealed(now, true);
    assert!(stage.archive_round(3_276).await.is_empty());
    assert!(!stage.data().join("archive-v1").exists());
    let lines = stage.archive_round(3_277).await;
    assert!(lines[0].starts_with("写者旧记录打包：8 条，"), "{lines:?}");
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 一轮时间到了就停：攒下的、看到哪都留着，下一轮接着来，不从头。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_round_cut_short_goes_on_where_it_stopped() {
    let stage = Stage::new(true).await;
    let now = super::steps::now_millis();
    *stage.producer.archive_clock.lock().unwrap() = Some(now + 8 * DAY);
    *stage.producer.archive_budget.lock().unwrap() = Duration::ZERO;
    stage.written(0..39);
    stage.sealed(now, true);
    // 一轮只走得了一步：这一步只挑。
    assert!(stage.archive_round(2_048).await.is_empty());
    assert!(!stage.data().join("archive-v1").exists());
    // 下一轮接着写。
    let lines = stage.archive_round(2_048).await;
    assert!(lines[0].starts_with("写者旧记录打包：40 条，"), "{lines:?}");
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 关停叫停之后不再开始新的一步，也不打行。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_down_stops_packing_before_the_next_step() {
    let stage = Stage::new(true).await;
    let now = super::steps::now_millis();
    *stage.producer.archive_clock.lock().unwrap() = Some(now + 8 * DAY);
    stage.written(0..39);
    stage.sealed(now, true);
    let orphan = plant_orphan(&stage.data());
    stage.producer.settle_window();
    assert!(stage.archive_round(2_048).await.is_empty());
    assert!(orphan.exists());
    // 收口之后、打包之前才叫停的也一样：一步都不走。
    fs::remove_dir_all(stage.data().join("archive-v1")).unwrap();
    stage.producer.archive_records(Some(u64::MAX));
    assert!(!stage.data().join("archive-v1").exists());
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 写者已经不在（工作台先关了）：拿不到就是这一轮不做，不是故障，不打「没做完」。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_writer_that_is_gone_is_not_reported_as_a_failure() {
    let stage = Stage::new(true).await;
    let now = super::steps::now_millis();
    *stage.producer.archive_clock.lock().unwrap() = Some(now + 8 * DAY);
    stage.written(0..39);
    stage.sealed(now, true);
    stage.assembled.workbench.shutdown().await.unwrap();
    stage.pretend(2_048);
    assert!(stage.round().is_empty());
    assert!(!stage.data().join("archive-v1").exists());
}

/// 用量还新就不重量（量一次要在写者锁里数四个目录）；放久了、打过包才量。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn usage_is_measured_again_only_when_it_is_stale() {
    let stage = Stage::new(true).await;
    let workbench = &stage.assembled.workbench;
    workbench.refresh().await.unwrap();
    // 第一次重扫就量过，健康页读的就是它。
    assert_eq!(stage.usage().replays.limit, 4_096);
    stage.pretend(5);
    workbench.refresh().await.unwrap();
    assert_eq!(stage.usage().descriptors.used, 5);
    let pretended = stage.usage();
    for (age, expected) in [(50, 5), (60, 1)] {
        *workbench.core.usage.lock().unwrap() =
            CachedUsage::at(pretended, Instant::now() - Duration::from_secs(age));
        workbench.refresh().await.unwrap();
        assert_eq!(stage.usage().descriptors.used, expected, "{age}");
    }
    workbench.shutdown().await.unwrap();
}

/// 段和散件核不上：这一轮停下、打一行固定类别，什么都不删；写入和封存照旧由写者自己拒绝。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_record_that_does_not_check_out_stops_the_round_and_is_logged() {
    let stage = Stage::new(true).await;
    let now = super::steps::now_millis();
    *stage.producer.archive_clock.lock().unwrap() = Some(now + 8 * DAY);
    stage.sealed(now, true);
    let requests = stage.data().join("history/meta-v1/requests");
    let ack = fs::read_dir(&requests)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_str().unwrap().ends_with("-intent.json.ack"))
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&ack).unwrap()).unwrap();
    value["digest"] = "b".repeat(64).into();
    fs::write(&ack, serde_json::to_vec(&value).unwrap()).unwrap();
    let lines = stage.archive_round(2_048).await;
    assert_eq!(lines, ["写者旧记录打包没做完：写者记录核不上"]);
    assert_eq!(fs::read_dir(&requests).unwrap().count(), 3);
    assert!(!stage.data().join("archive-v1").exists());
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 一条正文请求的四份散件编成一段、放进 `archive-v1/`，状态不存：就是打包断在提交之前留下的样子。
fn plant_orphan(data: &std::path::Path) -> std::path::PathBuf {
    use sha2::{Digest as _, Sha256};
    let hex = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let requests = data.join("history/meta-v1/requests");
    let id = fs::read_dir(&requests)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .find_map(|name| name.strip_suffix("-intent.json.ack").map(str::to_owned))
        .unwrap();
    let descriptor = fs::read(requests.join(format!("{id}.json"))).unwrap();
    let envelope: serde_json::Value = serde_json::from_slice(&descriptor).unwrap();
    let key = envelope["key_hash"].as_str().unwrap().to_owned();
    let records = [
        ("descriptor", descriptor),
        (
            "intent_ack",
            fs::read(requests.join(format!("{id}-intent.json.ack"))).unwrap(),
        ),
        (
            "outcome_ack",
            fs::read(requests.join(format!("{id}-outcome.json.ack"))).unwrap(),
        ),
        (
            "replay",
            fs::read(data.join(format!("idempotency/{key}.json"))).unwrap(),
        ),
    ];
    let entries: Vec<String> = records
        .iter()
        .map(|(part, bytes)| {
            format!(
                r#"{{"part":"{part}","bytes":{},"sha256":"{}"}}"#,
                bytes.len(),
                hex(bytes)
            )
        })
        .collect();
    let header = format!(
        r#"{{"version":1,"pair_id":{},"vault_uuid":{},"sequence":1,"previous":null,"packs":[{{"kind":"content","id":"{id}","key":"{key}","entries":[{}]}}]}}"#,
        envelope["pair_id"],
        envelope["vault_uuid"],
        entries.join(",")
    );
    let mut segment = b"SWARCH01".to_vec();
    segment.extend_from_slice(&u32::try_from(header.len()).unwrap().to_be_bytes());
    segment.extend_from_slice(header.as_bytes());
    for (_, bytes) in &records {
        segment.extend_from_slice(bytes);
    }
    let archive = data.join("archive-v1");
    fs::create_dir(&archive).unwrap();
    fs::set_permissions(&archive, fs::Permissions::from_mode(0o700)).unwrap();
    let path = archive.join(format!("{:08}-{}.seg", 1, hex(&segment)));
    fs::write(&path, segment).unwrap();
    path
}

/// 打包断在提交之前留下孤儿段：它在的时候封存做不成。下一轮不看用量、也不用有水位就收掉，封存照常做成。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_uncommitted_segment_is_cleared_next_round_whatever_the_usage() {
    const SETTLED: &str = "写者旧记录打包：上次没做完的已收口";
    let stage = Stage::new(true).await;
    let now = super::steps::now_millis();
    // 没有过成功封存（没有水位），用量也不到一半。
    let orphan = plant_orphan(&stage.data());
    assert_eq!(stage.archive_round(2_047).await, [SETTLED]);
    assert!(!orphan.exists());
    stage.sealed(now, true);
    // 有水位、用量不到一半也一样。
    let orphan = plant_orphan(&stage.data());
    assert_eq!(stage.archive_round(2_047).await, [SETTLED]);
    assert!(!orphan.exists());
    // 轮到封存的那一轮也先收，这一次封存就做得成。
    let orphan = plant_orphan(&stage.data());
    stage.sealed(now + DAY, true);
    assert!(!orphan.exists());
    // 收口之后用量要重量。
    assert_eq!(stage.usage().descriptors.used, 2_047);
    stage.assembled.workbench.refresh().await.unwrap();
    assert_eq!(stage.usage().descriptors.used, 1);
    stage.assembled.writable(true);
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 收口核不上：打一行固定类别，什么都不删，这一轮也不往下打。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_segment_that_does_not_check_out_is_left_alone_and_stops_the_round() {
    let stage = Stage::new(true).await;
    let now = super::steps::now_millis();
    *stage.producer.archive_clock.lock().unwrap() = Some(now + 8 * DAY);
    stage.written(0..39);
    stage.sealed(now, true);
    let orphan = plant_orphan(&stage.data());
    // 散件少了一条：段里那份是唯一的，不能删。
    let requests = stage.data().join("history/meta-v1/requests");
    let id = orphan_id(&orphan);
    let ack = requests.join(format!("{id}-outcome.json.ack"));
    let kept = fs::read(&ack).unwrap();
    fs::remove_file(&ack).unwrap();
    assert_eq!(
        stage.archive_round(2_048).await,
        ["写者旧记录打包没做完：写者记录核不上"]
    );
    assert!(orphan.exists());
    fs::write(&ack, kept).unwrap();
    let lines = stage.archive_round(2_048).await;
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(!orphan.exists());
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 孤儿段里那个请求号。
fn orphan_id(path: &std::path::Path) -> String {
    let bytes = fs::read(path).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    let at = text.find("\"id\":\"").unwrap() + 6;
    text[at..at + 64].to_owned()
}
