# 人侧代码候选落地

本模块是受信宿主的代码落地点。HTTP 必须先验证当前人类会话、CSRF 和 L3；Runner 及 AI 黑板写入口不能调用它。请求只有 task_id、expected_head、patch_sha256，不接收宿主路径、仓库路径、patch 正文、模型参数或凭据。

`RunnerPlane::land_candidate(task_id, expected_head: String, patch_sha256: String) -> Result<CodeLandingResult, String>` 只接受 Claude/Codex 已封存的 NeedsReview 候选。它从 Control DB 的最后 attempt、artifact receipt 和 Broker 实际内容验证 candidate.patch / patch.json 的路径、revision、SHA-256，并把 metadata.retained_path 限定为配置 retained_root 下由 adapter/attempt/fence 派生的唯一文件。留存文件字节必须等于 Broker 候选。

全局模型冻结时仍可进行人侧落地；原任务执行身份的类型、实例及全部祖先必须仍有效，当前策略也必须保留该完整项目的 board write 权限。重新配置前不接受新仓库映射。操作不调用模型。

落地和新 Runner 快照使用同一把宿主仓库锁：state_root 中的锁文件名绑定 repository 目录的 dev/ino，0600、普通单链接文件、O_NOFOLLOW、独占非阻塞 flock。仓库必须属于服务 UID 且不可由组/其他用户写入。全程持锁，已进入启动或落地操作互斥；其他任务的已停止候选不占 v1 Worker 槽位。宿主人工编辑工具若不遵守该锁，前后完整树核验仍会拒绝不匹配结果，但不会自动覆盖或回滚人工改动。

`CodeLandingPlan` 同时绑定 expected_head、expected_tree、expected_after_tree 与补丁摘要。应用前使用 0700 临时目录中的 GIT_INDEX_FILE 将完整工作树（含 dirty/untracked 非忽略内容）生成 tree，与候选 base_tree 做 CAS；HEAD 必须相同。Git 环境清空、全局配置/系统配置和 hooks/fsmonitor/external diff 被禁用，存在本地 include/filter 配置则拒绝。

先运行 git apply --check，适用则 git apply，否则尝试 --3way。三方合并使用私有临时 index，不覆盖用户的真实暂存区。成功必须没有冲突标记、HEAD 仍匹配，完整工作树 tree 必须等于候选 after_tree。空 patch 只在基线和目标树完全相同且 CAS 通过时作为明确 no-op 落地。任何路径都不自动 commit、push 或改变 ref；单独保留的 advance_code_ref API 不在本流程调用。

外部副作用前先写权威 audit intent，再 create_new + fsync 私有 intent 记录。真实操作后持久记录 outcome，再写权威 audit outcome。记录键绑定 attempt/fence；记录体额外绑定仓库身份、task、HEAD 和 patch SHA，替换仓库不会隐藏旧 intent。所有 provider 的共享 baseline 缓存在应用前失效。成功重放不重新应用补丁，而是重新核对记录与实际 HEAD/tree。已有 intent 无 outcome 或先前失败时禁止自动重放，保留证据供人工处理。

安全响应 `CodeLandingResult` 只含：

- task_id、attempt_id、patch_sha256、expected_head：精确操作绑定。
- after_tree：候选目标 Git tree OID，40 或 64 位；失败时也只是目标树。
- applied：已通过完整结果树与冲突检查。
- replayed：此前成功落地且本次重新核验通过；不会与 applied=false 同时返回。
- conflicts：仓库相对冲突路径，不含宿主绝对路径。

失败始终保留原候选与本地补丁。成功仍保持 NeedsReview，不生成通过的 acceptance，也不自动完成任务。

代码任务的人侧 Completed 验收必须先调用 `require_landed_candidate(task_id) -> Result<CodeLandingReviewGuard<'_>, String>`，保存返回 guard，再获取 engine 锁并调用 review_board_candidates；整个验收期间保持 guard 存活，不跨 await。它重新验证当前权限、Broker 候选、成功落地记录及当前 HEAD/tree，并持仓库锁防止受控落地在检查与验收间改动仓库。失败/停止/放弃回执及 Consumer 回执使用各自既有规则。

本轮写入四个临时 Git/文件锁契约测试：dirty/untracked CAS 与原 index/ref 保持、新增未跟踪内容或 stale HEAD 拒绝、仓库锁互斥/intent 不可覆盖、no-op 必须匹配完整基线。子任务未运行构建或测试；最终 Linux 验证由根任务统一执行。真实 CLI/模型及生产部署验收仍独立未验。
