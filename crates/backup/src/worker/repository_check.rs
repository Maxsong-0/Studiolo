//! 备份仓库每日轮流抽查的共用事实：组号、检查键、三类结果、审计事件和 restic 输出归类。
//!
//! 仓库按数据包 ID 分成 30 组（`check --read-data-subset=n/30`），每次读一组；结构检查
//! （全部快照的树）每次都跑。查哪组、什么时候查由人侧生产者按账本定，worker 只执行和上报。
//! 这里只有纯逻辑：不启动进程、不读写账本。
//!
//! 结果只分三类：干净、查出问题、没做完。restic 的退出码和 summary 分不出「仓库坏了」和
//! 「连不上、凭据不对」，所以「查出问题」只说检查没通过；只有 `broken_packs` 非空才说明读到了
//! 损坏的数据包。没做完的不算任何结论，也不算读过这一组。

mod event;
mod output;

pub use event::{CheckEvent, check_abandoned, check_intent, check_outcome, read_check_event};
pub use output::classify_check_output;

use super::restic::RepositoryRef;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::{num::NonZeroU64, time::Duration};

/// 抽查事件沿用备份的 L2 动作名前缀；和备份作业的事件键互不重叠。
pub const REPOSITORY_CHECK_ACTION: &str = "backup.repository_check";
/// 一圈分几组；restic 要求 `t ≤ 256`。
pub const CHECK_BUCKETS: u8 = 30;
/// worker 一次抽查最多跑多久，超了算「没做完（超时）」，不让检查长时间挡住备份。
/// 生产者判断原意图挂多久算 worker 中途没了、健康页正文写超时多久，都从这一个常量算。
pub const CHECK_WINDOW: Duration = Duration::from_secs(20 * 60);

/// 组号 1～30：第 `n` 组读数据包 ID 第一个字节对 30 取余等于 `n-1` 的那些包。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CheckBucket(u8);

impl CheckBucket {
    /// 一圈的第一组。
    pub const FIRST: Self = Self(1);

    /// 不在 1～30 就没有。
    #[must_use]
    pub fn new(value: u8) -> Option<Self> {
        (1..=CHECK_BUCKETS).contains(&value).then_some(Self(value))
    }

    /// 组号本身。
    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }

    /// 轮转的下一组；第 30 组之后回到第 1 组。
    #[must_use]
    pub fn following(self) -> Self {
        if self.0 >= CHECK_BUCKETS {
            Self::FIRST
        } else {
            Self(self.0 + 1)
        }
    }

    /// 交给 `--read-data-subset` 的 `n/30`。
    #[must_use]
    pub fn subset(self) -> String {
        format!("{}/{CHECK_BUCKETS}", self.0)
    }

    /// 全部 30 组，从第 1 组起。
    pub fn all() -> impl Iterator<Item = Self> {
        (1..=CHECK_BUCKETS).map(Self)
    }
}

impl<'de> Deserialize<'de> for CheckBucket {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = u8::deserialize(deserializer)?;
        Self::new(value).ok_or_else(|| serde::de::Error::custom("组号不在 1～30"))
    }
}

/// 这次检查为什么排上：按天轮转，还是重查上次没通过或没做完的组。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckPurpose {
    /// 每天轮到的下一组。
    Rotation,
    /// 重查最近一次不干净的组。
    Retry,
}

/// 检查键：生产者发的单调序号（和备份作业序号分开）加组号。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckKey {
    /// 检查序号，单调递增、不复用。
    pub sequence: NonZeroU64,
    /// 这次读哪一组。
    pub bucket: CheckBucket,
}

/// 生产者派下的一次检查。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckAssignment {
    /// 检查键。
    pub key: CheckKey,
    /// 排上的原因。
    pub purpose: CheckPurpose,
}

/// 三类结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckResult {
    /// restic 退出 0，唯一的 summary 里没有错误、没有损坏的数据包。
    Clean,
    /// summary 里报了错误或损坏的数据包：仓库坏了，或者连不上、凭据不对，分不出来。
    Problem,
    /// 没有可信的结论：被备份打断、超时、输出越界或格式不对、没装配起来、worker 中途没了。
    Incomplete,
}

