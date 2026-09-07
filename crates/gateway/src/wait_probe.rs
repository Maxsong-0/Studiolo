//! 私有测试观测点：只在锁内已确定 InFlight 后报告，不替代真实 guard 检查。

use std::sync::{Mutex, mpsc};

#[derive(Default)]
pub(super) struct WaitProbe {
    entered: Mutex<Option<mpsc::Sender<()>>>,
}

impl WaitProbe {
    pub(super) fn arm(&self) -> mpsc::Receiver<()> {
        let (sender, receiver) = mpsc::channel();
        *self.entered.lock().unwrap() = Some(sender);
        receiver
    }

    pub(super) fn entered(&self) {
        if let Some(sender) = self.entered.lock().unwrap().take() {
            // 无界通道发送不会等待测试线程；实际放锁仍由下方 Condvar::wait 完成。
            let _ = sender.send(());
        }
    }
}
