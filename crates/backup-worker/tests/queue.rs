//! 队列只接受文件名与 checkpoint 标识一致的作业键。
use std::num::NonZeroU64;
use studiolo_backup::worker::state::JobKey;
use studiolo_backup_worker::read_queue;
use studiolo_core::checkpoint::CheckpointId;

#[test]
fn missing_directory_is_an_empty_queue_and_a_mismatched_name_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("absent");
    assert!(read_queue(&missing).unwrap().is_empty());
    let queue = root.path().join("queue");
    std::fs::create_dir(&queue).unwrap();
    let key = JobKey {
        sequence: NonZeroU64::new(3).unwrap(),
        checkpoint_id: CheckpointId::parse("cp-3").unwrap(),
    };
    std::fs::write(queue.join("cp-3.json"), serde_json::to_vec(&key).unwrap()).unwrap();
    assert_eq!(read_queue(&queue).unwrap(), vec![key]);
    std::fs::write(
        queue.join("other.json"),
        b"{\"sequence\":1,\"checkpoint_id\":\"cp-3\"}",
    )
    .unwrap();
    assert!(read_queue(&queue).is_err());
}
