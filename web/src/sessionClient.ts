import { ApiError, cancelled, linkedSignal, requestJson } from './sessionHttp';
import { sessionList } from './sessionModel';
import type { RevokeScope } from './sessionModel';
import { discoverSession } from './sessionBootstrap';
import { factorState, validOtp, validRecoveryCode } from './factorModel';
import type { FactorAction, FactorOutcome } from './factorModel';
import { SessionCore } from './sessionCore';
import type { Replacement, Restoration } from './canonicalModel';
import type { FileOperationRequest } from './fileOperationModel';

/**
 * 切回页面时再核对会话的最短间隔：一天（用户 2026-09-22 拍板，见 vault ADR-022）。
 * 服务端对每个请求照旧逐个授权，任何 401 立即清内容；这里只决定「切回来要不要主动再问一次」。
 */
export const REVISIT_INTERVAL_MS = 24 * 60 * 60 * 1000;

/** 会话的公开Interface；协议与状态实现共享同一个拥有者，不增加独立授权入口。 */
export class SessionClient extends SessionCore {
  replace = (request: Replacement, signal: AbortSignal) =>
    this.writeContent(request, signal);
  restore = (request: Restoration, signal: AbortSignal) =>
    this.writeContent(request, signal);
  fileOperation = (request: FileOperationRequest, signal: AbortSignal) =>
    this.writeContent(request, signal, '/api/v1/file-operations');
  /** 策略平面写入（desired／校验／发布／吊销）：同一条会话链与 CSRF，路径由 policyApi 固定。 */
  policyPost = (path: string, body: unknown, signal: AbortSignal) =>
    this.postJson(path, body, signal);
  executionCommand = (body: unknown, signal: AbortSignal) =>
    this.postJson('/api/v1/execution/command', body, signal);
  private writeContent = (
    request: Replacement | Restoration | FileOperationRequest,
    signal: AbortSignal,
    endpoint:
      '/api/v1/mutations' | '/api/v1/file-operations' = '/api/v1/mutations',
  ): Promise<unknown> => this.postJson(endpoint, request, signal);
  private postJson = async (
    endpoint: string,
    request: unknown,
    signal: AbortSignal,
  ): Promise<unknown> => {
    if (this.state.phase !== 'ready' || !this.csrf || !this.state.ownerScope)
      throw new ApiError('需要带主人范围的有效应用会话才能保存。', 403);
    if (this.operation || this.contentPending)
      throw new ApiError('另一个操作尚未结束，请稍后保存。', 429);
    if (signal.aborted) cancelled();
    const revision = this.state.revision;
    const linked = linkedSignal([signal, this.lifetime.signal]);
    this.contentPending = true;
    this.update({ busy: true });
    try {
      const result = await requestJson(
        endpoint,
        linked.signal,
        request,
        this.csrf,
        'json-200',
      );
      this.current(revision);
      return result;
    } catch (error) {
      this.current(revision);
      if (error instanceof ApiError && error.status === 401) {
        this.clear(
          'problem',
          '登录已失效，内容已隐藏。草稿仅保留在本页内存，重新登录后可人工恢复。',
        );
        this.announce?.();
      }
      throw error;
    } finally {
      linked.dispose();
      this.contentPending = false;
      if (revision === this.state.revision) this.update({ busy: false });
    }
  };
  check = async () => {
    if (this.operation) return;
    this.clear('checking', '正在核对登录状态…');
    this.operation = 'check';
    const revision = this.state.revision;
    try {
      const result = await discoverSession(this.lifetime.signal);
      this.current(revision);
      if (result.phase === 'ready') this.accept(result.value);
      else if (result.phase === 'legacy') {
        this.checkedAt = Date.now();
        this.update({ phase: 'legacy', message: '' });
      } else
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

  /**
   * 切回标签页或应用时调用：会话一天内确认过就什么都不做——不清内容、不重挂、不打断
   * 进行中的保存。超过一天、或当前根本没有可用会话，才走一次完整核对。
   */
  revisit = () => {
    if (this.operation) return;
    const settled =
      this.state.phase === 'ready' || this.state.phase === 'legacy';
    if (settled && Date.now() - this.checkedAt < REVISIT_INTERVAL_MS) return;
    this.resume();
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
