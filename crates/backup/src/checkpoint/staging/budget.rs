//! 删除和回收的预算：每做一步删除（改名、删文件、删目录）之前先花一步。
//! 生产里只有截止时间，期限一到就停在两步之间，和进程被杀、I/O 出错停下的样子一样；
//! 私有测试另给一个步数上限，停在任意两步之间，看下一轮能不能接着做完。
use super::{Result, StagingError, check_deadline};
use std::time::Instant;

pub(super) struct Budget {
    deadline: Instant,
    steps: Option<u64>,
}
impl Budget {
    pub fn until(deadline: Instant) -> Self {
        Self {
            deadline,
            steps: None,
        }
    }
    /// 只给私有测试：做完 `steps` 步删除就停下，相当于期限正好在那一步之后到。
    #[cfg(test)]
    pub fn steps(deadline: Instant, steps: u64) -> Self {
        Self {
            deadline,
            steps: Some(steps),
        }
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    /// 只看期限、不花步数；核对和读取之前用。
    pub fn check(&self) -> Result<()> {
        check_deadline(self.deadline)
    }
    /// 每一次改名、删文件、删目录之前花一步；期限到了或步数花完就停在这里，已经做的不撤回。
    pub fn spend(&mut self) -> Result<()> {
        self.check()?;
        match &mut self.steps {
            Some(0) => Err(StagingError::Expired),
            Some(left) => {
                *left -= 1;
                Ok(())
            }
            None => Ok(()),
        }
    }
}
