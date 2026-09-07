# 应用会话 HTTP 契约（T-018）

2026-09-05。T-018已通过macOS519/Linux527项完整回归及生产正反检查；范围见[M2](../m2/README.md)。
权威端点等级与失败行为已先登记 Vault《敏感动作枚举表》T-018。

## 装配与权限

`router_with_sessions(vault, assets, label, port, access, Arc<SessionManager>)`
是独立显式组合入口。两个旧 `router` / `router_with_access` 保持原来的
M1 GET/HEAD/405 行为，不增加 Cookie 要求、不隐式开放新认证端点。

可信启动层负责安全打开既有 Auth/Audit（使用 `open_existing`），HTTP 不读取配置、
秘密环境或文件，不直接持 SQLite、种子或首次登记句柄，不初始化、迁移、抢注主人。
新入口仍只适用于 IPv4 loopback，Tunnel 改写 Host 为固定 `127.0.0.1:port`。
该拓扑不是 localhost 身份豁免或 OS 隔离证明。

所有成功请求，包括静态引导，每次重新验证真实 Access 签名与声明。
新模式的全部 `/api/` 内容路由还须应用会话 Cookie 与 CSRF 配对检查；
仅首次签发免除尚不存在的应用 Cookie，不免除 Access、主人和已登记门槛。
HTTP 输入中的 identity/owner/role、转发邮箱、IP 或来源布尔均不产生授权。

## 封闭端点

| 请求 | 正文 / 结果 |
|---|---|
| `POST /api/v1/sessions` | 严格 `{}`；无应用 Cookie 才签发；201 返回自身元信息和 `csrf_token`，Cookie 单次交付 |
| `GET /api/v1/session` | 200 返回 `{session, csrf_token}`，不返回 session token |
| `GET /api/v1/sessions` | 200 返回 `{sessions:[...]}`，最多原 Auth 的1024条非秘密元信息 |
| `POST /api/v1/sessions/revoke` | `{"scope":"current"}`、`{"scope":"all"}` 或 `{"scope":"session","session_id":"..."}`；200 返回 `{revoked:N}` |

新认证端点不接受查询参数、HEAD、OPTIONS 或额外方法。未知字段、重复 JSON 字段、
错误 scope/目标组合拒绝。指定目标复用原 Auth 会话标识解析，不接受路径或 SQL。
元信息固定为 `id/state/created_at_ms/last_seen_at_ms/absolute_expires_at_ms`；
状态使用小写名称，不能把返回快照当成后续请求的授权凭据。

## 浏览器请求与秘密交付

- 应用 Cookie 固定 `__Host-studiolo-session=token~csrf`：两段沿原规范编码，不 URL 解码。
- 固定 `Secure; HttpOnly; SameSite=Strict; Path=/`，无 Domain、无延期 Max-Age；
  单 Cookie 避免两个秘密分别交付的撕裂，服务端期限仍为权威。
- CSRF 在 AuthDB 仅存摘要，GET 不是从 DB 重建秘密；先由 `check_with_csrf`
  校验 Cookie 内双秘密与当前会话配对，再向同源脚本回送 CSRF。session token 不进 JSON。
- 吊销必须显式 `X-CSRF-Token` 与 Cookie 内配对值恒时比较，Auth 仍检查持久摘要；
  自动携带的 Cookie 不能替代请求头的 CSRF 证明。网页应仅把返回 CSRF 放在内存。
- 全部新模式 API 要求 `X-Studiolo-Request: 1`、`Sec-Fetch-Site: same-origin`、
  `Sec-Fetch-Mode: cors|same-origin`、`Sec-Fetch-Dest: empty`，不接受 `Sec-Fetch-User`。
- GET 正常可不带 Origin；若出现必须精确匹配配置。POST 必须精确 HTTPS Origin。
  缺失 Fetch Metadata、null Origin、same-site、跨站或导航请求均拒绝；无宽松旧浏览器回退。
- 重复安全头、重复应用 Cookie、畸形或超预算 Cookie 拒绝。其他合法 Cloudflare Cookie
  不参与身份判断。Cookie 总值最多8KiB/64对；JWT沿 Access16KiB原有上限。
