# T-035 · Apple 界面与动效发布

2026-09-08，用户授权完善、合并并部署已在主线的 T-011 界面。本次沿用其白色内容区、
通透导航、蓝色共享选中态和只读阅读器，修复实际交互边界，不宣称完整 M2 已验收。

## 变化

- 手机文档搜索与项目范围独立成行，输入与选择文字均为 16px；切换主栏目及“查看全部”回到页面起点。
- 系统减少动态偏好即时生效。导航与文档分段只重建装饰节点，保留焦点、输入和展开目录；
  指针高光立即停止，关闭偏好后下一次指针操作可恢复。
- 页面标题适用于本机及远程只读工作台。图标与其他构建资源同源加载，SVG 仅由已有可信
  `/assets/` 路由提供；用户笔记和附件没有新增静态读取入口。
- 发布包携带独立 production server/admin 程序，进程测试明确使用包内绝对路径。
  包内与 CI 均检查部署脚本；HTTP smoke 覆盖 bootstrap、文件树、图标、代次与拒绝写入。

## 发布目标与兼容性

目标为现有 `studiolo-access.service`、`127.0.0.1:4319`、`https://studiolo.maxsong.me`。
使用现有示例库、Access audience/email 配置和服务隔离。无 session、canonical writer 或 factor
启动参数；bootstrap 为 `access`，没有开启应用会话或写入模式。

旧 T-009 binary 缺少 `/api/v1/bootstrap` 和 `/api/tree`，不能只替换网页资源。
必须将当前网页与当前 production server 作为同一个运行包发布。
4318 预览、4400 MCP、Tunnel 和 Access 策略保持既有配置。

## 构建与切换

候选发布名：`m1-t035-20260908-ui01`。本机输出目录为
`/private/tmp/m1-t035-20260908-ui01/payload/`，由 `scripts/package-preview.sh` 从白名单源码快照构建。
包中 `source.tar.gz` 与 `SHA256SUMS` 是实际构建内容的依据，不能仅用 base commit 推断产物一致。

服务器恢复 SSH 后，先核验主机、三个应用服务及 Tunnel 状态，保留当前 Access 单元作为回滚副本。
将候选包放到以下全新目录，拒绝覆盖已有目录：

```text
/home/maxsong/.local/share/studiolo-preview/releases/m1-t035-20260908-ui01/
```

在包根校验 `SHA256SUMS`，将 `source.tar.gz` 解压至该目录的 `source/`，进入 `source/` 执行
`bash ../run-preview-tests.sh`。全部 Linux 原生测试必须通过，包括路径安全、架构与真实进程契约。
不把 macOS 或 amd64 仿真构建当作 Linux 原生验收。

先用包内程序和示例库启动一个空闲回环端口，执行包内 `check-preview.py <回环地址> --example`。
这是无鉴权示例 smoke，不直接用于 4319 Access 端点。随后仅安装包内 `studiolo-access.service`，
执行用户级 daemon-reload 并重启该服务。记录实际 release 路径、单元状态与重启次数。
公网检查应确认 Access 登录、总览、中文搜索、原文、文件树和静态资源可用，且未登录请求仍被门禁拦截。
同时核验 4318、4400 和 Tunnel 仍正常。回滚时恢复先前保存的 Access 单元并仅重启该服务。

## 当前证据与限制

- 前端 28 个测试文件、130 项通过；TypeScript、生产构建、ESLint、Prettier 通过。
- 本机 production server 的 HTTP/Access/tree 定向测试 22 项通过；Python smoke 回归 10 项通过。
- 本机复制后 production admin 的真实 PTY 进程契约通过；受限沙箱内不能创建所需前台终端，
  在已获自动审批的沙箱外重新执行通过。
- 本机完整只读 HTTP smoke 74/74 通过，入口 `http://127.0.0.1:4322/`，仅仓库虚构示例库。
- Chromium 149.0.7827.55（本机已有 Playwright 运行时，未安装 Browser 插件）真实浏览器 35 项检查通过：
  1536×1024、390×844、320×844；筛选、快速切换键盘、原文第 6 行、焦点归还、空目录、附件元数据、
  模态互斥、搜索状态保留、手机回顶、减少动态开关与增强对比。控制台和所有资源响应无错误。
  截图仅保存于 Codex 本机可视化目录，未放入发布源码；未做真实 iPhone/Safari 或公网登录验收。
- 2026-09-08 服务器 `192.168.100.169:22` 持续拒绝 SSH 连接，用户已收到恢复连接的请求。
  本记录不代表服务器原生测试、单元切换或公网登录验收已经完成；部署状态须以后续实际证据更新。
