//! 客户端自报的名字和版本（MCP `initialize` 的 `clientInfo`）：没人验过，只进登记审计当显示。
//!
//! 字段私有、没有读取方法：清洗完唯一的去处是登记审计，授权判断拿不到它。
use rmcp::model::Implementation;
use studiolo_policy::invisible;
use studiolo_supply::audit::Registration;

/// 名字、版本各最多这么多个字符；截断的末位换成省略号，看得出不是全文。
const SHOWN_LIMIT: usize = 64;

pub(super) struct ClientReported {
    name: String,
    version: String,
}

impl ClientReported {
    pub(super) fn new(info: &Implementation) -> Self {
        Self {
            name: shown(&info.name),
            version: shown(&info.version),
        }
    }

    /// 连同服务器应答的协议版本交给登记审计。
    pub(super) fn registration<'a>(&'a self, answered: &'a str) -> Registration<'a> {
        Registration::mcp(answered, &self.name, &self.version)
    }
}

/// 去掉控制字符和看不见的字符（后者能让名字显示成别的样子），再截到上限。
fn shown(raw: &str) -> String {
    let mut kept = raw
        .chars()
        .filter(|character| !character.is_control() && !invisible(*character));
    let mut text = kept.by_ref().take(SHOWN_LIMIT).collect::<String>();
    if kept.next().is_some() {
        text.pop();
        text.push('…');
    }
    text
}
