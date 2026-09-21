import { useCallback, useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { factorRuntime } from '../factorApi';
import type { FactorState } from '../factorModel';
import { validOtp } from '../factorModel';
import { useDialogSurface } from '../useDialogSurface';
import { Icon } from './Icon';

type Load =
  | { kind: 'loading' }
  | { kind: 'ready'; state: FactorState }
  | { kind: 'no-path' }
  | { kind: 'no-session' }
  | { kind: 'failed' };

/**
 * 发布被 403 `reauthentication_required` 挡下时的就地认证窗口。
 *
 * 认证仍由人现场完成、仍走同一条受控会话的 `/api/v1/auth/step-up`；本组件不持有凭据、
 * 不记住认证结果，也不自己决定「已经认证过了」——通过与否由服务端的近期认证窗口裁定。
 * 成功后只回调一次，由调用方原样重试那一个动作。
 */
export function PolicyStepUp({
  action,
  attempt,
  onAuthenticated,
  onClose,
}: {
  action: string;
  attempt: number;
  onAuthenticated: () => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const { dismiss } = useDialogSurface(ref, onClose);
  const field = useRef<HTMLInputElement>(null);
  const focused = useRef(false);
  const lifetime = useRef<AbortController | null>(null);
  const [load, setLoad] = useState<Load>({ kind: 'loading' });
  const [error, setError] = useState('');
  const [submitting, setSubmitting] = useState(false);

  // 每次提交都会使服务端的因子快照作废；读一次状态既是提交的前置，也是失败后的重新核对。
  // 失败后的重读用 quiet：表单与刚给出的失败原因留在原处，不闪回「正在核对」。
  const read = useCallback(async (quiet = false) => {
    const runtime = factorRuntime();
    const controller = lifetime.current;
    if (!controller || controller.signal.aborted) return;
    if (!runtime) {
      setLoad({ kind: 'no-session' });
      return;
    }
    if (!quiet) setLoad({ kind: 'loading' });
    try {
      const state = await runtime.state(controller.signal);
      if (controller.signal.aborted) return;
      setLoad(state === null ? { kind: 'no-path' } : { kind: 'ready', state });
    } catch {
      if (!controller.signal.aborted) setLoad({ kind: 'failed' });
    }
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    void read();
    return () => controller.abort();
  }, [read]);

  useEffect(() => {
    if (load.kind !== 'ready' || focused.current) return;
    focused.current = true;
    field.current?.focus();
  }, [load.kind]);

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const code = String(new FormData(event.currentTarget).get('code') ?? '');
    // 验证码不留在 DOM 里，无论这一次提交是否发得出去。
    event.currentTarget.reset();
    if (submitting || load.kind !== 'ready') return;
    if (!validOtp(code)) {
      setError('请输入六位数字验证码。');
      return;
    }
    setError('');
    setSubmitting(true);
    void (async () => {
      const runtime = factorRuntime();
      const controller = lifetime.current;
      if (!runtime || !controller) {
        setLoad({ kind: 'no-session' });
        setSubmitting(false);
        return;
      }
      try {
        const outcome = await runtime.submit(
          { kind: 'step-up', code },
          controller.signal,
        );
        if (controller.signal.aborted) return;
        if (outcome.kind === 'step-up') {
          onAuthenticated();
          return;
        }
        setError(
          outcome.kind === 'failed'
            ? outcome.message
            : '认证没有开始或已被中断：可能有别的操作尚未结束，或登录状态已变化。请核对后再输入新的验证码。',
        );
        await read(true);
      } catch {
        if (!controller.signal.aborted)
          setError('认证未完成。请核对状态后再输入新的验证码。');
      } finally {
        if (!controller.signal.aborted) setSubmitting(false);
      }
    })();
  }

  const recent =
    load.kind === 'ready' ? load.state.recent_authentication : null;
  return (
    <dialog
      className="session-dialog policy-stepup"
      ref={ref}
      aria-labelledby="policy-stepup-title"
      onCancel={(event) => {
        event.preventDefault();
        if (!submitting) dismiss();
      }}
    >
      <header className="session-dialog-header">
        <div>
          <h2 id="policy-stepup-title">
            <Icon name="lock" />
            二次认证后继续
          </h2>
          <p>
            本次动作：{action}
            。认证通过后只继续这一个动作，并且按你刚才看过的 effective
            提交；期间 effective 变了会被服务端挡下，不会改用新的。
          </p>
          {/* 会话重新核对会按 revision 重挂整棵工作台（T-021 的安全契约），本窗口与
              待重试的动作随之作废。那是失败关闭，但得先说清楚，别让人以为回来还接着发。 */}
          <p>切走或刷新本页会关掉这个窗口，回来后要重新点一次发布。</p>
        </div>
        <button
          className="icon-button"
          onClick={dismiss}
          disabled={submitting}
          data-dialog-dismiss
          aria-label="取消二次认证"
        >
          <Icon name="close" />
        </button>
      </header>
      <div className="session-dialog-body">
        {load.kind === 'loading' ? (
          <p role="status">正在核对二次认证是否可用…</p>
        ) : null}
        {load.kind === 'no-session' ? (
          <p className="factor-warning" role="alert">
            本页没有可用的受控会话，不能就地认证。请重新登录后再发布。
          </p>
        ) : null}
        {load.kind === 'no-path' ? (
          <p className="factor-warning" role="alert">
            本入口未启用二次认证，没有 One-time Code 路径，不能发布。
          </p>
        ) : null}
        {load.kind === 'failed' ? (
          <p className="factor-warning" role="alert">
            暂时无法确认二次认证是否可用。请关闭本窗口、核对登录状态后重试。
          </p>
        ) : null}
        {load.kind === 'ready' ? (
          <form
            className="factor-form"
            aria-label="二次认证"
            onSubmit={submit}
            noValidate
          >
            {attempt > 0 ? (
              <p className="factor-warning">
                上一次认证之后发布仍被要求重新认证：认证有效期可能已过，或会话已变化。请输入一个新的验证码。
              </p>
            ) : null}
            {load.state.pending ? (
              <p className="factor-warning">
                有认证决定尚待处理。状态不会确认先前操作的结果；请勿重复使用刚才的输入。
              </p>
            ) : null}
            {recent ? (
              <p>
                本会话已有二次认证（有效至{' '}
                {new Date(recent.expires_at_ms).toLocaleTimeString()}
                ），但服务端仍要求为这次发布重新认证。
              </p>
            ) : (
              <p>请输入验证器中当前的六位验证码。</p>
            )}
            <label>
              验证码
              <input
                ref={field}
                name="code"
                aria-label="验证码"
                type="text"
                inputMode="numeric"
                autoComplete="one-time-code"
                spellCheck={false}
                disabled={submitting}
              />
            </label>
            {error ? (
              <p className="factor-warning" role="alert">
                {error}
              </p>
            ) : null}
            {submitting ? (
              <p role="status">正在认证…停止等待不代表撤回这次认证。</p>
            ) : null}
            <div className="factor-actions">
              <button
                className="outline-button"
                type="submit"
                disabled={submitting}
              >
                完成认证并继续
              </button>
              <button
                className="text-button"
                type="button"
                data-dialog-dismiss
                disabled={submitting}
                onClick={dismiss}
              >
                取消，不发布
              </button>
            </div>
          </form>
        ) : null}
      </div>
    </dialog>
  );
}
