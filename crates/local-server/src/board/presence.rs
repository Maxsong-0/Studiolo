//! 「在做什么」状态的纯逻辑（ADR-046）：收整份快照、比 `seq`、过期、按授权过滤、登记限频。
//!
//! 不碰文件、网络和时钟：时刻由调用方以「进程起步以来多久」传入，用例可以自己拨。状态只在内存里，
//! 不是工作项，不进控制库和备份。记录里没有对话、命令、路径、清单文字、会话标题。
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::Duration,
};
use studiolo_core::VaultPath;

/// 满这么久没有新快照，这个来源的状态整份消失。
pub(super) const EXPIRES: Duration = Duration::from_secs(300);
/// 一个来源最多这么多条。
const MAX_PER_SOURCE: usize = 64;
/// 全部来源合计最多这么多条：人侧内存贴着上限，常驻必须有界。
const MAX_TOTAL: usize = 256;
/// JSON 里能精确表示的最大整数；再大的 `seq` 两端会读成不同的数。
const MAX_SEQ: u64 = (1 << 53) - 1;
/// 「新会话第一次出现」的登记，一个来源每小时最多这么多条（账本不能被灌）。
const RECORDS_PER_HOUR: usize = 60;
const HOUR: Duration = Duration::from_secs(3600);
/// 同一（来源，原因）的拒绝最多每分钟登记一条。
const DENIAL_EVERY: Duration = Duration::from_secs(60);
const MAX_BRANCH_BYTES: usize = 120;

/// 一份整份快照；多一个字段、取值不在枚举里都解析失败。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Snapshot {
    seq: u64,
    sessions: Vec<Session>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    kind: Kind,
    key: String,
    vendor: Vendor,
    project: VaultPath,
    #[serde(default)]
    branch: Option<String>,
    state: State,
    #[serde(default)]
    step: Option<u16>,
    #[serde(default)]
    total: Option<u16>,
    #[serde(default)]
    last_action: Option<Action>,
    #[serde(default)]
    idle_secs: Option<u32>,
}

/// 记录种类；以后 Bottega P4 加 `card`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Session,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Vendor {
    ClaudeCode,
    Codex,
    Agy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Running,
    Waiting,
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Read,
    Edit,
    Run,
    Web,
    Ask,
    Dispatch,
    Other,
}

impl Snapshot {
    /// 解析并核全部字段；任何一条不合就整份不收，不说是哪一条、不回显内容。
    pub(super) fn parse(value: serde_json::Value) -> Option<Self> {
        let snapshot = serde_json::from_value::<Self>(value).ok()?;
        let keys = snapshot
            .sessions
            .iter()
            .map(|session| session.key.as_str())
            .collect::<BTreeSet<_>>();
        (snapshot.seq <= MAX_SEQ
            && snapshot.sessions.len() <= MAX_PER_SOURCE
            && keys.len() == snapshot.sessions.len()
            && snapshot.sessions.iter().all(Session::valid))
        .then_some(snapshot)
    }
}

impl Session {
    fn valid(&self) -> bool {
        let Kind::Session = self.kind;
        (16..=64).contains(&self.key.len())
            && self
                .key
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            && self.project.as_str().starts_with("项目/")
            && self.project.as_str().split('/').count() == 2
            && self.branch.as_deref().is_none_or(branch_name)
            && self.step.is_none_or(|step| step <= 999)
            && match (self.step, self.total) {
                (_, None) => true,
                (Some(step), Some(total)) => total <= 999 && total >= step,
                (None, Some(_)) => false,
            }
            && self.idle_secs.is_none_or(|secs| secs <= 86_400)
    }
}

