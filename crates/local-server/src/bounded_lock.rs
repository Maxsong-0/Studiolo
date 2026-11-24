//! 封存冻结要的几把进程内锁：有人短暂拿着就等一小会儿，不当场放弃（BUG-055）。
//!
//! 冻结在排空完整操作之后进行，但有些读取不取完整操作许可就拿这几把锁（网页读一篇笔记、监视回调登记一次失效、
//! 就绪检查），每次几毫秒。以前用 `try_lock`，撞上就整轮封存失败、白等一个间隔。
//! 等有上限：拿着不放的（真卡死）照旧按原来的类别失败，不把冻结预算耗在一把锁上。
use std::{
    sync::{Mutex, MutexGuard, TryLockError},
    thread,
    time::{Duration, Instant},
};

/// 冻结时最多为一把锁等这么久。短暂持有者都是毫秒级；再久就是卡住了，等下去只会拉长冻结窗口。
pub(crate) const BRIEF: Duration = Duration::from_secs(1);
/// 两次试拿之间的间隔。
const POLL: Duration = Duration::from_millis(5);

/// 没等到的两种原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    /// 到期还没等到。
    TimedOut,
    /// 持有者 panic 过，里面的状态不可信。
    Poisoned,
}

/// 冻结预算之内、最多 [`BRIEF`]：取两者较早的一个。
pub(crate) fn brief(deadline: Instant) -> Instant {
    deadline.min(Instant::now() + BRIEF)
}

/// 等到 `deadline` 为止。拿得到就当场返回；到期之后不再试。
pub(crate) fn lock_until<T>(
    lock: &Mutex<T>,
    deadline: Instant,
) -> Result<MutexGuard<'_, T>, Fault> {
    loop {
        match lock.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(_)) => return Err(Fault::Poisoned),
            Err(TryLockError::WouldBlock) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(Fault::TimedOut);
                }
                thread::sleep(left.min(POLL));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BRIEF, Fault, brief, lock_until};
    use std::{
        sync::{Arc, Mutex, mpsc},
        thread,
        time::{Duration, Instant},
    };

    /// 另起一个线程拿着锁 `hold` 这么久；返回时它已经拿到。
    fn held(lock: &Arc<Mutex<u32>>, hold: Duration) -> thread::JoinHandle<()> {
        let lock = lock.clone();
        let (taken, took) = mpsc::channel();
        let holder = thread::spawn(move || {
            let guard = lock.lock().unwrap();
            taken.send(()).unwrap();
            thread::sleep(hold);
            drop(guard);
        });
        took.recv().unwrap();
        holder
    }

    #[test]
    fn a_lock_held_briefly_is_waited_for() {
        let lock = Arc::new(Mutex::new(7));
        let holder = held(&lock, Duration::from_millis(200));
        let guard = lock_until(&lock, Instant::now() + Duration::from_secs(10)).unwrap();
        assert_eq!(*guard, 7);
        drop(guard);
        holder.join().unwrap();
    }

    #[test]
    fn a_lock_that_is_never_let_go_times_out_at_the_deadline() {
        let lock = Arc::new(Mutex::new(7));
        let holder = held(&lock, Duration::from_millis(600));
        let started = Instant::now();
        let deadline = started + Duration::from_millis(100);
        assert_eq!(lock_until(&lock, deadline).err(), Some(Fault::TimedOut));
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "过了期限还在等"
        );
        holder.join().unwrap();
    }

    /// 期限已过但锁空着：照样拿到，不因为期限先拒。期限的判断在调用方的冻结步骤里。
    #[test]
    fn a_free_lock_is_taken_even_after_the_deadline() {
        let lock = Mutex::new(7);
        let guard = lock_until(&lock, Instant::now()).unwrap();
        assert_eq!(*guard, 7);
    }

    #[test]
    fn a_poisoned_lock_is_a_fault_not_a_wait() {
        let lock = Arc::new(Mutex::new(7));
        let poisoner = lock.clone();
        assert!(
            thread::spawn(move || {
                let _guard = poisoner.lock().unwrap();
                panic!("模拟持有者出错");
            })
            .join()
            .is_err()
        );
        let started = Instant::now();
        let result = lock_until(&lock, started + Duration::from_secs(10));
        assert_eq!(result.err(), Some(Fault::Poisoned));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn the_wait_is_bounded_by_both_the_freeze_deadline_and_the_cap() {
        let now = Instant::now();
        assert!(brief(now) <= now, "冻结到期了就不再多等");
        assert!(brief(now + Duration::from_secs(60)) <= Instant::now() + BRIEF);
    }
}
