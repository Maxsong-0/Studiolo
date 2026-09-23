/*
 * 协作 › 执行的纯函数：执行九步的阶段表与判定、时间与摘要的等宽写法（T-130 设计规范第 7 节）。
 * 只读已经取到的数据，不为画进度多发请求；读不到、对不上的一律不猜，返回原因。
 */
import type { BoardItem } from './boardModel';
import { receiptLabels, statusLabels } from './collab/boardLabels';
import { candidatePatch } from './executionModel';
import type { LandingResult, RunSummary } from './executionModel';

/**
 * 执行九步（画布驾驶舱标题栏「执行 7 / 9」）。阶段表由界面定义，接口里没有「第几步」这个字段：
 * 每一段写明「凭什么算走到这一步」所依据的接口字段，以及服务端在这一步写进权威审计的事件
 * （`/api/v1/board/replay` 返回的 `action`）。进度按状态字段判定，不靠翻审计：回放一页只有 100 条，
 * 一次正常执行的模型中继、工具调用与心跳远不止这些。
 *
 * | 段 | 名称       | 走到这一步的依据（接口字段）                                                     | 审计事件 |
 * |----|------------|----------------------------------------------------------------------------------|----------|
 * | 1  | 启动       | 黑板任务 draft / queued / needs_reconfirm / needs_revision：停在这一步，等人启动 | execution.start（主人经执行接口启动） |
 * | 2  | 认领       | 任务建立 attempt（fence 与租约）；status = claimed 时停在这一步                  | task.claim |
 * | 3  | 执行体启动 | `/api/v1/execution` 列出这次 attempt 的 run                                       | runner.start |
 * | 4  | 工作       | run.state = running，progress.phase = working                                     | gateway.model_relay、gateway.tool_call、task.heartbeat |
 * | 5  | 节奏检查   | progress.phase = pacing：工具调用数或已用时长过了节奏阈值                         | 同上 |
 * | 6  | 准备收尾   | progress.phase = wrapup；exhausted 是预算耗尽，执行体随即被取消                   | 同上；耗尽时另有 task.cancel_requested |
 * | 7  | 提交候选   | run.state = candidate_retained：候选已留存在隔离区，封存待恢复                     | task.runner_candidate_retained → task.runner_candidates |
 * | 8  | 落地与审阅 | run.state = candidate_review，或黑板 needs_review 且带 candidate_receipt          | execution.land → runner.code_land（只有代码候选）；execution.review |
 * | 9  | 结案       | 黑板回执写入（board.receipt 非空），任务进入终态                                   | task.candidates_reviewed |
 *
 * 「当前」是正在进行或停下来的那一段，它之前的都算已经过；九段全黑只在回执写入之后。
 * 预算没到节奏或收尾阈值就交了候选的执行，第 5、6 段同样算经过（它越过了这两段）。
 */
export const EXECUTION_STEPS = [
  '启动',
  '认领',
  '执行体启动',
  '工作',
  '节奏检查',
  '准备收尾',
  '提交候选',
  '落地与审阅',
  '结案',
] as const;

export type ExecutionStep =
  | {
      known: true;
      /** 当前所在的一段（1–9）；走完时为 9。 */
      step: number;
      name: string;
      /** 回执已写入：九段全部走完，没有「当前」段。 */
      finished: boolean;
      /** 停在这一段出了问题（失败、预算耗尽、候选没封存上）。 */
      halted: boolean;
      /** 这一段的补充说明；没有就是空串。 */
      note: string;
    }
  | { known: false; reason: string };

function at(
  step: number,
  note = '',
  halted = false,
  finished = false,
): ExecutionStep {
  return {
    known: true,
    step,
    name: EXECUTION_STEPS[step - 1],
    finished,
    halted,
    note,
  };
}

function unknown(reason: string): ExecutionStep {
  return { known: false, reason };
}

const phaseSteps: Record<RunSummary['progress']['phase'], number> = {
  working: 4,
  pacing: 5,
  wrapup: 6,
  exhausted: 6,
};

