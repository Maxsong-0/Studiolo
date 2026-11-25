//! 写者旧记录的自动打包（BUG-051）：只在不封存的那一轮、用量到一半、有过一次成功封存时打；打完照常封存。
use super::{producer::Stage, *};
use crate::{backup::producer::TARGET, health::RecordUsageProbe as _};
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};
use studiolo_backup::worker::{
    audit::{AdmissionFact, QueueAnswer, QueueCommand},
    job::BACKUP_AUDIT_ACTION,
};
use studiolo_vault_broker::{RecordUsage, RecordUsageItem};

const DAY: i64 = 24 * 60 * 60 * 1000;

impl Stage {
    /// 用量到一半（没到就 `half` 给假），「现在」拨到八天后，跑一轮只回收不封存的。
    async fn archive_round(&self, half: bool) -> Vec<String> {
        let core = &self.assembled.workbench.core;
        // 先让在途的重扫走完，免得它把下面摆的用量量回去。
        self.assembled.workbench.refresh().await.unwrap();
        // 每次重扫都量一次，健康页读的就是它。
        let measured = self.assembled.workbench.record_usage();
        assert_eq!(measured.map(|usage| usage.replays.limit), Some(4_096));
        let item = RecordUsageItem {
            used: if half { 2_048 } else { 2_047 },
            limit: 4_096,
        };
        *core.usage.lock().unwrap() = Some(RecordUsage {
            descriptors: item,
            descriptor_bytes: RecordUsageItem { used: 0, limit: 1 },
            replays: RecordUsageItem { used: 0, limit: 1 },
            replay_bytes: RecordUsageItem { used: 0, limit: 1 },
            file_operations: RecordUsageItem { used: 0, limit: 1 },
            file_operation_bytes: RecordUsageItem { used: 0, limit: 1 },
            ..RecordUsage::default()
        });
        self.producer.seal_lines.lock().unwrap().clear();
        self.producer.reclaim();
        self.producer.settle();
        self.producer.seal_lines.lock().unwrap().clone()
    }
    fn data(&self) -> std::path::PathBuf {
        self.assembled.fixture.root.join("data")
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
    // 还没有一次成功封存：用量到了也不打。
    assert!(stage.archive_round(true).await.is_empty());
    stage.sealed(now, false);
    assert!(stage.archive_round(true).await.is_empty(), "没验过不算成功");
    stage.sealed(now + DAY, true);
    // 用量没到一半：不打，写者状态的格式不会自己变。
    assert!(stage.archive_round(false).await.is_empty());
    assert!(!archive.exists());
    let lines = stage.archive_round(true).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("写者旧记录打包：1 条，"), "{lines:?}");
    assert_eq!(fs::read_dir(&archive).unwrap().count(), 1);
    assert_eq!(
        fs::read_dir(stage.data().join("history/meta-v1/requests"))
            .unwrap()
            .count(),
        0
    );
    // 打完量过一次，健康页读到的是打包之后的用量。
    let usage = stage
        .assembled
        .workbench
        .core
        .usage
        .lock()
        .unwrap()
        .unwrap();
    assert_eq!((usage.packs, usage.descriptors.used), (1, 0));
    // 打了包的照常封存、逐条核回执；业务照常读写。
    stage.sealed(now + 2 * DAY, true);
    stage.assembled.writable(true);
    // 再来一轮没有可打的：不打行、不多出段。
    assert!(stage.archive_round(true).await.is_empty());
    assert_eq!(fs::read_dir(&archive).unwrap().count(), 1);
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 关停叫停之后不再开始新的一步；写者已经不在时也只是这一轮不打。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_down_stops_packing_before_the_next_step() {
    let stage = Stage::new(true).await;
    let now = super::steps::now_millis();
    *stage.producer.archive_clock.lock().unwrap() = Some(now + 8 * DAY);
    stage.sealed(now, true);
    stage.producer.settle_window();
    assert!(stage.archive_round(true).await.is_empty());
    assert!(!stage.data().join("archive-v1").exists());
    stage.assembled.workbench.shutdown().await.unwrap();
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
    let lines = stage.archive_round(true).await;
    assert_eq!(lines, ["写者旧记录打包没做完：写者记录核不上"]);
    assert_eq!(fs::read_dir(&requests).unwrap().count(), 3);
    assert!(!stage.data().join("archive-v1").exists());
    stage.assembled.workbench.shutdown().await.unwrap();
}
