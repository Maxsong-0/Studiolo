//! 把平面关停标志传给执行器；仅在完整动作之间生效。
use std::sync::atomic::{AtomicBool, Ordering};
use studiolo_sync_bridge::{
    Action,
    executor::{IssuedJob, JobBackend, JobOutcome, ObjectId},
};

pub(super) struct StoppableBackend<'a> {
    pub inner: &'a mut dyn JobBackend,
    pub stopping: &'a AtomicBool,
}

impl JobBackend for StoppableBackend<'_> {
    fn stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }
    fn apply(&mut self, job: &IssuedJob, action: &Action, staged: Option<ObjectId>) -> JobOutcome {
        // 不在 Issued 之后取消调用：必须把本动作真实终态交回 journal。
        self.inner.apply(job, action, staged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Unused;
    impl JobBackend for Unused {
        fn apply(&mut self, _: &IssuedJob, _: &Action, _: Option<ObjectId>) -> JobOutcome {
            panic!("此用例只检查真实关停标志接线");
        }
    }
    #[test]
    fn observes_the_live_plane_flag() {
        let flag = AtomicBool::new(false);
        let mut inner = Unused;
        let backend = StoppableBackend {
            inner: &mut inner,
            stopping: &flag,
        };
        assert!(!backend.stopping());
        flag.store(true, Ordering::Release);
        assert!(backend.stopping());
    }
}
