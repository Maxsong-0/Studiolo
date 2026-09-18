//! 健康页的纯数据模型：三态、时刻与恢复点证据。
//!
//! 这里没有任何 IO 与时钟能力；`now` 一律由调用方注入，
//! 以便时钟回退、证据缺失等情形都能被显式测试。

use std::fmt;

/// 健康三态。证据缺失一律 `Unknown`，绝不退化成 `Normal`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HealthState {
    /// 证据齐备且全部在预算内。
    Normal,
    /// 证据齐备但已越过预警或上限。
    Degraded,
    /// 证据缺失、不可信或超出可判断范围；不显示绿。
    Unknown,
}

impl HealthState {
    /// 汇总排序：`Degraded` 最重，其次 `Unknown`，`Normal` 最轻。
    ///
    /// 这样已知故障不会被"未知"掩盖，而"未知"仍然压得住"正常"，
    /// 任何一节缺证据就不可能把整体显示成绿色。
    #[must_use]
    pub fn severity(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Unknown => 1,
            Self::Degraded => 2,
        }
    }

    /// 取两者中较重的一个。
    #[must_use]
    pub fn worse(self, other: Self) -> Self {
        if other.severity() > self.severity() {
            other
        } else {
            self
        }
    }

    /// 稳定机器码，供前端与公开摘要使用；不含中文正文。
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Degraded => "degraded",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for HealthState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Normal => "正常",
            Self::Degraded => "降级",
            Self::Unknown => "未知",
        })
    }
}

/// Unix 秒时刻。没有时区与单调性承诺，回退必须显式判定。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant(i64);

impl Instant {
    /// 由 Unix 秒构造；不校验是否"合理"，合理性交给比较逻辑。
    #[must_use]
    pub fn from_unix_seconds(seconds: i64) -> Self {
        Self(seconds)
    }

    /// 取回 Unix 秒。
    #[must_use]
    pub fn unix_seconds(self) -> i64 {
        self.0
    }

    /// 自 `earlier` 起的秒数；`earlier` 晚于自身即判定为时钟回退。
    ///
    /// 整数一律 `checked_*`，溢出返回 [`AgeEvidence::Untrusted`] 而不是回绕。
    #[must_use]
    pub fn age_since(self, earlier: Self) -> AgeEvidence {
        match self.0.checked_sub(earlier.0) {
            None => AgeEvidence::Untrusted,
            Some(delta) if delta < 0 => AgeEvidence::ClockRolledBack,
            Some(delta) => u64::try_from(delta).map_or(AgeEvidence::Untrusted, AgeEvidence::Known),
        }
    }
}

/// 年龄证据。任何不可信情形都必须能被区分出来，不得折叠成 0。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgeEvidence {
    /// 可信年龄，单位秒。
    Known(u64),
    /// 本地时钟相对证据时刻回退；时间证据不可信。
    ClockRolledBack,
    /// 整数越界等无法判断的情形。
    Untrusted,
}

/// 灾难恢复点证据。上传返回 0 只是中间证据，这里只接受已核验的内容截止时刻。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryPointEvidence {
    /// 首次 seed 校验尚未通过；健康页固定显示"尚未建立灾难恢复点"。
    NotEstablished,
    /// 已核验的恢复点，时刻按内容截止时刻记，不按上传完成时刻记。
    Established {
        /// 该恢复点覆盖的内容截止时刻。
        content_cutoff: Instant,
    },
}

/// 组件角色标签。这是进程角色（如 `backup-worker`），不是自然人身份。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct OwnerLabel(String);

/// 标签不合法；不回显原值，避免把配置正文带进日志。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("组件角色标签不是 1..=32 位的小写 ASCII 短横线标识")]
pub struct OwnerLabelError;

impl OwnerLabel {
    /// 只接受小写字母、数字与短横线，长度 1..=32；不接受路径、邮箱或主机名。
    pub fn parse(value: &str) -> Result<Self, OwnerLabelError> {
        if value.is_empty()
            || value.len() > 32
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(OwnerLabelError);
        }
        Ok(Self(value.to_owned()))
    }

    /// 取回标签正文。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 单个 owner 的活性证据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerLiveness {
    /// 组件角色。
    pub owner: OwnerLabel,
    /// 最近一次活性回执时刻；`None` 表示本轮没有任何证据。
    pub last_receipt: Option<Instant>,
    /// 约定回执周期，单位秒；0 表示未约定，按未知处理。
    pub expected_period_seconds: u64,
}

/// 一节健康结论。`detail` 已在构造期排除路径与控制字符。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthSection {
    code: &'static str,
    title: &'static str,
    state: HealthState,
    detail: String,
}

/// 结论正文含有可能泄露位置或身份的证据。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("健康结论正文含有路径、控制字符或超过长度预算")]
pub struct SectionDetailError;

impl HealthSection {
    /// 构造一节结论；正文含 `/`、`\`、`@`、控制字符或超过 200 字符即拒绝。
    ///
    /// 这条边界是防泄露的实现手段：健康页对外只讲结论，不讲文件在哪、主人是谁。
    pub fn new(
        code: &'static str,
        title: &'static str,
        state: HealthState,
        detail: impl Into<String>,
    ) -> Result<Self, SectionDetailError> {
        let detail = detail.into();
        if detail.is_empty()
            || detail.chars().count() > 200
            || detail
                .chars()
                .any(|value| value.is_control() || matches!(value, '/' | '\\' | '@'))
        {
            return Err(SectionDetailError);
        }
        Ok(Self {
            code,
            title,
            state,
            detail,
        })
    }

    /// 稳定机器码。
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// 中文标题。
    #[must_use]
    pub fn title(&self) -> &'static str {
        self.title
    }

    /// 本节状态。
    #[must_use]
    pub fn state(&self) -> HealthState {
        self.state
    }

    /// 本节结论正文。
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// 本地只读健康聚合结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthReport {
    overall: HealthState,
    sections: Vec<HealthSection>,
}

impl HealthReport {
    /// 由各节结论汇总；没有任何一节时整体为 `Unknown`。
    ///
    /// 从 `Normal` 起按 [`HealthState::worse`] 折叠：只要有一节降级整体就降级，
    /// 只要有一节未知整体最少是未知，因此空证据永远不可能显示成绿色。
    #[must_use]
    pub fn new(sections: Vec<HealthSection>) -> Self {
        let overall = if sections.is_empty() {
            HealthState::Unknown
        } else {
            sections
                .iter()
                .map(HealthSection::state)
                .fold(HealthState::Normal, HealthState::worse)
        };
        Self { overall, sections }
    }

    /// 整体状态。
    #[must_use]
    pub fn overall(&self) -> HealthState {
        self.overall
    }

    /// 各节结论。
    #[must_use]
    pub fn sections(&self) -> &[HealthSection] {
        &self.sections
    }

    /// 公开摘要：只有机器码与状态，没有任何中文正文、路径或身份。
    #[must_use]
    pub fn public_summary(&self) -> PublicSummary {
        PublicSummary {
            overall: self.overall,
            sections: self
                .sections
                .iter()
                .map(|section| (section.code(), section.state()))
                .collect(),
        }
    }
}

/// 对外公开摘要。结构上装不下路径、主人身份或秘密。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicSummary {
    overall: HealthState,
    sections: Vec<(&'static str, HealthState)>,
}

impl PublicSummary {
    /// 整体状态。
    #[must_use]
    pub fn overall(&self) -> HealthState {
        self.overall
    }

    /// 各节机器码与状态。
    #[must_use]
    pub fn sections(&self) -> &[(&'static str, HealthState)] {
        &self.sections
    }
}
