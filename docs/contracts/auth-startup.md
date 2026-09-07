# 显式会话启动契约（T-019）

2026-09-05。实现与测试边界见[M2记录](../m2/README.md)；不等于完整M2或线上部署。

## 唯一配置与明确模式

```text
studiolo-local-server --vault /approved/replica --assets /approved/web \
  --port 4322 --session-config /approved/private/owner.json \
  --public-origin https://notes.example.invalid
```

这是虚构路径与域名示意，不可直接当作生产命令。新模式要求全部根为规范绝对路径，
原来的相对`web/dist`默认值不能用于此模式；必须显式给出合格assets目录。
`--session-config`必须配`--public-origin`，与旧`--access-team/audience/email`互斥。
此TCP模式无自动搜索配置、环境覆盖、种子读取、隐式登记/迁移或启动失败降级。
不指定新开关时，旧本机与完整四参数Access只读模式保持原行为。

复用[登记工具](auth-admin.md)的JSON v1 `owner.json`。共享`studiolo-host-config`
Module只公开文件加载与只读getter、Session/Access派生、目录快照；不公开构造或反序列化入口。
同一owner的issuer/audience/email产生Access配置，subject同时纳入Auth精确主人绑定。
不能由请求、环境或独立旧参数覆盖。TTL仍须与AuthDB一致。
`seed_ref`只是既有非秘密元数据；此TCP模式不使用它取得种子或登记句柄。
T-024的[显式Unix因子模式](auth-factor-http.md)另从固定秘密环境装配运行因子，沿用本契约配置与就绪检查。

## 私有文件与公开根

- 配置最多16KiB，严格JSON对象，拒绝未知/重复/缺失字段；0600单硬链接普通文件。
- 配置父目录及Auth/Audit根均预先存在、当前UID且0700；拒绝setuid/setgid、符号链接祖先。
- 配置通过NOFOLLOW/NONBLOCK/CLOEXEC有界读取，比较打开前后dev/inode、长度和纳秒时间戳。
- Auth/Audit/Vault/assets四根两两不能相同或互为祖先，同时比较真实目录身份，拒绝大小写别名。
- 配置文件不能落在任何公开根内；允许配置父目录中的兄弟目录作公开根，但仍须通过四根隔离。
- 保存配置文件与四根身份；打开后的ReadOnlyVault根句柄必须与保存的身份相同。
  配置/目录在取钥后与最终监听前再次复核；SQLite侧文件改变目录mtime不被误判成换根。

沿既有存储契约依赖可信稳定父目录和同UID管理员不恶意更名；这些检查不是管理员隔离，
不承诺跨任意路径操作的系统原子性或阻止管理员一致克隆。Broker根句柄身份仅用于本机装配，
不是Vault UUID/revision，不创建持久标记，不向HTTP暴露。

## 固定启动顺序

参数校验 → 配置/根隔离 → Audit/Auth `open_existing` → Auth就绪检查 → 打开并比较只读根句柄
→ 固定真实Access公钥初始化 → 只读路由完整初扫 → 配置/根复核 → 最后绑定IPv4 loopback。

所有初始化错误均非零退出，固定脱敏说明，不打印路径、主人字段或底层存储/网络日志。
不触碰另一启动模式，不留下自建主库或后台任务。`open_existing`不是零写：合法打开仍可能
推进Auth时间地板、过期状态或产生SQLite侧文件；进程非零退出不代表所有存储字节均未变化。
只读Vault与assets仍不创建缓存、控制文件或数据库。

## Auth就绪检查

`SessionManager::require_http_ready()`无种子、身份或秘密参数，无授权返回值。
支持已确认因子的v3/v4；v2返回MigrationRequired，未登记或仅首次登记pending返回FactorUnavailable。
在Auth原事务中取当前确认Proof快照，锁外通过原AuditLedger查询精确的intent/outcome双ACK，
再回Auth事务重验同一Proof并观察时间。登记或恢复所产生的当前因子证明均走同一检查。

失败或非精确回执返回AuditUnavailable；最终证明改变则拒绝。不append原事件来“修复”错误账本，
不维护回收、不签发后丢弃会话、不消费OTP，不把检查结果计成登录拒绝。
这里证明当前因子锚点属于所配置的审计账本，不穷举全部历史ACK，不认证一致克隆。
结果只在启动时有效；每次HTTP仍真验Access、原会话和已登记签发门槛，不能缓存启动结果授权。

## 验证与未交付

文件/SQLite使用真实私有临时对象；失败路径与旧本机TCP运行真实binary。
新模式成功组合仅在私有测试Seam替换取钥Adapter，仍使用真实配置、双库、签名JWT和路由；
生产入口只能调用AccessVerifier::connect，无可配置JWKS URL、生产身份或公钥工厂。
生产与test-support特征混用须以既有指定编译错误拒绝。

T-021已完成会话引导，T-024继续因子HTTP；验证范围见M2记录。因子界面、真实主人登记、
HTTPS浏览器Cookie、服务器发布/回滚以及完整M2剩余项仍需独立验收，不能直接切换线上入口。
