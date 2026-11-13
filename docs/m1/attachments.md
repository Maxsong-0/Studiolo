# T-201 人侧附件预览与下载

文件树的附件面板提供 PNG/JPEG/GIF/WebP 预览、UTF-8 小文本预览与文件下载。
继承当前应用会话，不增加接入方附件工具或权限，不允许上传、编辑或执行附件。

`GET /api/attachment?path=<逻辑路径>&generation=<当前代次>` 必须唯一提供两项参数，
拒绝未知参数。路径须可见、规范，并命中该代次文件树的 `Attachment`；目录、Markdown
与未知文件为 404。旧代次或快照大小与读取内容不一致为 409，超限 413，忙时 429。
路径不安全返回 400/403，读取暂不可用返回固定 503，响应不包含宿主路径。

复用 Broker rooted 稳定读取和两个查询工作槽。单文件上限 **32 MiB**，先核清单大小，
再由 Broker 核磁盘大小、类型、链接数、设备和前后元信息。读取前后检查代次。
查询许可持续到下载体销毁；响应以独立 16 KiB 小块发送，慢客户端不会堆积额外整文件缓冲。
**字节属于请求时刻，同大小内容变化允许；不是文件版本下载。**

| 扩展名与内容校验 | 响应类型 | 面板行为 |
|---|---|---|
| png、jpg/jpeg、gif、webp，匹配对应完整签名前缀 | 对应 image 类型 | 图片适宽，可切原始尺寸 |
| txt/csv/tsv/json/canvas/yaml/yml/toml/log，UTF-8 且无 NUL | text/plain; charset=utf-8 | ≤1 MiB 纯文本节点；更大只下载 |
| pdf，匹配 `%PDF-` | application/pdf | 下载，交系统阅读器 |
| 其余扩展名，或任何内容校验失败 | application/octet-stream | 仅下载；HTML/SVG 永不预览 |

全部成功响应包括精确 `Content-Length`、`Content-Disposition: attachment`（ASCII
安全回退名与 RFC 8187 `filename*`）、`Accept-Ranges: none`。Range 请求忽略并回
200 完整内容，HEAD 同读同限但不发正文。

下载响应 CSP 固定为 `default-src 'none'; sandbox; frame-ancestors 'none'`，继承
no-store/no-transform、nosniff、no-referrer、CORP same-origin；会话模式保留 Vary 和全部来源、
Fetch Metadata、应用会话门禁。直接导航、img 或普通链接请求 API 在会话模式被拒绝，
前端必须经过认证 fetch。页面只把 `img-src` 增加 `blob:`，`object-src 'none'` 不变。

前端二进制读取与 JSON 共用两槽认证队列和 SessionClient 生命周期，401 清除工作台。
声明长度、实际流长度与快照大小逐一比较。未知 MIME 降为惰性下载；文本按文本节点插入。
图片和小文本自动读取；PDF、其他文件和大文本先点击「准备下载」，再点击真实下载链接。
关闭取消在途读取，卸载/换代次撤销 Blob URL。人侧读取与正文一致，不记接入方读取审计。

## 验证边界

2026-11-13 的 108 契约已覆盖 MIME 降级、路径与链接、大小变化、32 MiB 上限、
安全头、Range/HEAD、认证门禁、取消、长度异常与会话失效。浏览器人工检查确认
PNG 解码、CSV 中 script 字样仅为文本、HTML 只有下载入口、390×844 无横向溢出与焦点归还。
人工检查使用开发夹具。108 真实 Rust＋生产前端产物冒烟 84 项通过；
随后针对多块下载、空附件、HEAD/Range 和 no-transform 的真实 HTTP 增量验证也通过。

用户选择的 PDF 新标签入口以浏览器探针通过为门槛；本轮验收工具禁止进入 Blob
导航，**并未证明产品无法显示 PDF**。按既定回退保留下载，不启用未经验证的新标签入口。
iPhone Safari 真机保存文件与 PDF 阅读仍未验收。

Blob 生命周期参考 [MDN](https://developer.mozilla.org/en-US/docs/Web/URI/Reference/Schemes/blob)，
本地 scheme 继承安全策略见 [CSP 规范](https://www.w3.org/TR/CSP/)。

Cloudflare 自动压缩可能移除 Content-Length，因此附件成功响应增加 `no-transform`，
保留精确长度契约；这是 T-150 原设计的部署兼容性补正。依据
[Cloudflare 压缩文档](https://developers.cloudflare.com/speed/optimization/content/compression/)。
