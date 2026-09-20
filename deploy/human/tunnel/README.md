# T-092 同 UID Unix 源站的 Tunnel 服务

生产上的Studiolo专用Tunnel现由系统manager管理，服务名仍为studiolo-tunnel.service，User/Group=maxsong、PrivateUsers=no。维护用 `sudo systemctl status studiolo-tunnel.service`；原 `systemctl --user` 同名单元已停止且disabled，保留原文件供检查，禁止把两者同时启动。

## 502根因与修复

2026-09-20T06:44:13Z，Cloudflare已正确下发unix:/run/studiolo-human/http.sock及Host，但旧用户服务cloudflared的真实userns只映射UID1000，继承unprivileged_userns AppArmor profile。内核明确拒绝Unix connect：Failed name lookup - disconnected path；fsuid和socket owner同为1000。单元显示PrivateUsers=no并不能证明实际userns；ProtectHome/ProtectSystem/PrivateTmp在用户manager里的隔离触发了该环境。

只把同一固定二进制/token-file与原hardening迁到系统manager，由manager建立挂载隔离；进程仍UID/GID1000，和human处在初始userns。保留NoNewPrivileges、ProtectSystem=strict、ProtectHome=read-only、PrivateTmp、RestrictSUIDSGID、LockPersonality、地址族/架构限制、256MiB/64Tasks。没有读取/更换token、没有关闭AppArmor、没有放宽socket父0700和socket0600、没有换TCP、没有改Access门禁。

系统单元必须使用固定/home/maxsong绝对路径和WorkingDirectory，不能把用户单元的%h直接照搬。候选文件SHA-256为6055f3743bc1a5a7209aa5b25bbd9a0091fbc6234fc2c950d7a2ff2504a4d6f2。

## 已验证的切换

先在相同hardening的临时系统单元运行无凭据tunnel-preflight.py：root读取初始ns，作为唯一参数传给UID1000探针；真实ns、UID、LSM、Unix连接及匿名401全过。不要让目标UID自己依赖/proc/1/ns/user做基准，生产实测该链接在此身份下返回不可用结果。

切换记录 `/var/lib/studiolo-deployments/t092-tunnel-system-smn57eoj`，旧用户单元文本另存。06:51:09.697585Z停旧PID1851并核实退出，06:51:10.583727Z新系统PID20890已连接就绪；随后enable系统单元、disable用户单元。未改动Auth/Audit/因子密文或人侧进程。

最终验证：新Tunnel/人侧/agent active/running且NRestarts0；新Tunnel与PID1/human同initial userns，实际UID/GID四项均1000、LSM unconfined；4条边缘连接，新connector错误0、内核拒绝0。用户原502标签页刷新后显示Studiolo工作台，随后出现账户入口；仅读取标题/按钮，不查看Cookie/OTP/恢复码。agent公网无凭据initialize仍401。真正因子替换仍须主人完成，502修复不等于L3验收。

需要维护或回退时，先停正在运行的一方并确认PID退出，再启另一方，避免两个connector竞争同一metrics端口或承接同一路由。退回旧用户服务会重新暴露本次Unix故障，不能称为恢复方案；不得借回退放宽权限。
