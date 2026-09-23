/**
 * 会话与二次认证：`/api/v1/bootstrap`、`/api/v1/session(s)`、`/api/v1/auth/*`。
 *
 * 假数据模式按「会话模式」运行（authentication = session），这样账户入口、写操作、
 * 需要 One-time Code 的发布/落地/解冻都能走通。形状以 `sessionModel.ts`、`factorModel.ts` 为准。
 */
import { HOUR, MINUTE } from './clock.ts';
import {
  CSRF_TOKEN,
  OTHER_SESSION_ID,
  OWNER_SCOPE,
  RECOVERY_BATCH,
  SESSION_ID,
  sha256,
} from './ids.ts';

/** 近期认证窗口：与后端一致，最长 5 分钟。 */
export const STEP_UP_WINDOW_MS = 5 * MINUTE;
/** 会话绝对寿命 12 小时。 */
const SESSION_LIFETIME_MS = 12 * HOUR;

export interface SessionRecord {
  id: string;
  created_at_ms: number;
  last_seen_at_ms: number;
}

export function sessionInfo(record: SessionRecord) {
  return {
    id: record.id,
    state: 'active',
    created_at_ms: record.created_at_ms,
    last_seen_at_ms: record.last_seen_at_ms,
    absolute_expires_at_ms: record.created_at_ms + SESSION_LIFETIME_MS,
  };
}

export function sessionPair(record: SessionRecord) {
  return {
    session: sessionInfo(record),
    csrf_token: CSRF_TOKEN,
    owner_scope: OWNER_SCOPE,
  };
}

/** 启动时已登录：本机这一个会话，外加另一台设备上的一个。 */
export function initialSessions(now: number): SessionRecord[] {
  return [
    { id: SESSION_ID, created_at_ms: now - 3 * HOUR, last_seen_at_ms: now },
    {
      id: OTHER_SESSION_ID,
      created_at_ms: now - 9 * HOUR,
      last_seen_at_ms: now - 2 * HOUR,
    },
  ];
}

export function factorState(
  now: number,
  recent: { verified_at_ms: number; expires_at_ms: number } | null,
) {
  return {
    factors: {
      observed_at_ms: now,
      recent_authentication:
        recent && recent.expires_at_ms > now && recent.verified_at_ms <= now
          ? recent
          : null,
      recovery_candidate_available: true,
      pending: false,
    },
  };
}

/** 十个恢复码：同一批次号、序号 00–09、各不相同的尾段。 */
export function recoveryCodes(now: number) {
  return {
    codes: Array.from(
      { length: 10 },
      (_, index) =>
        `r1.${RECOVERY_BATCH}.0${index}.${sha256(`recovery ${now} ${index}`).slice(0, 32)}`,
    ),
  };
}

/** 验证码 000000 模拟「验证未通过」，其余六位数字一律通过。 */
export const REJECTED_CODE = '000000';
