import { getJson } from './sessionHttp';
import {
  bridgeCanBeNormal,
  keys,
  object,
  parseBackupJobs,
  parseBridge,
  parseManualQuota,
  type BackupJobs,
  type BridgeHealth,
  type ManualQuota,
} from './health/model';

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
  bridge?: BridgeHealth | null;
  manualSyncQuota?: ManualQuota | null;
  /** 备份作业明细；和「备份作业」一节对不上时整份丢掉，那一节降为未知。 */
  backup?: BackupJobs | null;
}

/** 「备份作业」一节的机器码，和后端 `BACKUP_JOBS_CODE` 一致。 */
export const BACKUP_JOBS = 'backup-jobs';

/** 未建立恢复点时后端固定给出的正文；前端据此显示灰而不是绿。 */
export const NO_RECOVERY_POINT = '尚未建立灾难恢复点';

function toState(value: unknown): HealthState {
  return value === 'normal' || value === 'degraded' ? value : 'unknown';
}

function toSection(value: unknown): HealthSection | null {
  if (!object(value)) return null;
  const record = value;
  const code = typeof record.code === 'string' ? record.code : '';
  if (!/^[a-z][a-z0-9-]{0,63}$/.test(code)) return null;
  const title =
    typeof record.title === 'string' &&
    record.title.length > 0 &&
    record.title.length <= 128;
  const detail =
    typeof record.detail === 'string' &&
    record.detail.length > 0 &&
    record.detail.length <= 2048;
  const valid =
    title && detail && keys(record, ['code', 'title', 'state', 'detail']);
  return {
    code,
    title: title ? (record.title as string) : code,
    state: valid ? toState(record.state) : 'unknown',
    detail: detail ? (record.detail as string) : '证据缺失。',
  };
}

/**
 * 把后端响应收成封闭模型。
 *
 * 缺字段、类型不对、多余字段一律按"未知"处理：宁可把健康页显示成灰，
 * 也不能因为解析宽松而把故障显示成绿。
 */
export function parseHealth(value: unknown): HealthReport {
  const record = object(value) ? value : {};
  const validTop =
    Object.keys(record).every((key) =>
      [
        'overall',
        'evidence_complete',
        'sections',
        'bridge',
        'manual_sync_quota',
        'backup',
      ].includes(key),
    ) &&
    typeof record.evidence_complete === 'boolean' &&
    ['normal', 'degraded', 'unknown'].includes(record.overall as string);
  const parsed =
    Array.isArray(record.sections) && record.sections.length <= 64
      ? record.sections
          .map(toSection)
          .filter((item): item is HealthSection => item !== null)
      : [];
  const byCode = new Map<string, HealthSection>();
  for (const section of parsed) {
    byCode.set(
      section.code,
      byCode.has(section.code)
        ? {
            ...section,
            state: 'unknown',
            detail: '检查项重复，证据无法核验。',
          }
        : section,
    );
  }
  const sections = [...byCode.values()];
  const unique = sections.length === parsed.length;
  const bridge = parseBridge(record.bridge);
  const quota = parseManualQuota(record.manual_sync_quota);
  const badBridge =
    record.bridge != null &&
    (bridge === null ||
      !sections.some((section) => section.code === 'sync-bridge'));
  const badQuota =
    record.manual_sync_quota != null &&
    (quota === null ||
      !sections.some(
        (section) =>
          section.code === 'manual-sync-quota' && section.state === quota.state,
      ));
  const backup = parseBackupJobs(record.backup);
  const badBackup =
    record.backup != null &&
    (backup === null ||
      !sections.some(
        (section) =>
          section.code === BACKUP_JOBS && section.state === backup.state,
      ));
  for (const section of sections) {
    // 「备份作业」报正常必须有一份对得上的明细撑着，而且明细自己的结论就是正常。
    if (
      section.code === BACKUP_JOBS &&
      (badBackup ||
        (section.state === 'normal' && backup?.verdict !== 'healthy'))
    )
      section.state = 'unknown';
    if (
      section.code === 'sync-bridge' &&
      (badBridge ||
        (bridge && section.state === 'normal' && !bridgeCanBeNormal(bridge)))
    )
      section.state = 'unknown';
    if (section.code === 'manual-sync-quota' && badQuota)
      section.state = 'unknown';
  }
  const shapeValid =
    validTop &&
    unique &&
    Array.isArray(record.sections) &&
    record.sections.length === sections.length &&
    !badBridge &&
    !badQuota &&
    !badBackup;
  let overall = shapeValid ? toState(record.overall) : 'unknown';
  if (sections.some((section) => section.state === 'degraded'))
    overall = 'degraded';
  else if (
    overall === 'normal' &&
    (record.evidence_complete !== true ||
      sections.some((section) => section.state === 'unknown'))
  )
    overall = 'unknown';
  const complete =
    shapeValid &&
    overall !== 'unknown' &&
    record.evidence_complete === true &&
    sections.length > 0 &&
    sections.every((section) => section.state !== 'unknown');
  return {
    overall: sections.length === 0 ? 'unknown' : overall,
    evidenceComplete: complete,
    sections,
    bridge,
    manualSyncQuota: quota,
    backup: badBackup ? null : backup,
  };
}

/** 读取本机只读健康页。走与其他内容 API 相同的会话门禁，没有独立入口。 */
export async function readHealth(signal: AbortSignal): Promise<HealthReport> {
  return parseHealth(await getJson('/api/v1/health', signal));
}
