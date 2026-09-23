/*
 * 黑板状态的中文说法与状态方块（任务与回执、协作拓扑、任务详情共用）。
 * 文案原样沿用黑板视图；方块语法见设计规范第 1 节：■ 运行/正常、□ 进行中/待定、红 ■ 故障/等你、灰虚线 休眠。
 */
import type { BoardStatus, Receipt } from '../boardModel';
import type { StatusState } from '../ui';

export const statusLabels: Record<BoardStatus, string> = {
  draft: '草稿，尚未排队',
  queued: '待认领',
  running: '进行中',
  claimed: '已认领',
  committing: '写回中',
  cancel_requested: '等待停止回执',
  succeeded: '已完成',
  failed: '失败',
  cancelled: '已停止',
  abandoned: '已放弃',
  needs_review: '待审阅',
  needs_revision: '等待修订',
  needs_reconfirm: '等待重新确认',
  approved: '已审阅通过',
};

export const receiptLabels: Record<Receipt['status'], string> = {
  completed: '已完成',
  failed: '失败',
  stopped: '已停止',
  abandoned: '已放弃',
};

export const assessmentLabels: Record<
  Receipt['acceptance'][number]['result'],
  string
> = { pass: '通过', fail: '未通过', unverified: '未验证' };

/** 已结束的四个状态；`approved` 与各 `needs_*` 仍算未结束（与汇总接口同一口径）。 */
const finished: readonly BoardStatus[] = [
  'succeeded',
  'failed',
  'cancelled',
  'abandoned',
];
/** 等主人动手的三个状态：待审阅、等待修订、等待重新确认。 */
const waiting: readonly BoardStatus[] = [
  'needs_review',
  'needs_revision',
  'needs_reconfirm',
];
/** 有活跃认领、按理应当持续回报进度的三个状态（休眠只在这里判）。 */
const claimed: readonly BoardStatus[] = ['claimed', 'running', 'committing'];

export const isFinished = (status: BoardStatus) => finished.includes(status);
export const isWaiting = (status: BoardStatus) => waiting.includes(status);
export const isClaimedWork = (status: BoardStatus) => claimed.includes(status);

/** 任务状态对应的方块；租约到期、休眠这类由调用方先判，再落到这里。 */
export function statusMark(status: BoardStatus): StatusState {
  switch (status) {
    case 'claimed':
    case 'running':
    case 'committing':
      return 'active';
    case 'approved':
    case 'succeeded':
      return 'ok';
    case 'needs_review':
    case 'needs_revision':
    case 'needs_reconfirm':
    case 'failed':
      return 'alert';
    case 'cancelled':
    case 'abandoned':
      return 'dormant';
    case 'draft':
    case 'queued':
    case 'cancel_requested':
      return 'pending';
  }
}
