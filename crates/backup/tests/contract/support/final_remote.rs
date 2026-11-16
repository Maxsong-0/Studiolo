//! 明确的假远端；不会发网络请求，只用于持久状态及崩溃边界。
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use studiolo_backup::worker::{
    final_manifest::{
        FinalManifestDraft, FinalManifestLookup, FinalManifestReader, FinalManifestReceipt,
        FinalManifestWriter,
    },
    restic::{ResticError, SnapshotRef},
};
use studiolo_core::checkpoint::InstantMillis;

#[derive(Default)]
pub struct StubFinal {
    pub receipt: Mutex<Option<FinalManifestReceipt>>,
    pub lose_ack: Mutex<bool>,
    pub omit_write: Mutex<bool>,
    pub read_error: Mutex<bool>,
    pub absent_at: Mutex<i64>,
    pub commits: AtomicUsize,
    pub reads: AtomicUsize,
}
impl FinalManifestWriter for StubFinal {
    fn commit_final(
        &self,
        draft: &FinalManifestDraft,
    ) -> Result<FinalManifestReceipt, ResticError> {
        self.commits.fetch_add(1, Ordering::SeqCst);
        let receipt = FinalManifestReceipt::new(
            draft.clone(),
            SnapshotRef::parse("cd".repeat(32)).unwrap(),
            InstantMillis::from_millis(3000),
        )
        .unwrap();
        if !*self.omit_write.lock().unwrap() {
            *self.receipt.lock().unwrap() = Some(receipt.clone());
        }
        if *self.lose_ack.lock().unwrap() || *self.omit_write.lock().unwrap() {
            return Err(ResticError::ExecutionFailed);
        }
        Ok(receipt)
    }
}
impl FinalManifestReader for StubFinal {
    fn read_final(&self, draft: &FinalManifestDraft) -> Result<FinalManifestLookup, ResticError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if *self.read_error.lock().unwrap() {
            return Err(ResticError::ExecutionFailed);
        }
        match self.receipt.lock().unwrap().clone() {
            Some(receipt) if receipt.draft() == draft => {
                Ok(FinalManifestLookup::Verified(Box::new(receipt)))
            }
            Some(_) => Err(ResticError::ExecutionFailed),
            None => Ok(FinalManifestLookup::Absent {
                observed_at: InstantMillis::from_millis(*self.absent_at.lock().unwrap()),
            }),
        }
    }
}
