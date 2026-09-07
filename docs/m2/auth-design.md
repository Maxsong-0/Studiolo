# M2 应用认证：分片实施设计

状态：2026-09-05 T-013真实Access迁移与持久会话已通过双平台契约验收，该片当时没有新增HTTP端点；后续T-018另加显式会话组合。
T-014安全保留/显式迁移已验收，见 [v2补充契约](../contracts/auth-retention.md)；T-015预配置TOTP因子基础已双平台契约验收，见[v3契约](../contracts/auth-totp.md)。
T-016恢复码与实际因子替换已双平台契约验收，见[v4契约](../contracts/auth-recovery.md)。
实际已交付 Interface 见 [持久会话契约](../contracts/auth-session.md)，下列完整认证入口仍为后续设计。
T-017可信本地CLI已验收；T-018应用会话HTTP已实现，[独立契约](../contracts/auth-http.md)为该片最终设计、最终验证见M2记录；下面广义草案不覆盖其已裁决范围。
权威等级与审计行为以 Vault 的《敏感动作枚举表》为准，新增动作须先登记再实现。
不把现有 Access 邮箱头或 Gateway capability 当成人类应用会话。

## 模块边界

Auth 独占私有身份存储，不能直接写 TaskEngine-owned ControlDb。
会话记录只存高熵令牌摘要；TOTP 种子在独立密钥平面，Auth 只存引用。
HTTP 不持有原始 SQLite 连接；时间、随机源、密钥读取和审计是可替换 Adapter。

- `provision(local_authority, command)`：可信本地 CLI 建立主人绑定和首次登记。
- `exchange(verified_access, request_origin)`：经验证身份换取应用会话。
- `authorize(verified_access, cookie, request_context, action)`：返回受限授权结果。
- `command(..., AuthCommand)`：确认登记、step-up、因子/恢复码轮换、恢复、吊销。
- `sessions(authorized_owner)`：不含令牌的会话元信息。

动作使用闭合枚举；请求不能自行指定认证等级。可信 Access 主体必须由现有验证器
生成，不可从普通 JSON/公开字段构造；包含 issuer、audience、sub、email、exp。
每次请求继续验 Access，并与应用会话绑定，未知或已吊销 Cookie 不自动换发新会话。

## 启动与恢复

未登记时，远端邮箱登录不能抢注主人、绑定 TOTP 或获得权限。
localhost 请求、转发头或布尔 `is_local` 不是本地授权证明。
真实首次登记与恢复码保管由主人完成，秘密不经普通聊天或同步档案。

T-016裁决为恢复码加预配置新因子OTP直接完成真实替换，不发独立恢复权限；成功后
吊销全部旧会话，必须重新登录并提权。没有手机和恢复码走独立可信本地流程，不允许邮箱自行移除 TOTP。
干净机 restore bootstrap 身份与以上流程仍完全独立。

## 会话与写请求

- 不透明随机 Cookie；生产 `__Host-`、Secure、HttpOnly、Path=/、无 Domain。
- 服务端同时检查 idle、absolute expiry、吊销及身份版本；轮换不延长绝对期限。
- 状态变更同时验证精确可信 Origin 与会话绑定 CSRF，拒绝缺失/null/重复安全头。
- 首次 exchange 使用独立 Origin/Fetch Metadata 检查，不能以未登录豁免。
- TOTP 验证、已用时间步、恢复码消费、失败限流必须持久事务化；并发和重启不能重放。
- 来源地址只允许固定可信代理链产生，不信任任意 X-Forwarded-For。

## 审计与跨库失败

已定：L3、立即备份与单页恢复失败关闭；吊销和普通内容 mutation 在审计不可用时仍执行。
有效会话普通读取不新增审计依赖；认证存储不可用则仍拒绝认证。

T-013 已在权威表登记新会话签发失败关闭、吊销先持久生效再通过 outbox 补记，
登录拒绝/到期用有界白名单补记。T-015/T-016已裁决预配置登记、step-up、恢复码换发与实际因子替换；消费只收紧权限，双回执后原调用重验才激活。
登录失败计数必须先持久化，审计故障不得重置。
AuthDB 与 AuditLedger 不是同一事务：稳定事件 ID、pending/outbox 与响应发布状态
必须覆盖崩溃窗口；成功消费的验证码不能因补记重试再次授权。
审计载荷从白名单字段构建，不能仅依靠字段名黑名单处理原始请求正文。

## 参数分片裁决（未作为部署配置生效）

- idle 30分钟；absolute 12小时且不超过建立时 Access 凭证期限。
- T-015已裁决L3与登记pending5分钟；L3从消费时起并受会话/Access封顶，操作执行时重新检查。
- T-015已裁决TOTP 30秒、6位、±1步，成功计数器只能前进。
- T-016否决独立10分钟恢复Cookie；恢复/换发pending5分钟并受会话与Access封顶。
- T-015已裁决5次受理验证/15分钟，成功也不退款；账号与可信来源两维持久化。
- T-016恢复码10枚，每枚独立32字节随机输入派生128位秘密；换发旧码全失效。恢复验证独立5次/15分钟，换发不扣猜码预算。

T-015/T-016部分已在权威表记录最终工程裁决，不冒称用户逐项批准参数。
诊断导出等其他待定动作不由认证模块静默决定。

## T-013 已验收的实现约束

- `AccessIdentity` 私有字段；真实 JWT 验证为生产唯一构造途径，保留 configured audience 与到期。
- 显式可信 owner/TTL 配置；固定 `auth.sqlite3`，不共用 ControlDb 或审计连接。
- `check` 本片只验证读取，不能代替未来内容写请求的 Origin + CSRF 授权链。
- 新会话先 pending，审计 outcome 仅表示授权决定通过；原签发调用重新检查后才激活并返回秘密。
- idle 从最终激活起算，pending 仍受预检确定的绝对期限约束；持久激活标记与终态事件关联校验防简单状态翻回复活。
- 重启/补记绝不激活 pending；也不在新实例 open 时误废弃另一个存活实例的 pending。到自身期限后清理。
- 激活提交与客户端收到凭证不能原子交付；未交付孤儿会话会到期，不承诺无孤儿。
- 固定虚构应用/邮箱配置的测试 verifier 只有非默认 `test-support`；发布用 `production` 特征，混用由编译器拒绝。
- T-013当时不注册真实身份，不提供 HTTP/Cookie、首次登记 CLI、TOTP/L3 或恢复码；T-015后新增预配置TOTP/L3库接口，仍无真实登记或HTTP接线。
- T-013最初累计所有状态会话上限1024；T-014已验证安全回收后累计超过1024仍可签发。该数字现在指保留行上限，不是并发上限；T-018专用已登记签发前执行一轮维护，真实生产装配与完整认证仍待后续。

## 本地契约验收

1. Access 各字段错误/过期/伪造头拒绝；远端无抢注路径。
2. Cookie/Origin/CSRF、fixation、单个/全部吊销、身份版本、双过期边界。
3. RFC 向量、时间步边界、OTP 并发/重放/回退/重启。
4. 恢复码并发单次、轮换失效、恢复权限隔离；Debug/审计无秘密。
5. 限流跨 Cookie/重启持久；计数写失败不能继续验证。
6. 审计、Auth 提交、响应发布前后注入失败，提权不泄漏，吊销不回滚。
7. 仅临时私有存储、模拟时钟/随机源与虚构主体；原 M1 仍只读 405。

## 官方依据

- [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)
- [OWASP CSRF Prevention](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)
- [RFC 6238](https://www.rfc-editor.org/rfc/rfc6238.html#section-5.2)
