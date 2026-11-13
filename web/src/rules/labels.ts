/** 规则视图的人话：列表里草案一栏的写法。纯函数，只读接口给的字段。时间与来历的写法在 policy/labels.ts。 */
import type { DesiredRecord } from '../policyModel';

export { displayTime } from '../policy/labels';

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
