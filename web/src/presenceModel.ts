/**
 * `GET /api/v1/board/presence` 的封闭解码器（T-237，ADR-046；契约 docs/contracts/connector.md）。
 * 每条的必带字段一个不能少、可不带字段只认约好的那几个；任何一处不符整份判无效，
 * 不把读不懂的当成「没有会话」。
 */

export const PRESENCE_INVALID = '本机会话格式无效，状态不可确认。';
export const PRESENCE_MAX_SESSIONS = 256;

const vendors = ['claude_code', 'codex', 'agy'] as const;
const states = ['running', 'waiting', 'idle'] as const;
const actions = [
  'read',
  'edit',
  'run',
  'web',
  'ask',
  'dispatch',
  'other',
] as const;

export type PresenceVendor = (typeof vendors)[number];
export type PresenceState = (typeof states)[number];
export type PresenceAction = (typeof actions)[number];

/** 一条会话状态；没带的是 undefined，界面那一格空着，不写「未知」。 */
export interface PresenceSession {
  /** 哪个连接器推来的（AI 类型名）。 */
  source: string;
  vendor: PresenceVendor;
  project: string;
  state: PresenceState;
  /** 这条状态是多少秒以前收到的。 */
  ageSecs: number;
  branch?: string;
  step?: number;
  total?: number;
  lastAction?: PresenceAction;
  /** 到此刻多久没动（秒）。 */
  idleSecs?: number;
}

export interface Presence {
  /** 状态从几点起攒的：人侧重启后先空着。 */
  since: string;
  expiresInSecs: number;
  sessions: PresenceSession[];
}

const REQUIRED = ['source', 'vendor', 'project', 'state', 'age_secs'];
const OPTIONAL = ['branch', 'step', 'total', 'last_action', 'idle_secs'];

function invalid(): never {
  throw new Error(PRESENCE_INVALID);
}

function record(
  value: unknown,
  required: readonly string[],
  optional: readonly string[],
): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    return invalid();
  const row = value as Record<string, unknown>;
  if (
    required.some((key) => !Object.hasOwn(row, key)) ||
    Object.keys(row).some(
      (key) => !required.includes(key) && !optional.includes(key),
    )
  )
    return invalid();
  return row;
}

/** 非空、有长度上限、不含控制字符与双向／零宽字符（分支名是别处来的文字）。 */
function text(value: unknown, max: number): string {
  if (
    typeof value !== 'string' ||
    !value.trim() ||
    value.length > max ||
    /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/u.test(value)
  )
    return invalid();
  return value;
}

function integer(value: unknown, max: number): number {
  if (
    typeof value !== 'number' ||
    !Number.isSafeInteger(value) ||
    value < 0 ||
    value > max
  )
    return invalid();
  return value;
}

function choice<T extends string>(value: unknown, values: readonly T[]): T {
  if (typeof value !== 'string' || !values.includes(value as T))
    return invalid();
  return value as T;
}

function source(value: unknown): string {
  const name = text(value, 32);
  if (!/^[a-z0-9][a-z0-9-]*$/.test(name)) return invalid();
  return name;
}

function project(value: unknown): string {
  const path = text(value, 512);
  const parts = path.split('/');
  if (
    path.normalize('NFC') !== path ||
    parts.length !== 2 ||
    parts[0] !== '项目' ||
    !parts[1]
  )
    return invalid();
  return path;
}

const RFC3339_UTC = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?Z$/;

function instant(value: unknown): string {
  const at = text(value, 40);
  if (!RFC3339_UTC.test(at) || !Number.isFinite(Date.parse(at)))
    return invalid();
  return at;
}

function session(value: unknown, expires: number): PresenceSession {
  const row = record(value, REQUIRED, OPTIONAL);
  const parsed: PresenceSession = {
    source: source(row.source),
    vendor: choice(row.vendor, vendors),
    project: project(row.project),
    state: choice(row.state, states),
    ageSecs: integer(row.age_secs, expires),
  };
  if (row.branch !== undefined) parsed.branch = text(row.branch, 120);
  if (row.step !== undefined) parsed.step = integer(row.step, 999);
  if (row.total !== undefined) {
    parsed.total = integer(row.total, 999);
    // 「共几条」只能跟着「第几条」来，且不小于它。
    if (parsed.step === undefined || parsed.total < parsed.step)
      return invalid();
  }
  if (row.last_action !== undefined)
    parsed.lastAction = choice(row.last_action, actions);
  if (row.idle_secs !== undefined)
    parsed.idleSecs = integer(row.idle_secs, 86_400 + expires);
  return parsed;
}

export function parsePresence(value: unknown): Presence {
  const top = record(value, ['since', 'expires_in_secs', 'sessions'], []);
  const expiresInSecs = integer(top.expires_in_secs, 3_600);
  if (
    !Array.isArray(top.sessions) ||
    top.sessions.length > PRESENCE_MAX_SESSIONS
  )
    return invalid();
  return {
    since: instant(top.since),
    expiresInSecs,
    sessions: top.sessions.map((entry) => session(entry, expiresInSecs)),
  };
}
