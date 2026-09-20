# T-093 历史M4恢复点回执（2026-09-20，agent隔离恢复通过）

最新含人侧v2与当前M5/M6 agent的恢复点是T-094的`full-offline-r2.tar.age`，见[最新回执](../../human/offline/ACCEPTANCE.md)。
下文保留05:15 UTC历史快照与19:50 UTC验收边界，不是当前最新完整封装状态。

- `test_pack_offline.py` 在107原生Linux执行：22项通过，5.275秒；不含生产数据。
- 已在169执行原恢复器的来源预检、停写一致恢复点、完整白名单副本冻结、严格离线封装和age加密。
- 作业：`/var/lib/studiolo-dr-export/t093-20260920-_p_ys10v`。加密文件 `m4-offline.tar.age`，31,352,488字节，SHA-256 `6f3065acac422d82cb42127d7737c95de14bc0811fb79c7129b30009ecd49822`。
- 停写：2026-09-20T05:15:42.590628Z 至05:15:43.065411Z，0.474783秒。原agent已恢复active；随后公网六工具、白名单拒绝、策略读取、新鲜ready通过。
- 封装含agent身份/审计/策略/凭据、完整白名单replica、程序、可信恢复/探针工具；本次额外在tools/runtime带齐二进制的动态库普通文件及独立来源摘要。库从固定生产二进制的ldd解析，在恢复新目录需由受控步骤设加载器执行位。
- 解密身份仅在Mac私有目录 `~/.local/share/studiolo-dr/keys/t093-20260920.agekey`；生产只持recipient公钥；NAS/CI/Docker未收到私钥。虚构内容加解密回路通过。
- NAS sudo由主人完成首次密码输入，随后独立新SSH中 `sudo -n -k` 验证uid0；规则 `/etc/sudoers.d/zz-studiolo-t093-maxsong` 为root:root0440、单链接。
- 用户明确授权后，真实密文已保存到Mac专用receipts目录，源/目标SHA-256一致，完整解密到空设备通过，末字节篡改被拒绝。该Mac验证未落盘明文。后续NAS与108验收见下。

## 验收边界

这份恢复点不含正在准备的人侧Auth/因子，不是canonical全Vault备份，不代表PRD整库/RPO/不可变存储已完成。备份原目录、解密后的恢复副本与可信工具分别管理；107禁止生产数据。用户本次明确允许108临时解密做隔离恢复，例外仅用于本次演练。

NAS内核4.4.302+不能承载现有Studiolo严格路径契约。108已获准并用于下述隔离演练。不得使用旧token正向验证恢复，不得把历史审计删除当作身份失效。

## 19:50 UTC实际异机证据

- NAS仅保存密文：`/volume1/studiolo-dr/t093-20260920-_p_ys10v/m4-offline.tar.age`，root私有0700目录、0600普通单链接文件；31,352,488字节及SHA-256与上文一致。19:38:42Z完成文件及目录fsync、原子不覆盖发布，并保存非秘密receipt.json。私钥未传NAS或108。
- 108专用root0700目录 `/var/lib/studiolo-dr-tests/t093-20260920-1933` 从Mac经age→SSH私有流接收；密文摘要先核对，age认证及管线退出均成功。已审阅独立工具验证完整解密包，原恢复器restore-copy成功；ldd无缺失，原生程序启动成功。
- `check-restored-agent.py` 在PrivateNetwork=yes、无宿主监听的临时systemd单元中，用产品type-revoke撤销恢复副本唯一原类型；原token实际initialize返回401。进程core=0、不可转储、mlockall成功；生产未修改。
- 非秘密回执：network_namespace_isolated=true，runtime_started=true，types_revoked=1，original_tokens_http_401=1，new_identity_issued=false，positive_supply_check=false。
- 19:49 UTC再次严格验证原解密包仍完整、原临时单元inactive/MainPID0；19:49:59.275267Z确认无引用本目录的进程、节点root所有且无链接/跨设备后，只删除该次私有目录并确认不存在。这是逻辑删除，不承诺物理介质擦除。
- 尚未完成：新agent身份正向供给、旧会话ID重放、含最终人侧Auth/v2因子的完整恢复。当前生产agent另由M5/M6线更新，本包是明确时间点的历史M4恢复点，不冒充当前全部运行状态。T-094补齐人侧离线恢复撤权与可移植因子后，将另建完整恢复点。
