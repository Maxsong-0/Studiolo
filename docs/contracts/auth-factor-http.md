# 因子 HTTP 与可信 Unix 入口（T-024）

本契约扩展[固定槽运行装配](auth-factor-runtime.md)及[会话 HTTP](auth-http.md)。
测试结果另记[M2验证记录](../m2/README.md)；接口存在不代表真实秘密、手机或发布已验收。

## 显式启动模式

```text
studiolo-local-server --vault /approved/canonical --assets /approved/web \
  --session-config /approved/private/owner.json \
  --public-origin https://notes.example.invalid \
  --factor-socket /approved/proxy/http.sock --canonical
```

以上仅为虚构配置示意。`--factor-socket`要求会话配置，禁止同时指定`--port`；整个应用
只监听这一Unix socket。旧本机、Access或会话TCP模式不安装三条因子路由，也不读种子环境。
可选`--replacement-seed-ref`只指定非秘密候选引用，不能从请求配置候选或登记新主人。

秘密只取固定`STUDIOLO_TOTP_SEED_HEX`和可选`STUDIOLO_TOTP_REPLACEMENT_SEED_HEX`环境槽，
候选引用与秘密必须同时存在或同时缺失。每个值严格为40位小写ASCII十六进制；共用Auth
纯解码入口，服务器仅持有零化的自有副本。读取前确认真实/有效UID和GID一致、core软硬限额为0。
不在argv、配置、仓库、日志或HTTP中放种子；不宣称能擦除操作系统原始环境或代理的副本。

启动顺序：参数及配置/目录隔离 → Unix路径准备 → 进程保护和固定秘密解码 → 既有双库及
就绪验证 → 固定槽装配 → 公开根及可选canonical身份 → 真实Access取钥和初扫 → 全部复核
→ 最后绑定。因子模式要求已登记的Auth v4；没有初始化、迁移、秘密生成或TCP失败降级。

## 连接证明与来源

- socket路径是规范绝对UTF-8路径，最多103字节；所有祖先无符号链接。
- 直接父目录预先存在、当前UID且0700，与配置父目录、Auth、Audit、Vault和assets互不嵌套。
  比较目录真实身份以拒绝别名，准备阶段不创建或清除任何节点。
- 监听前再次检查路径不存在与祖先身份。绑定后核类型、UID、单链接、inode并设0600；
  退出/取消只通过保留的父目录FD清理仍同身份的本次socket。已有路径一律拒绝，不自动删“旧socket”。
- 私有连接凭据仅由真实Unix连接的内核`peer_cred`构造。每次因子请求要求同UID，TCP、
  手填转发头或无真实连接扩展均不能得到证明；这不认证可执行文件名，也不隔离恶意同UID或管理员。
- 每请求严格解析唯一`CF-Connecting-IP`，不能在复用连接中缓存IP；拒绝重复、多值、空白、
  端口、方括号、zone和非ASCII。不回退XFF、X-Real-IP或TCP地址；网络桶归一化继续由Auth负责。

权限变更采用锁定rustix的目录FD操作；Linux版本的chmod接口没有可用NOFOLLOW组合，
依赖受信0700父目录及前后身份检查，不承诺抵御同UID路径竞争或任意系统原子替换。
SIGINT/SIGTERM触发优雅排空；没有超时强杀策略，实际服务退出还须等待在途请求完成。

## 封闭端点

三者均只接受POST、无query，要求真实Access、有效L2、匹配CSRF、严格同源Fetch/Origin及
上述来源证明。沿用32KiB/128头、1KiB实际正文、总2秒/2048帧限制、JSON类型与未知/重复字段拒绝。
两个共享blocking槽随实际任务存活；取消HTTP不退还仍运行的许可，也不回滚已提交动作。

| 路径 | 唯一JSON正文 | 成功响应 |
|---|---|---|
| `/api/v1/auth/step-up` | `{"code":"六位数字"}` | 200：仅`verified_at_ms`、`expires_at_ms` |
| `/api/v1/auth/recovery-codes` | `{}`，另需有效L3 | 200：仅`codes`，十个恢复码的一次性交付 |
| `/api/v1/auth/factor/recover` | `{"recovery_code":"规范恢复码","code":"候选六位数字"}` | 204，清Cookie；所有旧会话失效 |

换码不会通过GET重新取回原码；恢复不发新会话或通用L3，须重新显式登录并用活动因子认证。
HTTP没有种子、数据库、环境或登记能力；输入秘密取得所有权即封装并移动到工作任务。
恢复码响应以借用字段序列化进自有零化字节缓冲，Body保留owner至释放；代理/网络/浏览器
传输缓冲并非由这个保证覆盖。所有响应继续无缓存，错误不反射请求或秘密。

| 情形 | 状态及Cookie语义 |
|---|---|
| 会话预检已过期/撤销 | 401，清Cookie |
| OTP/恢复码错误、缺L3，或执行时发现会话被撤销 | 403，保留Cookie；后续会话查询再确认 |
| 无候选、因子工作pending等冲突 | 409，保留Cookie |
| 持久预算耗尽 | 429，保留Cookie |
| 数据库/审计/工作槽不可用或执行结果不能确认 | 503，保留Cookie |

提交成功但响应丢失仍可能已换码或切因子。客户端不得自动重放POST；应清理输入并核对
当前会话，必要时重新登录。补记审计不交付原秘密，也不自动提权。

## Tunnel接入条件

部署时用同一受信UID运行入口代理，固定公开authority作为`httpHostHeader`，service指向
`unix:/approved/proxy/http.sock`；固定authority由已验证HTTPS Origin派生。
Cloudflare文档明确支持Unix服务及Host覆盖：[服务类型](https://developers.cloudflare.com/tunnel/routing/)、
[Origin参数](https://developers.cloudflare.com/tunnel/advanced/origin-parameters/)。

来源预算依赖边缘提供原始客户端IP。该hostname不经可改来源的Worker子请求，不启用移除IP头
或Pseudo IPv4的Overwrite Headers；这些行为会改变或去掉`CF-Connecting-IP`，须在真实接入时核验。
依据：[Cloudflare请求头规则](https://developers.cloudflare.com/fundamentals/reference/http-headers/)。
当前本地夹具没有改变或验收真实Tunnel配置，也未完成HTTPS浏览器、不同系统UID实操或手机配对。
