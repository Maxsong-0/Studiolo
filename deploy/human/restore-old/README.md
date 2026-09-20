# T-092 找回被删的旧手机条目

主人已删去旧手机条目，但权威 Auth 仍登记 v1；新 v2 只是候选，不能用于当前二次认证。没有已生成的恢复码。r6 仅把仍登记的旧因子在主人私有 TTY 再交付一次，随后必须走原生恢复；不是重置因子或签发 L3。

## 主人操作

```sh
ssh -t maxsong@192.168.100.169 'sudo python3 -I -B -S /home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r6-restore-old/restore-old-factor.py'
```

只在自己的终端按回车并添加“Studiolo 旧因子（仅用于替换）”，保留已有新条目，不截图、录屏或粘贴终端内容。用旧条目手机码完成网页“二次认证”，再生成并保存恢复码；在“恢复验证器”中填一枚恢复码和新条目手机码。看到成功并退出全部会话后回复“恢复完成”。

若工具失败，只报 T092 阶段代码；不重建凭据、不修改 Auth 或尝试预算。真正替换后由操作者只读核对 v2/pending，再按 ../current-v2/README.md 启用 r5。之后主人重新登录、用新时间步 OTP 获取 L3 并生成最终恢复码；用于替换的旧整批恢复码已失效。

## 边界

- 固定隔离密文 `/var/lib/studiolo-human-quarantine/t092-exposed-enrollment-yaghjdqh/retired-totp.cred`，认证名称 `totp-seed`；不读取聊天中已暴露的密钥，不回写活动凭据槽。
- root、隔离 Python、同一个前台控制终端、core=0/dumpable=0/mlockall、安全祖先/文件权限、只读 Auth v4 状态门禁。初始、首次回车后解密前、解密后显示前均确认 v1 enrolled 且两个 pending 表为空。
- 三次检查不与 Auth 写事务原子互斥；不持写锁、不停服务。如果最后检查后另一个会话完成替换，已有屏幕或手机副本不会恢复旧权限。不得宣称 v2 后旧画面在任何时刻绝不仍可见。
- 解密到进程内私有管道，只给主人临时画面；结束/中断清屏。不能抵御主人截图、录屏或终端日志插件。失败只输出封闭阶段/类别/errno，不输出异常正文、子进程内容或秘密。
- 不生成 OTP、不改 Auth/Audit、不清限流、不签会话/L3、不变更服务。

## 2026-09-20 验证

169 新目录冻结，root 只读，两个脚本摘要与 SHA256SUMS 相符。7 项完整真实 SSH PTY 合成测试通过：成功、隐藏非空重提示、Ctrl+C 清屏、坏密文、未登记、已替换 v2、pending 拒绝；夹具使用全零虚构种子，每例确认密文/配置/DB 不变，自动删除夹具。真实脚本无 TTY 先拒绝，生产两份密文及 Auth 元数据不变；旧活动凭据槽仍为空。human/Tunnel/agent 均 active/running、NRestarts=0。

运行合成测试：将 test-restore-pty.py 上传到 `/home/maxsong/t092-r6-test-restore-pty.py`，再在 Mac 执行 `python3 run-pty-tests.py`。控制器只能运行合成夹具；不得经 agent 的真实 PTY 调用 restore-old-factor.py 主入口。

源码未提交/推送；尚未收到主人替换成功，不把 helper 测试当作真实 L3/因子替换验收。

源码收口仅移除文件末尾多余空行；当前`SHA256SUMS`对应提交源码。上述r6历史发行字节与回执保持原样，不据当前源码摘要重写旧包。