/// 没做完的固定原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncompleteReason {
    /// 有备份作业要做，检查让出并结束了进程组。
    Preempted,
    /// 到了单次检查的时间上限。
    Deadline,
    /// 输出超过上限。
    OutputLimit,
    /// 输出不是固定版本的格式：没有或不止一个 summary、字段不对、退出码和 summary 对不上。
    Protocol,
    /// 固定包、凭据、临时目录或子进程没装配起来。
    Unavailable,
    /// worker 在检查中途没了，由生产者补记。
    Abandoned,
}

/// 一次检查的结果；字段组合在构造时就核过。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "OutcomeFields", into = "OutcomeFields")]
pub struct CheckOutcome {
    result: CheckResult,
    reason: Option<IncompleteReason>,
    errors: u64,
    broken_packs: u64,
}

impl CheckOutcome {
    /// 干净。
    #[must_use]
    pub fn clean() -> Self {
        Self {
            result: CheckResult::Clean,
            reason: None,
            errors: 0,
            broken_packs: 0,
        }
    }

    /// 查出问题；错误数和损坏包数都是 0 就不是问题，返回 `None`。
    #[must_use]
    pub fn problem(errors: u64, broken_packs: u64) -> Option<Self> {
        (errors > 0 || broken_packs > 0).then_some(Self {
            result: CheckResult::Problem,
            reason: None,
            errors,
            broken_packs,
        })
    }

    /// 没做完。
    #[must_use]
    pub fn incomplete(reason: IncompleteReason) -> Self {
        Self {
            result: CheckResult::Incomplete,
            reason: Some(reason),
            errors: 0,
            broken_packs: 0,
        }
    }

    /// 结果类别。
    #[must_use]
    pub fn result(&self) -> CheckResult {
        self.result
    }

    /// 没做完的原因；另外两类没有。
    #[must_use]
    pub fn reason(&self) -> Option<IncompleteReason> {
        self.reason
    }

    /// summary 里的错误数。
    #[must_use]
    pub fn errors(&self) -> u64 {
        self.errors
    }

    /// summary 里列出的损坏数据包个数；只记个数，不记 ID，消息才有界。
    #[must_use]
    pub fn broken_packs(&self) -> u64 {
        self.broken_packs
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeFields {
    result: CheckResult,
    // 缺了这个字段不能默认成空：只认写出来的 `null` 或原因，形状才唯一。
    #[serde(deserialize_with = "Option::deserialize")]
    reason: Option<IncompleteReason>,
    errors: u64,
    broken_packs: u64,
}

impl TryFrom<OutcomeFields> for CheckOutcome {
    type Error = &'static str;

    fn try_from(fields: OutcomeFields) -> Result<Self, Self::Error> {
        let outcome = match (fields.result, fields.reason) {
            (CheckResult::Clean, None) => Some(Self::clean()),
            (CheckResult::Problem, None) => Self::problem(fields.errors, fields.broken_packs),
            (CheckResult::Incomplete, Some(reason)) => Some(Self::incomplete(reason)),
            _ => None,
        };
        outcome
            .filter(|outcome| {
                outcome.errors == fields.errors && outcome.broken_packs == fields.broken_packs
            })
            .ok_or("检查结果的字段组合不成立")
    }
}

impl From<CheckOutcome> for OutcomeFields {
    fn from(outcome: CheckOutcome) -> Self {
        Self {
            result: outcome.result,
            reason: outcome.reason,
            errors: outcome.errors,
            broken_packs: outcome.broken_packs,
        }
    }
}

/// 检查的是哪个仓库：仓库定位符加固定前缀的 SHA-256。换了仓库，旧仓库读过的组不再算数。
///
/// 只看定位符：同一个地址下删库重建认不出来，这种维护要按部署说明先换地址。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct RepositoryDigest(String);

impl RepositoryDigest {
    /// 由 worker 的固定仓库定位符算出；定位符本身不进账本。
    #[must_use]
    pub fn of(repository: &RepositoryRef) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"studiolo-backup-repository\0");
        hasher.update(repository.as_str().as_bytes());
        Self(
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        )
    }

    /// 只认 64 位小写十六进制。
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        (text.len() == 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        .then(|| Self(text.to_owned()))
    }

    /// 摘要文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RepositoryDigest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| serde::de::Error::custom("仓库摘要不是 64 位小写十六进制"))
    }
}

/// 谁写的结果：worker 跑完上报（带它检查的仓库），或生产者给中途没了的检查补「没做完」。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckRecorder {
    /// worker 上报。
    Worker(RepositoryDigest),
    /// 生产者补记。
    Producer,
}
