import { getJson } from './sessionHttp';

/** 健康三态。任何无法识别的值一律降到 unknown，绝不当作 normal。 */
export type HealthState = 'normal' | 'degraded' | 'unknown';

export interface HealthSection {
  code: string;
  title: string;
  state: HealthState;
  detail: string;
}

export interface HealthReport {
  overall: HealthState;
  evidenceComplete: boolean;
  sections: HealthSection[];
}

/** 未建立恢复点时后端固定给出的正文；前端据此显示灰而不是绿。 */
export const NO_RECOVERY_POINT = '尚未建立灾难恢复点';

function toState(value: unknown): HealthState {
  return value === 'normal' || value === 'degraded' ? value : 'unknown';
}

function toSection(value: unknown): HealthSection | null {
  if (!value || typeof value !== 'object') return null;
  const record = value as Record<string, unknown>;
  const code = typeof record.code === 'string' ? record.code : '';
  if (!code) return null;
  return {
    code,
    title: typeof record.title === 'string' ? record.title : code,
    state: toState(record.state),
    detail: typeof record.detail === 'string' ? record.detail : '证据缺失。',
  };
}

/**
 * 把后端响应收成封闭模型。
 *
 * 缺字段、类型不对、多余字段一律按"未知"处理：宁可把健康页显示成灰，
 * 也不能因为解析宽松而把故障显示成绿。
 */
export function parseHealth(value: unknown): HealthReport {
  const record =
    value && typeof value === 'object'
      ? (value as Record<string, unknown>)
      : {};
  const sections = Array.isArray(record.sections)
    ? record.sections
        .map(toSection)
        .filter((item): item is HealthSection => item !== null)
    : [];
  const overall = toState(record.overall);
  const complete =
    record.evidence_complete === true &&
    sections.length > 0 &&
    sections.every((section) => section.state !== 'unknown');
  return {
    overall: sections.length === 0 ? 'unknown' : overall,
    evidenceComplete: complete,
    sections,
  };
}

/** 读取本机只读健康页。走与其他内容 API 相同的会话门禁，没有独立入口。 */
export async function readHealth(signal: AbortSignal): Promise<HealthReport> {
  return parseHealth(await getJson('/api/v1/health', signal));
}
