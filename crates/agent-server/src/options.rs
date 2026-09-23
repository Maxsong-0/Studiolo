//! 封闭的命令行参数解析：只接受已知参数，不读环境变量或配置文件。

mod parser;

use std::path::PathBuf;

pub use parser::parse;

/// 用法说明。
pub const USAGE: &str = "studiolo-agent：Studiolo 供给面 MCP 主外壳（M3）\n\n\
用法：\n  \
studiolo-agent serve (--vault </绝对路径/vault> | --replica-root </绝对路径/副本>) [--replica-max-age-secs 300] --allow <相对目录> [--allow …] --data </绝对路径/私有目录> [--policy </绝对路径/策略私有目录>] [--board-socket </绝对路径/黑板socket>] [--index-cache </独立700目录>] [--port 4400]\n  \
studiolo-agent admin identity-upgrade --data <私有目录>（启用子实例；离线显式升级）\n  \
studiolo-agent admin init --data </绝对路径/私有目录>\n  \
studiolo-agent admin type-issue --data <私有目录> --type <claude|codex|grok|…>\n  \
studiolo-agent admin type-revoke --data <私有目录> --type <类型名>\n  \
studiolo-agent admin types --data <私有目录>\n  \
studiolo-agent admin reads --data <私有目录> [--type T] [--instance I] [--since RFC3339] [--until RFC3339] [--path-prefix P] [--limit N]\n  \
studiolo-agent admin rebuild-read-index --data <私有目录>\n  \
studiolo-agent admin audit-upgrade --data <私有目录>   # T-012 v1 账本 → 分段格式，一次性、单向\n  \
studiolo-agent admin outages --data <私有目录>\n  \
studiolo-agent admin instances --data <私有目录> [--limit N]\n  \
studiolo-agent admin policy-init --policy <策略私有目录>\n  \
studiolo-agent admin policy-bootstrap --policy <策略私有目录> --data <私有目录> --kind <rules|skills|capability_set> --subject <名> --file </绝对路径/正文>（只能写首版；须在真实前台控制终端运行）\n  \
studiolo-agent admin policy-status --policy <策略私有目录>\n\n\
私有目录及其 audit/ 子目录须预先以 0700 创建；服务只监听 127.0.0.1，客户端须带 Authorization: Bearer <类型 token>。";

/// `serve` 参数。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServeOptions {
    /// vault 根（绝对路径）。
    pub vault: Option<PathBuf>,
    /// 不可变副本根；与 vault 二选一。
    pub replica_root: Option<PathBuf>,
    /// 副本新鲜度阈值，60..=86400 秒。
    pub replica_max_age_secs: u64,
    /// 白名单目录（相对 vault 根）。
    pub allow: Vec<String>,
    /// 私有数据目录（绝对路径）。
    pub data: PathBuf,
    /// 策略平面私有目录（绝对路径）；缺省为 M3 兼容模式（无能力集、无吊销表）。
    pub policy: Option<PathBuf>,
    /// canonical 单写者的私有黑板 Unix 入口；必须同时配置策略。
    pub board_socket: Option<PathBuf>,
    /// 可丢弃的派生索引缓存；显式独立私有目录，不能与源或身份目录重叠。
    pub index_cache: Option<PathBuf>,
    /// loopback 端口。
    pub port: u16,
}

/// `admin` 子命令。
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(missing_docs)] // 字段含义见 USAGE；每个变体只有私有目录与显式过滤项
pub enum AdminCommand {
    /// 初始化私有库与审计账本。
    Init { data: PathBuf },
    /// 签发或轮换类型 token。
    TypeIssue { data: PathBuf, agent_type: String },
    /// 吊销类型。
    TypeRevoke { data: PathBuf, agent_type: String },
    /// 列出类型。
    Types { data: PathBuf },
    /// 四维读取审计查询。
    Reads {
        data: PathBuf,
        agent_type: Option<String>,
        instance: Option<String>,
        since: Option<String>,
        until: Option<String>,
        path_prefix: Option<String>,
        limit: usize,
    },
    /// 从账本重建读取索引。
    RebuildReadIndex { data: PathBuf },
    /// 把 T-012 的 v1 审计账本显式升级为分段格式（一次性、单向；v2 或校验不过一律拒绝）。
    AuditUpgrade { data: PathBuf },
    /// 显式升级子实例身份关系。
    IdentityUpgrade { data: PathBuf },
    /// 列出审计不可用窗口。
    Outages { data: PathBuf },
    /// 列出已注册实例。
    Instances { data: PathBuf, limit: usize },
    /// 初始化策略平面私有库。
    PolicyInit { policy: PathBuf },
    /// 首版 bootstrap（真实控制终端门禁；只允许从未有过版本的 subject）。
    PolicyBootstrap {
        policy: PathBuf,
        data: PathBuf,
        kind: String,
        subject: String,
        file: PathBuf,
    },
    /// 打印 effective 摘要、现行各项与生效中的吊销。
    PolicyStatus { policy: PathBuf },
}

/// 顶层命令。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// 打印用法。
    Help,
    /// 启动服务。
    Serve(ServeOptions),
    /// 管理子命令。
    Admin(AdminCommand),
}