/// 照 git 引用名的规矩，再挡不可见字符和像密钥的串。
fn branch_name(name: &str) -> bool {
    (1..=MAX_BRANCH_BYTES).contains(&name.len())
        && name.chars().all(|character| {
            !character.is_control()
                && !character.is_whitespace()
                && !invisible(character)
                && !matches!(character, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
        && !name.starts_with(['/', '-'])
        && !name.ends_with(['/', '.'])
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && name
            .split('/')
            .all(|part| !part.starts_with('.') && !part.ends_with(".lock"))
        && !secret_like(name)
}

/// 格式字符（Unicode `Cf` 全部）和行段分隔符：看不见，能让界面上的分支名读成别的样子或夹带文字。
/// 网页那边按同一范围拒（`presenceModel.ts`），这里少收一个，那边就整组读不到。
fn invisible(character: char) -> bool {
    matches!(
        character,
        '\u{ad}' | '\u{600}'..='\u{605}' | '\u{61c}' | '\u{6dd}' | '\u{70f}'
            | '\u{890}'..='\u{891}' | '\u{8e2}' | '\u{180e}' | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}' | '\u{110bd}' | '\u{110cd}' | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}' | '\u{1d173}'..='\u{1d17a}' | '\u{e0000}'..='\u{e007f}'
    )
}

/// 常见密钥前缀后面跟一长串，或一整段无分隔的长串（字母、数字都有）。
fn secret_like(name: &str) -> bool {
    const PREFIXES: [&str; 9] = [
        "sk-",
        "ghp_",
        "gho_",
        "ghs_",
        "github_pat_",
        "glpat-",
        "xox",
        "akia",
        "eyj",
    ];
    let lower = name.to_ascii_lowercase();
    let prefixed = PREFIXES.iter().any(|prefix| {
        lower.match_indices(prefix).any(|(at, _)| {
            let boundary = lower[..at]
                .chars()
                .next_back()
                .is_none_or(|character| !character.is_ascii_alphanumeric());
            let tail = lower[at + prefix.len()..]
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                })
                .count();
            boundary && tail >= 12
        })
    });
    prefixed
        || name
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|run| {
                run.len() >= 32
                    && run.bytes().any(|byte| byte.is_ascii_digit())
                    && run.bytes().any(|byte| byte.is_ascii_alphabetic())
            })
}

/// 一份快照没被收下的原因；上一份都不变。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refused {
    /// `seq` 不比上一份大。
    Stale,
    /// 全部来源合计会超过上限。
    Full,
    /// 这一小时「新会话」的登记用完了；只含老编号的快照照收。
    Throttled,
}

/// 要登记的一组新会话：同项目、同一家的并成一条。
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Fresh {
    pub project: String,
    pub vendor: Vendor,
    pub count: usize,
}

/// 核过、可以落下的一份快照。
pub(super) struct Planned {
    seq: u64,
    kept: BTreeMap<String, Session>,
    /// 没放行的记录在原快照里的下标。
    pub rejected: Vec<usize>,
    /// 这次第一次出现的会话；要先登记成功才落下。
    pub fresh: Vec<Fresh>,
}

impl Planned {
    pub(super) fn accepted(&self) -> usize {
        self.kept.len()
    }
}

struct Source {
    instance: String,
    seq: u64,
    received: Duration,
    sessions: BTreeMap<String, Session>,
}

impl Source {
    fn expired(&self, now: Duration) -> bool {
        now.saturating_sub(self.received) >= EXPIRES
    }
}

/// 全部来源的现行状态。
#[derive(Default)]
pub(super) struct Presence {
    sources: BTreeMap<String, Source>,
    /// 每个来源最近一小时登记过的时刻。
    records: BTreeMap<String, VecDeque<Duration>>,
    denials: BTreeMap<(String, &'static str), Duration>,
}

/// 给人和 AI 看的一条；没带的字段不出现，编号不给。
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(super) struct Seen {
    source: String,
    vendor: Vendor,
    project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    state: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    step: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_action: Option<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idle_secs: Option<u64>,
    age_secs: u64,
}

impl Presence {
    /// 清掉过期的来源和用完的限频记录。
    pub(super) fn sweep(&mut self, now: Duration) {
        self.sources.retain(|_, source| !source.expired(now));
        self.records.retain(|_, times| {
            while times
                .front()
                .is_some_and(|time| now.saturating_sub(*time) >= HOUR)
            {
                times.pop_front();
            }
            !times.is_empty()
        });
        self.denials
            .retain(|_, time| now.saturating_sub(*time) < DENIAL_EVERY);
    }

