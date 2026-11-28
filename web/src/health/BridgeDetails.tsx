import { bytes, localTime as time } from './format';
import type { BridgeHealth, ManualQuota, BudgetUsage } from './model';

const EXIT = {
  clean: '干净退出',
  non_zero: '非零退出',
  signal: '被信号终止',
  deadline_exceeded: '超过时限',
  owner_stopped: '随服务停止',
  unconfirmed: '退出结果未确认',
};
const RECOVERY = {
  idle: '无需恢复',
  forced_mirror_remote: '强制只下载恢复一次',
  backoff: '退避等待',
};
const count = (value: string | null) =>
  value === null ? '未知' : BigInt(value).toLocaleString('zh-CN');
const flag = (value: boolean | null, yes: string, no: string) =>
  value === null ? '未知' : value ? yes : no;
/** 已用的百分比，向下取整，与后端同一算法。 */
const percent = (used: bigint, budget: bigint) =>
  budget === 0n ? '比例未知' : `${(used * 100n) / budget}%`;
/** 超上限时同步已停住，直接说；否则给用量和百分比。 */
const paths = (value: BudgetUsage | null) => {
  if (value === null) return '未知';
  const used = BigInt(value.used),
    budget = BigInt(value.budget);
  if (used > budget)
    return `要比对 ${count(value.used)} 条，超过上限 ${count(value.budget)} 条，同步停住`;
  return `${count(value.used)} / ${count(value.budget)} 条（${percent(used, budget)}）`;
};
/** 比对内容（基线、主库、镜像三方各算一遍的字节合计）。超上限时写确切字节数：只超一点时两个数按 GiB 取整后一样。 */
const content = (value: BudgetUsage | null) => {
  if (value === null) return '未知';
  const used = BigInt(value.used),
    budget = BigInt(value.budget);
  if (used > budget)
    return `要比对 ${count(value.used)} 字节，超过上限 ${bytes(value.budget)}，同步停住`;
  return `${bytes(value.used)} / ${bytes(value.budget)}（${percent(used, budget)}）`;
};
/** 明细始终来自同一次健康快照；即使本批熔断，仍可看其他已知证据。 */
export function BridgeDetails({
  value,
}: {
  value: BridgeHealth | null | undefined;
}) {
  if (!value) return <p className="health-detail">尚无可核验的同步明细。</p>;
  const rows = [
    [
      // 后端只在一批闭合时盖序号和时刻，卡住的批次不刷新它（BUG-048）；终态未知、熔断看下面几行。
      '最近闭合',
      value.lastBatch
        ? `第 ${count(value.lastBatch.sequence)} 批 · ${time(value.lastBatch.closedAt)}`
        : '未知',
    ],
    ['路径用量', paths(value.pathUsage)],
    ['比对内容', content(value.byteUsage)],
    [
      '删除被挡',
      value.deletesBlocked === null
        ? '未知'
        : value.deletesBlocked === '0'
          ? '没有'
          : `远端删了 ${count(value.deletesBlocked)} 个文件（改名也算），主库没开文件操作，同步整批暂停`,
    ],
    ['删除保护', flag(value.fused, '熔断生效，后续批次阻断', '未熔断')],
    ['终态未知动作', count(value.unknownJobs)],
    ['静止证据', flag(value.quiescenceValid, '有效', '已过期，等待重新取得')],
    ['冲突副本', count(value.conflictCopies)],
    ['本机镜像', `${bytes(value.mirrorBytes)}（非账号用量）`],
    ['上次退出', value.lastExit === null ? '未知' : EXIT[value.lastExit]],
    ['恢复进度', value.recovery === null ? '未知' : RECOVERY[value.recovery]],
    [
      '排除项检查',
      value.residueGuard === null
        ? '未知'
        : value.residueGuard === 'clear'
          ? '无残留阻断'
          : '有残留，本轮阻断',
    ],
    [
      '同步模式',
      value.mode === null
        ? '未知'
        : value.mode === 'mirror-remote'
          ? '只下载'
          : '双向',
    ],
    [
      '配置核对',
      flag(value.configMatches, '与批准范围一致', '不一致，本轮未同步'),
    ],
  ];
  return (
    <details className="health-bridge-details">
      <summary>查看同步证据</summary>
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
export function ManualQuotaDetails({
  value,
}: {
  value: ManualQuota | null | undefined;
}) {
  if (!value) return null;
  const limit = BigInt(value.accountLimitBytes),
    used = BigInt(value.usedBytesManual);
  const percent =
    limit === 0n
      ? '比例未知'
      : `${(used * 100n) / limit}.${((used * 1000n) / limit) % 10n}%`;
  return (
    <div className="health-quota-detail">
      <p>
        人工记录：{bytes(value.usedBytesManual)} /{' '}
        {bytes(value.accountLimitBytes)}（{percent}；70%预警）
      </p>
      <p>记录于 {time(value.recordedAt)} · 超过24小时需重新核对</p>
      <p>实时账号用量未知；本机镜像大小单独列出。</p>
    </div>
  );
}
