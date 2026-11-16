import { duration, localTime } from './format';
import type {
  BackupFailureReason,
  BackupJobs,
  BackupUnverifiedReason,
} from './model';

const FAILED: Readonly<Record<BackupFailureReason, string>> = {
  barrier_not_established: '冻结、导出或准入没做成（跨 owner 屏障没有建立）',
  source_changed_before_seal: '封存前来源变了',
  out_of_space: '磁盘空间不够',
  upload_rejected: '上传被拒绝',
  remote_verification_failed: '远端校验没通过',
  final_manifest_commit_failed: '最终清单没提交成',
  target_unverified: '备份目标没接入，验证不了',
  source_expired: '封存正文过了保留期，作业没做完',
};
const UNVERIFIED: Readonly<Record<BackupUnverifiedReason, string>> = {
  no_real_repository: '未验证：还没有真实的备份仓库',
  target_unreachable: '未验证：备份目标连不上',
  immutability_unprovable: '未验证：证明不了目标的不可变保护',
};

function failureText(failure: NonNullable<BackupJobs['lastFailure']>): string {
  if (failure.result === 'failed') return FAILED[failure.reason];
  if (failure.result === 'unverified') return UNVERIFIED[failure.reason];
  return '结果读不出来，不当成功';
}

const judged = (value: boolean | null, yes: string, no: string) =>
  value === null ? '判断不了' : value ? yes : no;

/** 「备份作业」一节的明细；数据和本节结论出自同一次读账本，没有就不显示（本节正文已说明原因）。 */
export function BackupDetails({
  value,
}: {
  value: BackupJobs | null | undefined;
}) {
  if (!value) return null;
  const { lastSuccess, outstanding, lastFailure } = value;
  const rows = [
    [
      '最近成功',
      lastSuccess
        ? `第 ${lastSuccess.sequence} 号 · ${localTime(lastSuccess.at)}`
        : '还没有',
    ],
    [
      '挂着的作业',
      outstanding
        ? `第 ${outstanding.sequence} 号 · ${localTime(outstanding.since)} 开始 · ${
            outstanding.admitted ? '已封存，等 worker 上传' : '还没封存'
          } · ${judged(outstanding.stuck, '已卡住', '没卡住')}`
        : '没有',
    ],
    [
      '逾期',
      `${judged(value.overdue, '已逾期', '没有逾期')}（从 ${localTime(value.overdueSince)} 起算）`,
    ],
    [
      '最近失败',
      lastFailure
        ? `第 ${lastFailure.sequence} 号 · ${localTime(lastFailure.at)} · ${failureText(lastFailure)}${
            value.latestFailed ? '' : '（之后已有成功）'
          }`
        : '没有',
    ],
    ['卡住线', `挂过保留期 ${duration(value.stuckAfterSeconds)}`],
    ['逾期线', `两倍最小间隔 ${duration(value.overdueAfterSeconds)} 没有成功`],
    ['取数时刻', localTime(value.asOf)],
  ];
  return (
    <details className="health-backup-details">
      <summary>查看备份作业</summary>
      <dl>
        {rows.map(([label, detail]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{detail}</dd>
          </div>
        ))}
      </dl>
    </details>
  );
}
