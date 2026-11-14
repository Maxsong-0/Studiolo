//! 执行器必须消费真实封存能力；不能把参数校验改回任意有效路径。
use super::super::authority_tokens::compact;
use std::path::Path;
pub(super) fn fixed_binding(path: &Path, source: &str) -> bool {
    let source = compact(source);
    let anchors: &[&str] = if path.ends_with("crates/backup-worker/src/source.rs") {
        &[
            "config:&WorkerConfig,key:JobKey,reference:&SealedReference",
            "key.checkpoint_id!=reference.checkpoint_id",
            "SealedCheckpoint::open_for_worker(config.sealed_parent(),reference,config.reader_identity(),now,deadline)",
            "SealedCheckpoint::open_for_worker(&self.parent,self.sealed().reference(),self.identity,utc_now()?,self.deadline())",
            "self.binding.check_reopened(&reopened)?;",
            "self.check_parents()?;self.check_time()",
            "self.deadline())?!=self.parents",
        ]
    } else if path.ends_with("crates/backup-worker/src/runner.rs") {
        &[
            "Source::open(config,key,reference,deadline)?",
            "letbackup=self.source.accepts(invocation)?;self.source.check()?;",
            "self.source.deadline(),&||self.source.check_time()",
            "self.source.check()?;parse_output(&output,backup)",
        ]
    } else if path.ends_with("crates/backup-worker/src/source/binding.rs") {
        &[
            "key.checkpoint_id!=reference.checkpoint_id||sealed.reference()!=reference",
            "tag:JobTag::for_job(&key)",
            "source_root:sealed.ticket().transfer_source.root.clone()",
            "ifinvocation==&self.backup",
            "elseifinvocation==&self.check",
            "reopened.reference()!=self.sealed.reference()||reopened.ticket()!=self.sealed.ticket()||reopened.manifest()!=self.sealed.manifest()",
        ]
    } else if path.ends_with("crates/backup-worker/src/source/window.rs") {
        &[
            "SystemTime::now().duration_since(UNIX_EPOCH)",
            "self.closed.get()||now<self.last.get()||now>=self.expiry||instant>=self.deadline.get()",
            "self.closed.set(true);",
            "self.deadline.set(self.deadline.get().min(expiry))",
        ]
    } else {
        &[]
    };
    anchors.iter().all(|anchor| source.contains(anchor))
}
