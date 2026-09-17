export type Authentication = 'local' | 'access' | 'session';
export type SessionInfo = {
  id: string;
  state: 'pending' | 'active' | 'revoked' | 'expired' | 'abandoned';
  created_at_ms: number;
  last_seen_at_ms: number;
  absolute_expires_at_ms: number;
};
export type SessionView = {
  phase:
    'checking' | 'legacy' | 'signed-out' | 'ready' | 'problem' | 'uncertain';
  session: SessionInfo | null;
  revision: number;
  message: string;
  busy: boolean;
  factorNotice?: string;
  ownerScope?: string | null;
};
export type RevokeScope =
  { scope: 'current' | 'all' } | { scope: 'session'; session_id: string };

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) invalid();
  return value as Record<string, unknown>;
}
function exact(value: unknown, fields: string[]) {
  const object = record(value);
  if (
    Object.keys(object).length !== fields.length ||
    fields.some((field) => !Object.hasOwn(object, field))
  )
    invalid();
  return object;
}
function invalid(): never {
  throw new Error('登录状态的响应格式无效，请重新核对。');
}

export function authentication(value: unknown): Authentication {
  const mode = exact(value, ['authentication']).authentication;
  if (mode !== 'local' && mode !== 'access' && mode !== 'session') invalid();
  return mode;
}

export function sessionInfo(value: unknown): SessionInfo {
  const info = exact(value, [
    'id',
    'state',
    'created_at_ms',
    'last_seen_at_ms',
    'absolute_expires_at_ms',
  ]);
  if (
    typeof info.id !== 'string' ||
    !/^(?:[0-9a-f]{32}|v2-[0-9a-f]{16})$/.test(info.id) ||
    typeof info.state !== 'string' ||
    !['pending', 'active', 'revoked', 'expired', 'abandoned'].includes(
      info.state,
    )
  )
    invalid();
  for (const field of [
    'created_at_ms',
    'last_seen_at_ms',
    'absolute_expires_at_ms',
  ]) {
    if (!Number.isSafeInteger(info[field]) || (info[field] as number) < 0)
      invalid();
  }
  return info as SessionInfo;
}

export function sessionPair(value: unknown) {
  const hasScope = Object.hasOwn(record(value), 'owner_scope');
  const pair = exact(value, [
    'session',
    'csrf_token',
    ...(hasScope ? ['owner_scope'] : []),
  ]);
  if (
    hasScope &&
    (typeof pair.owner_scope !== 'string' ||
      !/^[0-9a-f]{64}$/.test(pair.owner_scope))
  )
    invalid();
  const session = sessionInfo(pair.session);
  if (
    session.state !== 'active' ||
    typeof pair.csrf_token !== 'string' ||
    !/^(?:[0-9a-f]{64}|c2\.[0-9a-f]{16}\.[0-9a-f]{64})$/.test(pair.csrf_token)
  )
    invalid();
  return {
    session,
    csrf: pair.csrf_token,
    ownerScope: hasScope ? (pair.owner_scope as string) : null,
  };
}

export function sessionList(value: unknown): SessionInfo[] {
  const items = exact(value, ['sessions']).sessions;
  if (!Array.isArray(items) || items.length > 1024) invalid();
  const sessions = items.map(sessionInfo);
  if (new Set(sessions.map((session) => session.id)).size !== sessions.length)
    invalid();
  return sessions;
}

export function revocation(value: unknown) {
  const count = exact(value, ['revoked']).revoked;
  if (!Number.isSafeInteger(count) || (count as number) < 0) invalid();
}