/** 只凭执行服务里的一条 run 判定（执行体进度列表用）。 */
export function runStep(run: RunSummary): ExecutionStep {
  const phase = phaseSteps[run.progress.phase];
  switch (run.state) {
    case 'running':
      return run.progress.phase === 'exhausted'
        ? at(phase, '预算耗尽：执行体不再获准调用工具，随即被取消。', true)
        : at(phase);
    case 'candidate_retained':
      return at(7, '候选已留存在隔离区、封存待恢复；这不算验收。', true);
    case 'candidate_review':
      return at(8, '候选已封存，等你落地（代码候选）并逐项审阅。');
    case 'failed':
      return at(phase, '执行体失败，停在这一段。', true);
    case 'stopped':
      // 在工作中被取消、与交了停止候选后停下，run 上看是同一个 stopped：停在哪一段只有任务知道。
      return unknown(
        '执行体已停止；接口没有给出它停在哪一段（可能在工作中被取消，也可能已提交停止候选）。',
      );
  }
}

/** 待审那一段的说明：代码候选先落地再审阅；本页收到过 applied 的落地回执就只剩审阅。 */
function reviewStep(task: BoardItem, landing: LandingResult | null) {
  const patch = candidatePatch(task);
  if (!patch) return at(8, '候选已封存，等你逐项审阅。');
  const landed =
    landing !== null &&
    landing.applied &&
    landing.task_id === task.id &&
    landing.attempt_id === task.attempt?.id &&
    landing.patch_sha256 === patch.sha256;
  return at(
    8,
    landed
      ? '代码已落地，等你逐项审阅。'
      : '代码候选：先核对 HEAD 并落地，再逐项审阅。',
  );
}

/**
 * 选中任务的九步：黑板条目为主，执行服务里同一 attempt 的 run 补上第 3–8 段。
 * `runsKnown` 为假表示执行服务状态此刻不可确认（出错、过期或还在读）。
 */
export function taskStep(
  task: BoardItem,
  run: RunSummary | undefined,
  landing: LandingResult | null,
  runsKnown: boolean,
): ExecutionStep {
  if (task.receipt)
    return at(9, `回执：${receiptLabels[task.receipt.status]}。`, false, true);
  const own = run !== undefined && run.task_id === task.id ? run : undefined;
  const matched =
    own !== undefined &&
    task.attempt !== null &&
    own.attempt_id === task.attempt.id
      ? own
      : undefined;
  const sealed = task.status === 'needs_review' && task.candidateReceipt;
  // 执行服务说候选只是留存、还没封存上时，审阅按钮按任务锁着（ExecutionTasks 同一规则）：
  // 停在第 7 段，不报「等你审阅」。
  if (own?.state === 'candidate_retained') return runStep(own);
  if (matched?.state === 'candidate_review') return reviewStep(task, landing);
  if (sealed) return reviewStep(task, landing);
  if (matched) return runStep(matched);
  switch (task.status) {
    case 'draft':
    case 'queued':
    case 'needs_reconfirm':
    case 'needs_revision':
      return at(1, `${statusLabels[task.status]}。`);
    case 'claimed':
      return at(2, '已认领，执行体准备中。');
    case 'running':
    case 'committing':
    case 'cancel_requested':
      return unknown(
        runsKnown
          ? '执行服务里没有这次 attempt 的执行记录（可能由会话里的 agent 直接认领），停在哪一段不可确认。'
          : '执行服务状态不可确认，停在哪一段不可确认。',
      );
    case 'needs_review':
      return unknown('待审阅，但没有封存的候选批次；以任务与回执为准。');
    case 'approved':
      return unknown('已审阅通过，但回执尚未写入；以任务与回执为准。');
    case 'succeeded':
    case 'failed':
    case 'cancelled':
    case 'abandoned':
      return unknown('任务已结束但没有回执，停在哪一段不可确认。');
  }
}

const pad = (value: number) => String(value).padStart(2, '0');

/**
 * 等宽时间：本机时区 `YYYY-MM-DD HH:MM:SS`，另给 ISO 串放进 `<time dateTime>`。
 * 接口只保证是有限数的毫秒值可能超出 Date 的范围：那种一律返回 null，由调用方显示「—」。
 */
export function stamp(
  value: number | string,
): { text: string; iso: string } | null {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return null;
  const text = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(
    date.getDate(),
  )} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
  return { text, iso: date.toISOString() };
}

/** 摘要只露前几位（完整值由调用方放进 title）；本来就短的原样返回。 */
export function shortHash(value: string, length = 8): string {
  return value.length > length ? value.slice(0, length) : value;
}
