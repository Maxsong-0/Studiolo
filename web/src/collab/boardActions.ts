/**
 * 黑板审计动作的词表（驾驶舱与协作页的「今天的时间线」、驾驶舱最近动态共用）：
 * 哪些算执行体在干活、哪些是你的决定、哪些是高频遥测，以及动作名的中文说法。只收后端真实存在的动作名
 * （task-engine、control-db、local-server 与 gateway 里 append 的审计 action）。
 */

/** 执行体自己的活动：认领、执行体启动与中继、模型与工具调用、心跳、提交或留存候选、交接往返。 */
export const WORK_ACTIONS: ReadonlySet<string> = new Set([
  'task.claim',
  'handoff.accepted',
  'runner.start',
  'runner.relay',
  'gateway.model_relay',
  'gateway.tool_call',
  'task.heartbeat',
  'task.runner_candidates',
  'task.runner_candidate_retained',
  'task.stopped_candidates',
  'task.stopped_candidates_committed',
  'handoff.returned',
]);

/** 主人经执行接口做的决定（启动、取消、审阅、落地）都记成 `execution.*`。 */
const OWNER_PREFIX = 'execution.';

/** 高频遥测与请求记录：不进「最近动态」（模型、工具调用与心跳仍算工作段）。 */
export const TELEMETRY: ReadonlySet<string> = new Set([
  'gateway.model_relay',
  'gateway.tool_call',
  'task.heartbeat',
  'board.request',
]);

/** 审计动作名的中文说法；没收录的原样显示。 */
const ACTION_TEXT: Readonly<Record<string, string>> = {
  'task.create': '创建',
  'task.claim': '认领',
  'task.heartbeat': '心跳',
  'task.reconcile': '对账',
  'task.cancel_requested': '请求取消',
  'task.abandoned': '放弃',
  'task.launch_failed': '启动失败',
  'task.runner_candidates': '提交候选',
  'task.runner_candidate_retained': '留存候选',
  'task.stopped_candidates': '停止并留存候选',
  'task.stopped_candidates_committed': '停止候选已入库',
  'task.candidates_reviewed': '候选已审阅',
  'task.candidate_recovery_pending': '候选待恢复',
  'task.board_projected': '投影到任务页',
  'task.board_out_of_band_preserved': '保留带外改动',
  'task.policy_pinned': '钉定策略版本',
  'task.number_reserved': '预留编号',
  'runner.start': '执行体启动',
  'runner.relay': '执行体中继',
  'runner.code_land': '代码落地',
  'runner.snapshot_cleanup': '清理快照',
  'gateway.model_relay': '模型中继',
  'gateway.tool_call': '工具调用',
  'handoff.created': '创建交接',
  'handoff.accepted': '接受交接',
  'handoff.returned': '交回回执',
  'board.request': '黑板请求',
  'execution.start': '启动执行',
  'execution.cancel': '取消执行',
  'execution.review': '审阅',
  'execution.land': '落地候选',
};

export function actionText(action: string): string {
  return Object.hasOwn(ACTION_TEXT, action) ? ACTION_TEXT[action] : action;
}

export function isOwnerAction(action: string): boolean {
  return action.startsWith(OWNER_PREFIX);
}
