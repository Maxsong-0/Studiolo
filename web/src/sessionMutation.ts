import { requestJson } from './sessionHttp';
import { recentAuthentication, recoveryCodes } from './factorModel';
import type { FactorAction, FactorSuccess } from './factorModel';
import { revocation } from './sessionModel';
import type { RevokeScope } from './sessionModel';

export type SessionMutation =
  FactorAction | { kind: 'login' } | { kind: 'revoke'; scope: RevokeScope };
export type MutationSuccess =
  FactorSuccess | { kind: 'login'; value: unknown } | { kind: 'revoked' };

// 协议解码只服务会话拥有者；这里不保留凭据、重试或决定授权。
export async function sendMutation(
  action: SessionMutation,
  signal: AbortSignal,
  csrf?: string,
): Promise<MutationSuccess> {
  switch (action.kind) {
    case 'login':
      return {
        kind: 'login',
        value: await requestJson('/api/v1/sessions', signal, {}),
      };
    case 'revoke':
      revocation(
        await requestJson(
          '/api/v1/sessions/revoke',
          signal,
          action.scope,
          csrf,
        ),
      );
      return { kind: 'revoked' };
    case 'step-up':
      return {
        kind: 'step-up',
        recent: recentAuthentication(
          await requestJson(
            '/api/v1/auth/step-up',
            signal,
            { code: action.code },
            csrf,
            'json-200',
          ),
        ),
      };
    case 'codes':
      return {
        kind: 'codes',
        codes: recoveryCodes(
          await requestJson(
            '/api/v1/auth/recovery-codes',
            signal,
            {},
            csrf,
            'json-200',
          ),
        ),
      };
    case 'recover':
      await requestJson(
        '/api/v1/auth/factor/recover',
        signal,
        { recovery_code: action.recovery_code, code: action.code },
        csrf,
        'empty-204',
      );
      return { kind: 'recovered' };
  }
}
