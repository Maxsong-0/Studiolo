import { ApiError, cancelled } from './sessionHttp';
import { sessionPair } from './sessionModel';
import type { SessionView } from './sessionModel';
import { sendMutation } from './sessionMutation';
import type { MutationSuccess, SessionMutation } from './sessionMutation';
import { factorFailure, factorUncertain } from './factorModel';
import type { FactorOutcome, FactorState } from './factorModel';

const uncertain =
  '操作可能已生效，但结果暂时无法确认。请先核对登录状态，再决定下一步。';

// SessionClient 的私有实现：仅认证状态与事务寿命，绝不保存业务内容或因子输入。
export class SessionCore {
  protected state: SessionView = {
    phase: 'checking',
    session: null,
    revision: 0,
    message: '正在核对登录状态…',
    busy: false,
  };
  protected csrf: string | null = null;
  protected lifetime = new AbortController();
  protected operation: 'check' | 'post' | null = null;
  protected pendingMessage = uncertain;
  protected pendingFactor = false;
  protected factors: FactorState | null | undefined;
  protected factorSequence = 0;
  protected listeners = new Set<() => void>();
  protected announce: (() => void) | null = null;

  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  onChange(announce: (() => void) | null) {
    this.announce = announce;
  }
  protected update(change: Partial<SessionView>) {
    this.state = { ...this.state, ...change };
    this.listeners.forEach((listener) => listener());
  }
  protected clear(phase: SessionView['phase'], message: string) {
    const factorNotice =
      this.operation === 'post' && this.pendingFactor
        ? this.pendingMessage
        : this.state.factorNotice;
    this.lifetime.abort();
    this.lifetime = new AbortController();
    this.csrf = null;
    this.operation = null;
    this.factors = undefined;
    this.factorSequence += 1;
    this.update({
      phase,
      message,
      factorNotice,
      session: null,
      busy: false,
      revision: this.state.revision + 1,
    });
  }
  protected current(revision: number) {
    if (revision !== this.state.revision) cancelled();
  }
  protected accept(value: unknown) {
    const { session, csrf } = sessionPair(value);
    this.csrf = csrf;
    this.update({ phase: 'ready', session, message: '', busy: false });
  }
  stop() {
    this.announce = null;
    this.clear('checking', '正在核对登录状态…');
    this.update({ factorNotice: undefined });
  }
  protected async mutate(
    action: SessionMutation,
    signal?: AbortSignal,
  ): Promise<MutationSuccess | FactorOutcome> {
    const login = action.kind === 'login';
    if (
      this.operation ||
      signal?.aborted ||
      (login
        ? this.state.phase !== 'signed-out'
        : this.state.phase !== 'ready' || !this.csrf)
    )
      return { kind: 'stopped' };
    const isFactor = action.kind !== 'login' && action.kind !== 'revoke';
    const own =
      action.kind === 'revoke' &&
      (action.scope.scope !== 'session' ||
        action.scope.session_id === this.state.session?.id);
    const csrf = this.csrf ?? undefined;
    if (login || own)
      this.clear('checking', login ? '正在登录…' : '正在退出登录…');
    else this.update({ busy: true });
    this.operation = 'post';
    this.pendingFactor = isFactor;
    this.pendingMessage = isFactor ? factorUncertain(action.kind) : uncertain;
    if (isFactor) {
      this.factors = undefined;
      this.factorSequence += 1;
    }
    const revision = this.state.revision;
    const interrupt = () => {
      if (revision === this.state.revision) this.interrupted();
    };
    signal?.addEventListener('abort', interrupt, { once: true });
    try {
      const result = await sendMutation(action, this.lifetime.signal, csrf);
      this.current(revision);
      if (result.kind === 'login') this.accept(result.value);
      else if (result.kind === 'recovered')
        this.clear(
          'signed-out',
          '验证器已恢复，所有旧会话已退出。请重新登录。',
        );
      else if (own) this.clear('signed-out', '已退出所选会话。');
      else this.update({ busy: false });
      if (isFactor) this.update({ factorNotice: undefined });
      if (login || action.kind === 'revoke' || result.kind === 'recovered')
        this.announce?.();
      return result;
    } catch (error) {
      if (revision !== this.state.revision) return { kind: 'stopped' };
      if (
        isFactor &&
        error instanceof ApiError &&
        error.status < 500 &&
        error.status !== 401
      ) {
        this.update({ busy: false });
        return {
          kind: 'failed',
          status: error.status,
          message: factorFailure(error.status),
        };
      }
      if (!(error instanceof ApiError) || error.status >= 500)
        this.interrupted();
      else {
        this.clear(
          'problem',
          error.status === 401
            ? '登录已失效，请重新核对登录状态。'
            : '操作未完成。请核对登录状态；如仍无法继续，请检查访问许可与主人登记。',
        );
        this.announce?.();
      }
      return { kind: 'stopped' };
    } finally {
      signal?.removeEventListener('abort', interrupt);
      if (revision === this.state.revision) this.operation = null;
    }
  }
  protected interrupted() {
    const message = this.pendingMessage;
    const factor = this.pendingFactor;
    this.clear('uncertain', message);
    if (factor) this.update({ factorNotice: message });
    this.announce?.();
  }
}
