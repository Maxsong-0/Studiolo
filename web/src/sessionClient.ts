import { ApiError, cancelled, linkedSignal, requestJson } from './sessionHttp';
import { sessionList } from './sessionModel';
import type { RevokeScope } from './sessionModel';
import { discoverSession } from './sessionBootstrap';
import { factorState, validOtp, validRecoveryCode } from './factorModel';
import type { FactorAction, FactorOutcome } from './factorModel';
import { SessionCore } from './sessionCore';

/** 会话的公开Interface；协议与状态实现共享同一个拥有者，不增加独立授权入口。 */
export class SessionClient extends SessionCore {
  check = async () => {
    if (this.operation) return;
    this.clear('checking', '正在核对登录状态…');
    this.operation = 'check';
    const revision = this.state.revision;
    try {
      const result = await discoverSession(this.lifetime.signal);
      this.current(revision);
      if (result.phase === 'ready') this.accept(result.value);
      else
        this.update({
          phase: result.phase,
          message:
            result.phase === 'signed-out' ? '登录后即可进入工作台。' : '',
        });
    } catch {
      if (revision === this.state.revision)
        this.clear(
          'problem',
          '暂时无法核对登录状态。请检查连接与访问许可后重新核对。',
        );
    } finally {
      if (revision === this.state.revision) this.operation = null;
    }
  };
  login = async () => {
    await this.mutate({ kind: 'login' });
  };
  revoke = async (scope: RevokeScope) =>
    (await this.mutate({ kind: 'revoke', scope })).kind === 'revoked';

  read = async (path: string, signal: AbortSignal): Promise<unknown> => {
    if (!['ready', 'legacy'].includes(this.state.phase)) cancelled();
    const revision = this.state.revision;
    const linked = linkedSignal([signal, this.lifetime.signal]);
    try {
      const value = await requestJson(path, linked.signal);
      this.current(revision);
      return value;
    } catch (error) {
      this.current(revision);
      if (error instanceof ApiError && error.status === 401) {
        this.clear(
          'problem',
          '登录已失效，工作台内容已关闭。请重新核对登录状态。',
        );
        this.announce?.();
      }
      throw error;
    } finally {
      linked.dispose();
    }
  };
  list = async (signal: AbortSignal) =>
    sessionList(await this.read('/api/v1/sessions', signal));
  factorState = async (signal: AbortSignal) => {
    const sequence = ++this.factorSequence;
    const revision = this.state.revision;
    this.factors = undefined;
    const value = factorState(await this.read('/api/v1/auth/state', signal));
    this.current(revision);
    if (sequence !== this.factorSequence || signal.aborted) cancelled();
    this.factors = value;
    return value;
  };
  factor = async (
    action: FactorAction,
    signal?: AbortSignal,
  ): Promise<FactorOutcome> => {
    if (this.operation || signal?.aborted) return { kind: 'stopped' };
    if (
      !this.factors ||
      (action.kind === 'recover' && !this.factors.recovery_candidate_available)
    )
      return { kind: 'failed', message: '请先核对安全操作是否可用。' };
    if (
      (action.kind !== 'codes' && !validOtp(action.code)) ||
      (action.kind === 'recover' && !validRecoveryCode(action.recovery_code))
    )
      return { kind: 'failed', message: '请输入规范的验证码与恢复码。' };
    const result = await this.mutate(action, signal);
    return result.kind === 'login' || result.kind === 'revoked'
      ? { kind: 'stopped' }
      : result;
  };

  resume = () => {
    if (this.operation === 'post') this.interrupted();
    else if (this.state.phase !== 'uncertain' && this.state.phase !== 'problem')
      void this.check();
  };
  changed = () => {
    if (this.operation === 'post') this.interrupted();
    else if (
      this.state.phase !== 'uncertain' &&
      this.state.phase !== 'problem'
    ) {
      this.clear('checking', '登录状态已变化，正在重新核对…');
      void this.check();
    }
  };
  suspend = () => {
    if (this.operation === 'post') this.interrupted();
    else if (['ready', 'legacy', 'checking'].includes(this.state.phase))
      this.clear('checking', '页面已暂停，返回后将重新核对登录状态。');
  };
}
