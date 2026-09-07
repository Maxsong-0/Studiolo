# T-009 · Cloudflare 域名示例入口

2026-09-04：用户要求通过 Tunnel 发布到 `studiolo.maxsong.me`，并明确同意单邮箱门禁。
**Access、独立鉴权服务和 DNS/路由已部署；真实登录与公网功能验收尚在进行。**
只接虚构 `example-vault`，不接真实笔记，不提供写入或 Runner，不代表完整 M1/M2 完成。

## 已有资源

- 服务器：`maxsong@192.168.100.169`。原 [T-008 回环示例](server-preview.md) 的
  `studiolo-preview.service` / 4318 / native02 保持不变。
- 新 Tunnel：`studiolo-preview`，ID `4c425eda-1a83-49f3-bee0-7014138d0886`。
- `studiolo-tunnel.service` active/enabled，4 条连接就绪；健康端点仅 `127.0.0.1:49319/ready`。
- cloudflared 2026.8.3 官方 Linux amd64 程序，SHA-256：
  `f29324fe934d1e100617484c78deef803c4dc2cd351d645bbde42e96b4fccc5e`。
- 程序位于 `/home/maxsong/.local/share/studiolo-tunnel/cloudflared-2026.8.3`；
  连接凭据位于 `/home/maxsong/.config/studiolo-tunnel/token`，权限 0600，父目录 0700。
  不读取、打印、归档或提交其内容。临时本机副本已删除。
- Access 策略 `Studiolo owner`，ID `18a9b3f8-b9b0-4131-8173-e04d679c3982`，
  单邮箱 Include/Allow，无 Bypass。用户明确授权后完成应用创建，未绕过审批。
- Access 应用 `Studiolo Preview`，ID `80135c30-758a-40bc-8e8a-46dc4e758e84`；
  AUD 为 `ba0f64bb17dbf4ce9d4086ffa2fb7d4b68044cbe5531fb4ac3080cad4d441071`（非秘密）。
- 新服务 `studiolo-access.service` active/enabled，只监听 `127.0.0.1:4319`，
  发布目录 `/home/maxsong/.local/share/studiolo-preview/releases/m1-t009-20260904-access01/`。
  原服务与连接器未重启，新服务首次核验 NRestarts=0。

## 已部署入口

公网 HTTPS → 目标整域的 Access 邮箱验证码门禁 → 专用 Tunnel → 回环 4319 的
`studiolo-access.service`。不开放服务器入站端口，不给公网隧道连接到无鉴权的 4318。

- Access 应用 `Studiolo Preview`，只允许 `maxsong5020@gmail.com`，24 小时会话，
  仅 onetimepin，HttpOnly、SameSite=Lax。现有其他应用/隧道/策略不变。
- 配置固定 team issuer `https://tengqing.cloudflareaccess.com`；AUD 必须从创建后的
  Studiolo 应用读取，不能使用策略 ID、应用 ID 或别的应用 AUD 替代。
- origin service `http://127.0.0.1:4319`，HTTP Host Header 精确 `127.0.0.1:4319`。
  应用仅接受配置的公共 HTTPS Origin；不信任 X-Forwarded-*，不删除 Origin 或放开 CORS。
- 四个 CLI 参数必须成组：`--public-origin`、`--access-team`、`--access-audience`、
  `--access-email`；缺项或非法值失败关闭，不静默回退本机模式。
- `studiolo-access.service.in` 使用独立发布包与必需的
  `/home/maxsong/.config/studiolo-access/access.env`；该文件仅保存非秘密 AUD/邮箱。

## 身份与请求契约

入口层集中验证 `Cf-Access-Jwt-Assertion`，只接受固定 team HTTPS JWKS 签名的 RS256
应用令牌；检查 kid、issuer、audience、时间、非空 subject 与精确邮箱。身份仅来自验签声明，
裸邮箱头、Cookie、Bearer 或转发身份都不能替代它。静态文件与所有 API 同样受保护。

JWKS 有界缓存、按 kid 选钥、未知 kid 刷新限频；禁止重定向和环境代理，过期缓存取钥失败
不继续使用旧密钥。拒绝重复安全头。缺失/非法身份 401，取钥不可用 503，同源策略失败 403；
写方法仍 405。失败响应也有 no-store、CSP、nosniff 等安全头。

默认 `router()` 本机模式保持原有约束。显式 Access 模式无本机身份豁免；仅首页 `/` 的
用户发起顶层导航可从外站进入，跨站 API、资源和 iframe 不放行。Vault 只读 Adapter 不变。

## 验证与下一步

本机 macOS 189 项 Rust 测试、格式与严格 Clippy 通过；服务器 Linux 原生 33 个程序
共 197 项测试通过，含新增鉴权/密钥缓存/CLI/HTTP 契约与路径安全回归。
同发布包前端 28 项测试及生产构建通过。发布包全文件校验通过，SHA256SUMS 文件摘要：
`4097c3c4577651fd19d0db74125ce1a8dbb2bf0181a7e170bcec3598e5cd7c7e`；
应用二进制摘要：`3d2413bc616b712d988a98f0fca3d652937b7faa5904a799cbb165d480ecfc34`。

服务器 17 项 HTTP 检查通过：8 个入口无身份均 401、非法 JWT/裸邮箱/Cookie 不放行、
错误 Host/Origin/跨站 API 403、POST 405；旧 4318 首页仍 200。
时间同步正常，真实 JWKS 初始化成功，服务文件系统只读与禁止提权限制生效。
连接器已收到 version=1 的唯一整域规则和 404 catch-all，Host 覆写精确正确。

示例源目录所有文件哈希与目录清单指纹在启动和检查前后未变，清单摘要：
`84450120cdbd8375f567a2a655ba516412104fd99e952fdccec80a55fbfda306`。

尚待：真实浏览器未登录门禁、指定邮箱 OTP 登录、静态资源/搜索/导航/刷新验收。
Cloudflare、Google 及本机配置的 Tailscale DNS 均已返回 A 记录；系统解析仍只有 IPv6，浏览器报
ERR_ADDRESS_UNREACHABLE，普通缓存刷新未解决。未改 DNS 配置或重启共享网络服务。
通过已核实公网 IP 的 HTTPS 诊断证书有效，服务器和本机无 Cookie 请求均受到 Cloudflare
challenge（不是应用鉴权失败），未关闭边缘保护。已请用户在浏览器自行完成 OTP；
验证码不交给 agent。不能用 197 项原生测试替代这项真实公网验收。

回滚只停止 `studiolo-tunnel.service` 或撤回本任务专用路由；4318 SSH 运维访问继续可用。
不删除发布包或其他 Cloudflare 资源，不改全账户访问设置。

## 官方依据

- [先创建 Access 再发布路由](https://developers.cloudflare.com/cloudflare-one/access-controls/applications/http-apps/self-hosted-public-app/)
- [JWT 验签与应用 AUD](https://developers.cloudflare.com/cloudflare-one/access-controls/applications/http-apps/authorization-cookie/validating-json/)
- [连接凭据文件参数](https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/configure-tunnels/run-parameters/)
- [登录 Cookie 设置](https://developers.cloudflare.com/cloudflare-one/access-controls/applications/http-apps/authorization-cookie/)