    /// 核这份快照能不能收；不改任何状态。`allowed` 是能力集里的项目。
    pub(super) fn plan(
        &mut self,
        source: &str,
        snapshot: Snapshot,
        allowed: &[String],
        now: Duration,
    ) -> Result<Planned, Refused> {
        self.sweep(now);
        let current = self.sources.get(source);
        if current.is_some_and(|current| snapshot.seq <= current.seq) {
            return Err(Refused::Stale);
        }
        let mut kept = BTreeMap::new();
        let mut rejected = Vec::new();
        for (index, session) in snapshot.sessions.into_iter().enumerate() {
            if allowed
                .iter()
                .any(|project| project == session.project.as_str())
            {
                kept.insert(session.key.clone(), session);
            } else {
                rejected.push(index);
            }
        }
        let others = self
            .sources
            .iter()
            .filter(|(name, _)| name.as_str() != source)
            .map(|(_, source)| source.sessions.len())
            .sum::<usize>();
        if others + kept.len() > MAX_TOTAL {
            return Err(Refused::Full);
        }
        let mut groups = BTreeMap::<(String, Vendor), usize>::new();
        for (key, session) in &kept {
            if !current.is_some_and(|current| current.sessions.contains_key(key)) {
                *groups
                    .entry((session.project.as_str().to_owned(), session.vendor))
                    .or_default() += 1;
            }
        }
        let used = self.records.get(source).map_or(0, VecDeque::len);
        if used + groups.len() > RECORDS_PER_HOUR {
            return Err(Refused::Throttled);
        }
        Ok(Planned {
            seq: snapshot.seq,
            kept,
            rejected,
            fresh: groups
                .into_iter()
                .map(|((project, vendor), count)| Fresh {
                    project,
                    vendor,
                    count,
                })
                .collect(),
        })
    }

    /// 登记了一条「新会话第一次出现」。登记成功就算数，后面落不落下都占名额。
    pub(super) fn note_record(&mut self, source: &str, now: Duration) {
        self.records
            .entry(source.to_owned())
            .or_default()
            .push_back(now);
    }

    /// 整份替换；快照里没有的当场消失。
    pub(super) fn commit(&mut self, source: &str, instance: &str, planned: Planned, now: Duration) {
        self.sources.insert(
            source.to_owned(),
            Source {
                instance: instance.to_owned(),
                seq: planned.seq,
                received: now,
                sessions: planned.kept,
            },
        );
    }

    /// 这条拒绝要不要登记：同一（来源，原因）一分钟内只登记第一条。
    pub(super) fn denial_due(&mut self, source: &str, reason: &'static str, now: Duration) -> bool {
        let slot = (source.to_owned(), reason);
        match self.denials.get(&slot) {
            Some(time) if now.saturating_sub(*time) < DENIAL_EVERY => false,
            _ => {
                self.denials.insert(slot, now);
                true
            }
        }
    }

    /// 现有来源与推它的实例。
    pub(super) fn sources(&self) -> Vec<(String, String)> {
        self.sources
            .iter()
            .map(|(name, source)| (name.clone(), source.instance.clone()))
            .collect()
    }

    /// 按现行授权收窄：没有授权就整份删掉（撤了不留），项目被收回的那些条当场删。
    pub(super) fn restrict(&mut self, source: &str, allowed: Option<&[String]>) {
        match allowed {
            None => {
                self.sources.remove(source);
            }
            Some(allowed) => {
                if let Some(source) = self.sources.get_mut(source) {
                    source.sessions.retain(|_, session| {
                        allowed
                            .iter()
                            .any(|project| project == session.project.as_str())
                    });
                }
            }
        }
    }

    /// 现行状态；`project` 给了就只给那个项目的。
    pub(super) fn view(&self, now: Duration, project: Option<&str>) -> Vec<Seen> {
        let mut seen = Vec::new();
        for (name, source) in &self.sources {
            if source.expired(now) {
                continue;
            }
            let age = now.saturating_sub(source.received).as_secs();
            for session in source.sessions.values() {
                if project.is_some_and(|project| project != session.project.as_str()) {
                    continue;
                }
                seen.push(Seen {
                    source: name.clone(),
                    vendor: session.vendor,
                    project: session.project.as_str().to_owned(),
                    branch: session.branch.clone(),
                    state: session.state,
                    step: session.step,
                    total: session.total,
                    last_action: session.last_action,
                    idle_secs: session.idle_secs.map(|secs| u64::from(secs) + age),
                    age_secs: age,
                });
            }
        }
        seen.sort_by(|a, b| {
            (&a.project, a.vendor, &a.branch, &a.source)
                .cmp(&(&b.project, b.vendor, &b.branch, &b.source))
        });
        seen
    }
}

#[cfg(test)]
mod tests;
