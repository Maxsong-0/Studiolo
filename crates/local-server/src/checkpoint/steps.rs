//! 封存分步用时（BUG-050）：冻结窗口里、放开之后各记一份，按走过的顺序记步名和毫秒，进日志。
//!
//! 169 上冻结窗口 26 秒，日志里只有一个总数，分不清是哪一步慢；有了分步，真机一眼看得出。
use std::time::{Duration, Instant};

/// 走过的步骤和各自用时；每一步从上一步记完（或开始计时）算到记它的时候。
#[derive(Clone, Debug)]
pub struct StepTimes {
    steps: Vec<(&'static str, Duration)>,
    since: Instant,
    /// 用例用：每记一步当场报一声，用例照它接着往下演，不靠睡多久去猜走到哪了。
    #[cfg(test)]
    watch: Option<std::sync::mpsc::Sender<&'static str>>,
}

impl StepTimes {
    /// 从现在开始计时，还没有走过任何一步。
    #[must_use]
    pub fn start() -> Self {
        Self {
            steps: Vec::new(),
            since: Instant::now(),
            #[cfg(test)]
            watch: None,
        }
    }

    /// 记一步：从上一步记完（或开始计时）到现在。
    pub fn mark(&mut self, step: &'static str) {
        let now = Instant::now();
        self.steps
            .push((step, now.saturating_duration_since(self.since)));
        self.since = now;
        #[cfg(test)]
        if let Some(watch) = &self.watch {
            let _ = watch.send(step);
        }
    }

    /// 用例用：从现在开始计时，每记一步往 `watch` 报一声。
    #[cfg(test)]
    pub(crate) fn watched(watch: std::sync::mpsc::Sender<&'static str>) -> Self {
        Self {
            watch: Some(watch),
            ..Self::start()
        }
    }

    /// 按走过的顺序列出步名；用例照它核对有没有漏步、顺序对不对。
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        self.steps.iter().map(|(step, _)| *step).collect()
    }

    /// 日志里的写法：「补记登录库 3、排空接入方 0」（毫秒，向下取整）；一步都没走过写「无」。
    #[must_use]
    pub fn render(&self) -> String {
        if self.steps.is_empty() {
            return "无".to_owned();
        }
        self.steps
            .iter()
            .map(|(step, took)| format!("{step} {}", took.as_millis()))
            .collect::<Vec<_>>()
            .join("、")
    }

    /// 用例用：直接给出各步用时（毫秒），不靠真实计时。
    #[cfg(test)]
    pub(crate) fn of(steps: &[(&'static str, u64)]) -> Self {
        Self {
            steps: steps
                .iter()
                .map(|(step, millis)| (*step, Duration::from_millis(*millis)))
                .collect(),
            ..Self::start()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StepTimes;

    #[test]
    fn steps_render_in_order_with_whole_milliseconds() {
        assert_eq!(StepTimes::start().render(), "无");
        let steps = StepTimes::of(&[("补记登录库", 3), ("定格写者两棵树", 26_000)]);
        assert_eq!(steps.render(), "补记登录库 3、定格写者两棵树 26000");
        assert_eq!(steps.names(), ["补记登录库", "定格写者两棵树"]);
        let mut marked = StepTimes::start();
        marked.mark("甲");
        marked.mark("乙");
        assert_eq!(marked.names(), ["甲", "乙"]);
    }
}
