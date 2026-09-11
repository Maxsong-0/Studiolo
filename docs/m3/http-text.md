# HTTP只读文本供给（T-037，2026-09-10）

依据Vault ADR-015。给支持请求头但不支持MCP的fetch客户端供给路牌和资料，
与MCP使用同一类型身份、白名单、读取审计、索引、完整副本验证及预算。
这是在线只读外壳；服务离线时的CLAUDE.md/skills参考由后续T-038
[客户端投影](client-projection.md)提供，发布状态独立验收。

## 入口与身份

沿用`studiolo-agent serve`的配置、私有库和回环端口，不新增监听地址或权限配置。
所有请求带`Authorization: Bearer <类型令牌>`，只接受一个Authorization头。
不接受URL内的令牌，也不使用Cookie或浏览器CORS授权。

| GET入口 | 参数 | 成功结果 |
|---|---|---|
| `/read/handshake` | 无 | 六项路牌；HTTP续读入口和头部说明计入≤2000 token硬预算 |
| `/read/entries` | `project`，精确白名单根 | 代次、条目种类、逻辑路径与字节数 |
| `/read/note` | `path`，笔记逻辑路径 | `revision`、托管模式新鲜度与正文，笔记上限4 MiB |
| `/read/search` | `project`、非空`text`；可选`mode=full_text或path` | 最多100个匹配的路径、行号与片段 |

查询参数须按UTF-8进行URL编码；原始查询总长最多8192字节。
重复、未知、缺失必填项、错误百分号编码、非UTF-8和控制字符一律拒绝。
只支持GET；已认证的HEAD/POST/PUT/PATCH/DELETE/OPTIONS均405，HEAD不会新建实例。

首次GET握手不带会话头，服务器登记一个实例，成功响应携带：

- `Studiolo-Session-Id`：续读时必须原样放回请求头；仅是会话关联句柄，不能替代Bearer。
- `Studiolo-Instance-Id`：非秘密的实例身份，可用于读取审计关联；不能当作会话句柄。

有效会话再次握手复用同一实例；其他三入口不隐式注册。会话绑定认证类型，
跨类型借用、缺失、未知或过期会话均401；格式不合法或重复会话头400。
吊销类型后，既有会话和新请求都失效。服务重启后需重新握手。

## 有界会话与失败清理

- 最大128个会话，每类型最多32个，初始化中的预留也计入容量。
- 每分钟最多32次初始化尝试；失败及取消也计入尝试预算，避免故障时无限登记。
- 最多16个并发供给请求；达到请求、会话或初始化上限返回429和`Retry-After: 60`。
- 闲置15分钟或创建满60分钟失效，使用单调时钟；仅成功供给推进闲置期限，拒绝不续命。
- 初始化名额用守卫预留；供给、最终身份复核及响应头准备均成功后才发布。
  失败或future取消会释放未发布名额。请求结束后不保留响应正文。

## 授权、审计与副本

MCP和HTTP只包装协议，实际调用同一组`AgentSession::text_*`方法。
正文仍经既有只读根或完整校验的托管副本供给；没有第二套文件读取或授权实现。
HTTP实例登记、路牌交付、内容读取和拒绝进入既有账本；正文/目录/搜索继续使用
`content.read`事件与四维派生索引，审计先于响应。

托管模式响应含来源generation、摘要、捕获/接收时间和年龄。
stale或sync_failed保持显式状态，允许读取最后完整副本；不能据此开展实时协作。
副本漂移或控制文件无效则503，拒绝内容，不回退到旧模式。
审计不可用时握手503，只给outage标识与读写全拒，不含六项路牌；其他读取同样503。
错误正文只给固定类别，不回显查询、令牌或宿主路径。

所有HTTP成功和失败响应使用`text/plain; charset=utf-8`、`Cache-Control: no-store`、
`Pragma: no-cache`、`X-Content-Type-Options: nosniff`及`Referrer-Policy: no-referrer`。
不把Markdown渲染成HTML，不输出开放CORS头，也不返回可被缓存绕过审计的304。

| HTTP状态 | 语义 |
|---|---|
| 200 | 已完成授权、完整性检查与审计的文本 |
| 400 | 参数、会话头或白名单路径无效 |
| 401 | 凭据或会话无效；应检查凭据或重新握手 |
| 404 | 未知入口或白名单内笔记不存在 |
| 405 | 已认证但方法不是GET，`Allow: GET` |
| 429 | 达到有界配额，稍后重试 |
| 503 | 身份/会话管理、审计、副本或预算等供给前置条件不可用 |

## 客户端示例

先在获准的本机独立实例上运行新版本，并从安全来源设置`STUDIOLO_AGENT_TOKEN`环境变量。
此例只打印授权路牌和正文，不打印Bearer或会话句柄；项目和路径按实际白名单修改。

```python
import os
import urllib.parse
import urllib.request

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, *args, **kwargs):
        return None

opener = urllib.request.build_opener(NoRedirect())

def read_text(response):
    payload = response.read(5 * 1024 * 1024 + 1)
    if len(payload) > 5 * 1024 * 1024:
        raise ValueError("响应超过客户端读取预算")
    return payload.decode("utf-8")

base = "http://127.0.0.1:4400/read"
headers = {"Authorization": "Bearer " + os.environ["STUDIOLO_AGENT_TOKEN"]}
with opener.open(urllib.request.Request(base + "/handshake", headers=headers), timeout=15) as response:
    headers["Studiolo-Session-Id"] = response.headers["Studiolo-Session-Id"]
    print(read_text(response))
query = urllib.parse.urlencode({"path": "项目/Studiolo/00-总览/INDEX.md"})
with opener.open(urllib.request.Request(base + "/note?" + query, headers=headers), timeout=15) as response:
    print(read_text(response))
```

实际客户端须限制响应读取大小、设置超时，并将凭据和会话头限制在可信目标地址；
不要把真实令牌粘到URL、聊天、日志或项目档案。

## 验收与发布状态

实现包含真实loopback四入口与MCP回归、审计与身份拒绝、方法和参数边界、
托管副本更新/陈旧/同步失败/漂移，以及会话容量、单调时钟期限、并发预留与取消清理测试。
本片新增21项：8项输入/会话边界、11项HTTP真实loopback、2项HTTP/MCP共用预算。

- macOS：agent-server与supply共53项（首轮51项，新增预算2项另跑），架构守卫25项，合计78项通过。
- Linux Docker aarch64：同一组53项与25项架构守卫全部通过，合计78项；不作为服务器x86_64原生验收。
- 两端全工作区严格Clippy、格式检查及`studiolo-agent` release构建通过；既有MCP契约全部保持通过。
- macOS最初在沙箱内无法绑定随机loopback端口，3项副本夹具在启动前因EPERM退出；
  允许本机监听后重跑供给测试全过。该失败不记作HTTP功能通过，也未修改测试绕过监听。
- 验收日志：`/private/tmp/studiolo-t037-macos-tests.log`、`studiolo-t037-clippy.log`、
  `studiolo-t037-release-build.log`及`studiolo-t037-linux.log`。

2026-09-11 已随 T-040 合并 main `cbfb5df` 并部署 `m3-t040-20260910-read01`。
系统阶段/正式服务与独立 HTTPS 公网复验均通过，包含四入口成功及 14 个拒绝场景；
上述开发测试记录保留，服务器原生、系统切换和公网证据见 [T-040 发布记录](read-supply-release.md)。
