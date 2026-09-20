# T-092 首次因子暴露后的替换装配

状态：原人侧单元已停止/禁用，原活动密文移到root私有隔离目录；Auth与Audit不清库、不重建。实测原会话0、L3 grant0、pending0。原只读公网入口及agent服务不变。

## 冻结r3

产品仍来自main50b33cf，Rust/web未改。运行包 `/home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r3`，manifest SHA-256 `2da5b3913a1469c426a4b8a6f899b000e3caf22a3a8d24734b3d6a47dfb0b3af`。5项107原生合成测试通过；169验包/systemd语法/非TTY拒绝通过。

`prepare-replacement.py` 只由主人私有前台终端运行，生成并保存新的systemd密文候选，在临时终端画面显示手机配对资料。普通画面只显示状态；不承诺防截图、录屏或终端日志插件。确认用getpass隐藏输入，不读取OTP、不自行调用enroll或改Auth库。

r3 launcher通过systemd载入当前与候选两槽，仅打开已有Auth/Audit，不带policy-data，不开放Policy写路径。新候选凭据ID=`totp-next`，新引用=`studiolo-human-totp-v2`。r3单元只是准备材料，未安装/启动；必须先由主人准备新候选，再按授权进行恢复入口启用。

## 必须遵守的替换顺序

1. 主人保留旧手机条目、添加新候选；不向聊天发种子/验证码/恢复码。
2. 确认候选准备后，安装r3恢复装配；保留原Access单邮箱门禁，未经确认不切公网路由。
3. 主人真实Access登录→建立L2→旧手机OTP取得L3→换发恢复码并自己保管。
4. 等新的OTP时间步，以一枚刚生成的恢复码和新手机OTP调用原生因子恢复。原Auth执行双审计、因子前滚、全部旧会话/L3/整批恢复码失效。
5. 核对权威factor绑定已为v2且pending为空，再配置只以新因子为current的正式运行包，删除配置中的旧槽。不能提前只换current，也不能倒回r2。
6. 主人重新登录，以新手机的新时间步OTP提权，再生成正式保存的恢复码。之后才能开启并验收策略发布/回滚。

两个秘密槽的角色固定。替换成功后原两槽重启会按数据库绑定选择新槽，不把旧槽当恢复候选。`owner.json.seed_ref`只参与秘密装配，不属于持久SessionConfig主人/TTL绑定；收口时改它不等于重建Auth。

依据：`docs/contracts/auth-recovery.md`、`crates/auth/src/session/factor_runtime.rs`。这里不实现无手机/无恢复码的旁路，不把保留旧目录再建新库称作安全恢复。

## r4 续接已经保存的候选

用户首轮未见临时配对画面且收到泛化错误；实测v2候选密文已保存，root:root/0600/单硬链，认证解密到空设备成功、无残留临时文件。凭据父目录fsync和仅虚构值的r3 PTY探针通过，尚未确认首轮错误根因。不得据此推断用户输错或重新生成候选。

续接工具冻结于 `/home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r4/resume-replacement.py`，SHA-256 `fd9ec20b60a0dc2e2f7707077efd24bb7eb175ec7e6fb3161a69250c18c16e5e`；支持脚本与r3相同。r4只是配对续接工具，不是替换服务版本。只读核验原Auth状态，按totp-next认证解密既有候选，明确newline=no及40字节小写hex门禁，保留原TTY/内存保护。终端完整短写处理、UTF-8直接输出、隐藏确认可重试；失败只输出封闭阶段/类别/整数errno，不输出异常正文或秘密。

主人执行：

```sh
ssh -t maxsong@192.168.100.169 'sudo python3 -I -B -S /home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r4/resume-replacement.py'
```

不要再运行不带resume的r3创建命令。保留旧手机条目，添加新候选并保存密钥，只回报“新因子准备好了”或T092错误代码。查看/添加候选不等于完成因子替换。

169实际SSH PTY完整五项合成流程通过：成功、隐藏非空输入重提示、Ctrl+C清屏、坏密文拒绝、未登记拒绝；每项候选/配置/数据库保持原样。夹具在自动清理的root私有 `/var/lib/t092-resume-fixture-*`，仅使用全零虚构秘密。另实际入口非TTY拒绝且生产候选/Auth元数据不变。最初把夹具放在/var/tmp被安全祖先门禁拒绝，改用安全私有目录后全过；未放宽产品门禁。测试入口为本目录 `test-resume-pty.py` 和 `run-pty-tests.py`，仅加载r4程序并覆盖到独立夹具，不调用真实候选配对main。人侧服务仍inactive/disabled，agent仍active，尚未网页替换、切公网或开启策略。
