import { useLayoutEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import type { FactorAction, FactorState } from '../factorModel';
import { validOtp, validRecoveryCode } from '../factorModel';
import { Icon } from './Icon';

type Mode = FactorAction['kind'] | null;
export function FactorForms({
  state,
  busy,
  onSubmit,
  onCancel,
}: {
  state: FactorState;
  busy: boolean;
  onSubmit: (action: FactorAction) => void;
  onCancel: () => void;
}) {
  const [mode, setMode] = useState<Mode>(null);
  const [error, setError] = useState('');
  const form = useRef<HTMLFormElement>(null);
  useLayoutEffect(() => {
    const element = form.current;
    return () => {
      element?.reset();
    };
  }, [mode]);
  function choose(next: Mode) {
    form.current?.reset();
    setError('');
    setMode(next);
  }
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const values = new FormData(event.currentTarget);
    const code = String(values.get('code') ?? '');
    const recovery = String(values.get('recovery') ?? '');
    event.currentTarget.reset();
    if (busy || !mode) return;
    if (
      (mode !== 'codes' && !validOtp(code)) ||
      (mode === 'recover' && !validRecoveryCode(recovery))
    ) {
      setError(
        mode === 'recover'
          ? '请输入完整的恢复码和六位数字验证码。'
          : '请输入六位数字验证码。',
      );
      return;
    }
    setError('');
    onSubmit(
      mode === 'codes'
        ? { kind: 'codes' }
        : mode === 'step-up'
          ? { kind: 'step-up', code }
          : { kind: 'recover', recovery_code: recovery, code },
    );
  }
  const recent = state.recent_authentication;
  return (
    <>
      <p>
        {recent
          ? `已完成二次认证，有效至 ${new Date(recent.expires_at_ms).toLocaleTimeString()}。`
          : '生成恢复码等敏感操作需要先完成二次认证。'}
      </p>
      <div className="factor-actions">
        <button
          className="outline-button"
          disabled={busy}
          onClick={() => choose('step-up')}
        >
          <Icon name="lock" />
          二次认证
        </button>
        <button
          className="outline-button"
          disabled={busy || !recent}
          onClick={() => choose('codes')}
        >
          <Icon name="key" />
          生成恢复码
        </button>
        {state.recovery_candidate_available ? (
          <button
            className="text-button"
            disabled={busy}
            onClick={() => choose('recover')}
          >
            恢复验证器
          </button>
        ) : null}
      </div>
      {mode ? (
        <form ref={form} className="factor-form" onSubmit={submit} noValidate>
          <h4>
            {mode === 'step-up'
              ? '输入验证器中的验证码'
              : mode === 'codes'
                ? '生成一批新的恢复码'
                : '恢复到准备好的新验证器'}
          </h4>
          {mode === 'codes' ? (
            <p>生成后，之前所有恢复码立即失效。新码仅本次显示，请及时保存。</p>
          ) : null}
          {mode === 'recover' ? (
            <>
              <p>
                请使用准备好的新验证器。确认后会替换旧验证器，并退出全部会话。
              </p>
              <label>
                恢复码
                <input
                  name="recovery"
                  aria-label="恢复码"
                  type="password"
                  autoComplete="off"
                  autoCapitalize="off"
                  spellCheck={false}
                  disabled={busy}
                />
              </label>
            </>
          ) : null}
          {mode !== 'codes' ? (
            <label>
              {mode === 'recover' ? '新验证器验证码' : '验证码'}
              <input
                name="code"
                aria-label={mode === 'recover' ? '新验证器验证码' : '验证码'}
                type="text"
                inputMode="numeric"
                autoComplete="one-time-code"
                spellCheck={false}
                disabled={busy}
              />
            </label>
          ) : null}
          {error ? <p role="alert">{error}</p> : null}
          {busy ? <p role="status">正在提交…停止等待不代表撤回操作。</p> : null}
          <div className="factor-actions">
            <button className="outline-button" type="submit" disabled={busy}>
              {mode === 'step-up'
                ? '确认二次认证'
                : mode === 'codes'
                  ? '确认生成新恢复码'
                  : '确认恢复验证器'}
            </button>
            <button
              className="text-button"
              type="button"
              onClick={() => {
                choose(null);
                onCancel();
              }}
            >
              关闭此操作
            </button>
          </div>
        </form>
      ) : null}
    </>
  );
}
