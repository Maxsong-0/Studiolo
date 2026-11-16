/** 策略平面里草案的人话：谁交的、何时交的。纯函数，只读接口给的字段。策略页、规则页共用。 */
import type { DesiredRecord } from '../policyModel';
import { STATUS_LABEL } from '../policyModel';

/** 接口时间戳按本地时间显示；解析不了就原样显示，不猜。 */
export function displayTime(value: string): string {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
}

/** 谁交的：你自己，或哪一类 AI 的哪个实例。 */
export function draftSubmitter(record: DesiredRecord): string {
  return record.submitter_kind === 'agent'
    ? `由 ${record.agent_type ?? '未知类型'} 实例 ${record.agent_instance ?? '未知'} 提交`
    : '你提交';
}

/** 一份草案的来历：状态、谁提交、何时提交。 */
export function draftOrigin(record: DesiredRecord): string {
  return `${STATUS_LABEL[record.status]}，${draftSubmitter(record)}，${displayTime(record.submitted_at)}`;
}
