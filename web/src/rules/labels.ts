/** 规则视图的人话：草案状态、提交者与时间的写法。纯函数，只读接口给的字段。 */
import type { DesiredRecord } from '../policyModel';
import { STATUS_LABEL } from '../policyModel';

/** 列表里「未发布草案」一栏：校验未通过与 AI 提案是等你处理的（红），主人自己的草案只是待定。 */
export function draftSummary(
  drafts: readonly DesiredRecord[],
): { state: 'alert' | 'pending'; text: string } | null {
  if (!drafts.length) return null;
  if (drafts.some((record) => record.status === 'rejected'))
    return { state: 'alert', text: '草案校验未通过' };
  if (drafts.some((record) => record.submitter_kind === 'agent'))
    return { state: 'alert', text: 'AI 提案待复核' };
  return { state: 'pending', text: `未发布 ${drafts.length} 份` };
}

/** 接口时间戳按本地时间显示；解析不了就原样显示，不猜。 */
export function displayTime(value: string): string {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
}

/** 一份草案的来历：状态、谁提交、何时提交。 */
export function draftOrigin(record: DesiredRecord): string {
  const who =
    record.submitter_kind === 'agent'
      ? `由 ${record.agent_type ?? '未知类型'} 实例 ${record.agent_instance ?? '未知'} 提交`
      : '你提交';
  return `${STATUS_LABEL[record.status]}，${who}，${displayTime(record.submitted_at)}`;
}
