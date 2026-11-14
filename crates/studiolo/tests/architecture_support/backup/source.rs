//! 执行器必须消费真实封存能力；不能把参数校验改回任意有效路径。
use super::super::authority_tokens::compact;
use std::path::Path;
pub(super) fn fixed_binding(path: &Path, source: &str) -> bool {
    let source = compact(source);
    let anchors: &[&str] = if path.ends_with("crates/backup/src/checkpoint/staging/io/journal.rs") {
        &[
            "LIMIT:usize=16*1024",
            "Self::Identity=>(\"identity.json\",\"identity.new\")",
            "Self::Export=>(\"export.json\",\"export.new\")",
            "Self::Pending=>(\"pending.json\",\"pending.new\")",
            "Self::Sealed=>(\"sealed.json\",\"sealed.new\")",
            "Regular::create(self,temporary,bytes.len()asu64)?",
            "self.publish(temporary,name)?;",
            "file.require_readonly()?;",
            "file.check(self,name)?;",
        ]
    } else if path.ends_with("crates/backup/src/checkpoint/staging/housekeeping.rs") {
        &[
            "parent.acquire_existing_lease()?;",
            "plan::read_all(&parent,reader,started,deadline)?;",
            "parent.check_path(parent_path)?;",
            "plan.check(&parent)?;",
            "expiry>now",
            "root.discard_until(&parent,name,reader,deadline)?;",
            "plan.marker.discard_until(&parent,&plan.name,None,deadline)?;",
        ]
    } else if path.ends_with("crates/backup/src/checkpoint/staging/housekeeping/plan.rs") {
        &[
            "identity.validate(parent,&marker,key,reader,now)?;",
            "parent.reclaim_child(&root_name,proof.directory,reader)?;",
            "ifknown!=covered",
            "self.marker.read_journal(*kind)?!=*expected",
            "ifretained&&prefix!=\"sealed\"",
        ]
    } else if path.ends_with("crates/backup-worker/src/source.rs") {
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
            "letsource_time=||self.source.check_time();",
            "self.source.deadline(),&source_time",
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
            "reopened.inventory()!=self.sealed.inventory()",
        ]
    } else if path.ends_with("crates/backup-worker/src/runner/inspect.rs") {
        &[
            "forfileinself.sealed().inventory().files()",
            "plan::file(self.source.repository(),snapshot,root,file.path())?",
            "process::DigestExpectation{bytes:file.bytes(),sha256:file.sha256()}",
            "self.source.check()",
            "self.runtime.digest(",
        ]
    } else if path.ends_with("crates/backup-worker/src/runner/probe.rs") {
        &[
            "ifkey!=self.key()",
            "self.locate_snapshot(self.key())?.as_ref()!=Some(snapshot)",
            "self.verify_snapshot_files(snapshot)?;",
            "inventory:self.sealed().inventory()",
            "letverified_at=self.source.current_time()?;",
            "self.runtime.capture(",
        ]
    } else if path.ends_with("crates/backup-worker/src/runner/inspect/plan.rs") {
        &[
            "snapshot.as_str().len()!=64",
            "root.join(relative.as_str())",
            "\"snapshots\".into(),\"--tag\".into(),tag.as_str().into()",
            "\"dump\".into(),snapshot.as_str().into(),text.into()",
            "\"ls\".into(),snapshot.as_str().into()",
        ]
    } else if path.ends_with("crates/backup-worker/src/runtime.rs") {
        &[
            "ifself.poisoned.get(){returnErr(ProcessError::CleanupFailed);}",
            "self.poisoned.set(true)",
            "self.window.check()?;source_time()",
            "self.temporary.check()",
            "ResticPackage::open(config.restic_sha256(),deadline)?",
        ]
    } else if path.ends_with("crates/backup-worker/src/runner/final_record.rs") {
        &[
            "self.source.check()?;",
            "self.verified.borrow_mut().take().ok_or(ProcessError::Source)?",
            "if&verified!=draft",
            "Some(&bytes),self.source.deadline(),&||self.source.check_time()",
            "read_remote(&self.runtime,draft)",
            "runtime.digest(argv,expected,runtime.deadline(),&||Ok(()))",
        ]
    } else if path.ends_with("crates/backup-worker/src/runner/final_record/read.rs") {
        &[
            "snapshot::locate_named(&output.stdout,&marker.tag,&marker.path)",
            "listing::verify(&output.stdout,&marker,&found,encoded.len()asu64)?",
            "DigestExpectation{bytes:encoded.len()asu64,sha256:&hash}",
            "iflocate()?.as_ref()!=Some(&found)",
        ]
    } else if path.ends_with("crates/backup-worker/src/runner/final_record/plan.rs") {
        &[
            "format!(\"{}.final\",original.as_str())",
            "format!(\"/studiolo-final-{}.json\",original.as_str())",
            "\"backup\".into(),\"--stdin\".into(),\"--stdin-filename\".into()",
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
