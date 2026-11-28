//! 常量表：会话协议版本、MCP tool 清单与 HTTP 续读入口（后两者进入路牌第 6 项）。

use rmcp::model::ProtocolVersion;

/// 本片只支持带会话的协议版本：实例身份绑定 MCP 会话（SEP-2567 无会话模式另行设计）。
pub(super) const SESSION_VERSIONS: &[ProtocolVersion] = &[
    ProtocolVersion::V_2025_11_25,
    ProtocolVersion::V_2025_06_18,
    ProtocolVersion::V_2025_03_26,
];

/// 客户端要的版本不在表里时应答的那一版；`get_info` 报的也是它，rmcp 拿那一格当兜底。
pub(super) const FALLBACK_VERSION: ProtocolVersion = ProtocolVersion::V_2025_11_25;

/// `initialize` 会应答的版本：要的在表里就是它，否则退到 [`FALLBACK_VERSION`]。
///
/// 应答的版本由 rmcp 在处理器返回之后按同一条规则定（那个函数不公开），登记审计写在这之前，所以先算一遍；
/// 两边不许走岔，由 `mcp_contract` 拿真实应答逐版本对照。
pub(super) fn answered_version(requested: &ProtocolVersion) -> ProtocolVersion {
    if SESSION_VERSIONS.contains(requested) {
        requested.clone()
    } else {
        FALLBACK_VERSION
    }
}

/// tool 清单与一句说明（进入路牌第 6 项）。
pub const TOOLS: &[(&str, &str)] = &[
    ("handshake", "重取冷启动路牌"),
    (
        "board",
        "读写黑板：project、operation、request；写入需有效能力集",
    ),
    ("list_entries", "列出一个白名单项目的条目：project"),
    (
        "read_note",
        "读一篇笔记：path[、lines=a-b|heading=标题|outline=true]",
    ),
    (
        "search",
        "全文或路径搜索：project、text、mode=full_text|path",
    ),
    (
        "policy_read",
        "读策略平面 effective：kind=rules|skills[、subject]",
    ),
    (
        "propose_desired",
        "提交 desired 提案（永不自动生效）：kind、subject、body",
    ),
    (
        "list_proposals",
        "核对本类型仍待处理的提案：[kind、subject]",
    ),
];

/// HTTP 续读说明放在入口名内，使各裁剪等级均保留并纳入预算。
pub(super) const HTTP_TOOLS: &[(&str, &str)] = &[
    (
        "所有 GET 均带 Authorization: Bearer <类型令牌>；续读带 Studiolo-Session-Id: <握手响应头值>",
        "会话过期后重新握手",
    ),
    ("GET /read/handshake", "首次握手或重取冷启动路牌"),
    ("GET /read/entries?project=…", "列出白名单项目条目"),
    ("GET /read/note?path=…", "读取白名单笔记"),
    (
        "GET /read/search?project=…&text=…&mode=full_text|path",
        "查询参数需 URL 编码",
    ),
    (
        "GET /read/policy?kind=rules|skills[&subject=…]",
        "读策略平面 effective（提案只经 MCP）",
    ),
];
