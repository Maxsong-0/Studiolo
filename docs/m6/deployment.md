# M5/M6 本次发布装配

当前用户指令：合并部署 M5 黑板和本机 Codex/Claude Code 订阅派发。禁止模型 API、额外付费回退以及将个人模型登录目录搬到服务器。当前范围见 [本机派发](local-client-dispatch.md)。

## 发布边界

agent 的七个 MCP 工具随二进制发布；缺少 board socket 时，黑板调用失败关闭。既有四个资料工具及策略工具继续使用原身份库、审计 v2、Claude 已生效策略和只读副本。不得使用无条件执行审计 v1 升级的旧 `upgrade-read-supply.sh`；新发布应保留线上单元所有参数与隔离设置，只替换已验证的 release 路径。

本机 Python 连接程序独立安装在 Mac，使用官方 CLI 自己的订阅登录。代码安装不等于客户端登记、MCP 会话加载或常驻服务启用。黑板授权尚未就绪时，不创建可工作的假配置或开启自动派发。

完整黑板上线还需要以下真实前置，不得用测试数据代替：

1. 正式 canonical Vault 与 Broker 数据目录的首次初始化及 Sync 边界。目前 local-server 启动仅接受已初始化的 canonical writer；不能把 agent 的轮换只读副本作为写者，也不能把 consumer-smoke 当作生产初始化工具。
2. 人侧恢复完成后的真实 L3 策略发布：Claude、Codex 仅对 `项目/Studiolo` 设置 `write.board=true` 和精确 `board_projects`，其余类型只读。已有 Claude v1 不能以 bootstrap 覆盖。
3. 将 BoardPlane 装配进唯一 canonical local-server，agent 以同 UID 私有 Unix socket 转发；完成持久状态、服务重启、项目越权与撤权检查。
4. 登记本机原生会话及每个客户端的 Studiolo 类型身份；保留 CLI 审批，确认原生额外 credits 关闭，再启用本机连接。

人侧 Auth/因子恢复与异机灾备由 T-092/T-093 持有。本发布不得重建其认证库、替换因子、恢复已隔离的旧单元，或把新二进制发布宣称为人侧 L3/整库灾备验收。前端与生产 local-server 可先生成不可变候选包，实际人侧切换需对齐恢复状态。

## 验证顺序

合并最新主线后执行 Linux fmt、严格 Clippy、workspace 测试、路径安全、生产特征构建，以及全部前端和 Python 部署契约。推送单个 PR，CI 通过后合入 main。运行包绑定提交及逐文件摘要，在原生 Linux 构建；生产凭据仅留生产主机。

agent 升级先在线只读预检，再短暂停写生成当前身份/审计/策略一致恢复点并恢复旧服务。在隔离副本上运行新 binary 的真实 systemd 阶段实例，以 MCP、HTTP、策略和新鲜度探针验收。切换只替换已绑定的 agent 单元 release 路径；失败恢复原单元，保留新产生审计。切换后独立检查进程 executable、隔离属性、持久索引重启、公网正负向和新鲜 ready。阶段实例及包含凭据的临时副本完成后清理，私有恢复点保留。