- 新 POST 只接受 `application/json`，实际正文最多1KiB，读取最多2秒；不是只信Content-Length。
- 所有经过应用路由的成功与失败响应带 no-store、nosniff、no-referrer、CSP、CORP，以及新模式的
  `Vary: Origin, Sec-Fetch-Site`；不开放 CORS，不将凭据放 URL、重定向或日志。

## Auth 专用签发与维护

`issue_enrolled(identity)` 不是 HTTP 中 `state → issue` 两次调用的拼装。
先在 Auth 的原 `run` 中验证真实主人/Access及已确认登记（同时观察时间/到期），
然后显式执行一次既有 `maintain_retention`，再复用原签发状态机。
后续每个签发事务，尤其最终激活 CAS，仍重验登记及权限。
未登记、只有消费/pending或缺少有效登记证明不能换取远端会话。

原 `issue` 语义保留，不隐式维护；HTTP 只调用专用已登记入口。
每次新登录最多原维护的一批16终态/16 epoch，不追着 More 无限循环，不重放失败签发。
维护确认原证据与稳定许可后才删除；active、pending、未交付证据仍受保护。
原签发已要求审计，本专用入口还会承担一轮维护成本；维护失败也不签发新权限。
没有自动定时后台维护，低流量时可直到下一次签发才观察并回收已到期会话。

## 失败、并发与取消

- 无效 Access / 失效配对返回401；同源与显式 CSRF 失败403；尚未登记（含只有首次登记pending）或
  已带应用 Cookie 的重复签发409。无效 Cookie 不在同一请求自动换发。
- JSON格式400、媒体类型415、正文过大413、读取超时408；认证存储/审计/执行池暂不可用503。
- Cookie语法/8KiB或64对预算错误400；总请求头最多128项/32KiB，超出431；重复安全头403。
  已登记之后的step-up/recovery pending不单独禁止普通会话签发；本片并未新增因子HTTP入口。
  固定脱敏错误不包含身份、路径、Cookie、CSRF、JWT或数据库细节。
  503明确提示操作可能已生效、先核对状态，不提示盲目重放授权或撤权。
- 配对被拒绝可清除本应用 Cookie；Access错误、CSRF错误、重复签发和503不清仍可能有效的Cookie。
  当前/全部或指定自身会话吊销成功才清本 Cookie，不删除 Cloudflare 或其他 Cookie。
- 撤权沿原 Auth 原子撤权/outbox后补记；审计暂不可用不反转已持久撤权。
  普通读取不新增审计依赖，认证存储不可用仍失败关闭。
- 同步数据库工作在有界 blocking 执行槽中运行，许可持有至工作结束；客户端取消
  不等于撤回事务，也不提前释放仍在运行的工作许可。
  同一AuthHttp克隆共享两个执行槽；慢审计工作仍可暂时占满这些槽，不承诺读取有独立执行通道。
- 激活提交与 HTTP 交付不原子：响应丢失可以留下孤儿会话；吊销提交后连接断开也可能已生效。
  不承诺任何错误都回滚，不自动重试原授权、重新登记或清库。

## 仍未交付

T-018本片是路由组合与协议契约，当时未改变二进制；T-019另增[显式安全启动](auth-startup.md)，保留两个旧模式，不切换线上服务。
真实部署配置、前端登录/退出流程、HTTPS浏览器 Cookie 与手机验收、可信代理来源、
TOTP/恢复码 HTTP、秘密交付和真正主人配对仍须后续独立实现与验收。
不提供网络登记、维护、邮箱移除因子、清库重登或整库 restore 身份。
完整 M2 的编辑/CAS/Sync/备份恢复/运维与真实发布门槛不因本片完成而免除。

## 官方依据

- [OWASP CSRF：Origin、Fetch Metadata和显式请求证明](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)
- [MDN Cookie属性与Host前缀](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie)
- [Axum正文限制](https://docs.rs/axum/0.8.9/axum/extract/struct.DefaultBodyLimit.html)
